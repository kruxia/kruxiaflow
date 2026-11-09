//! HTTP client for the worker activity APIs.

use crate::error::{ActivityError, ClientError};
use crate::types::{PendingActivity, PollActivitiesResponse, PollOutcome, UsageEntry};
use reqwest::{Client as HttpClient, StatusCode};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use uuid::Uuid;

#[derive(Clone)]
struct Credentials {
    client_id: String,
    client_secret: String,
}

/// Client for the worker HTTP API: poll, heartbeat, complete, fail.
///
/// Authenticates via the OAuth2 client-credentials flow with automatic token
/// refresh on 401. When constructed without credentials the client sends
/// unauthenticated requests, which works against a server running in dev mode
/// (`kruxiaflow serve --insecure-dev`).
#[derive(Clone)]
pub struct WorkerApiClient {
    http: HttpClient,
    /// For the long-lived wake stream: no total request timeout
    stream_http: HttpClient,
    api_url: String,
    credentials: Option<Credentials>,
    token: Arc<RwLock<Option<String>>>,
}

/// A wake stream that hears nothing for this long (three missed 15-second
/// keepalives) is treated as dead.
const WAKE_STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(45);

/// The answer to [`WorkerApiClient::open_wake_stream`].
pub enum WakeConnect {
    /// The stream is open
    Stream(WakeStream),
    /// The server does not offer a wake stream (HTTP 404, 405, 501 or 503)
    Unavailable { status: u16 },
}

/// An open wake stream (`text/event-stream`).
///
/// Every event means "poll now". Events carry no data, so the stream is read
/// only for its framing: any event (a block of field lines ended by a blank
/// line) is a wake, and comment lines (keepalives) only show the connection
/// is alive.
pub struct WakeStream {
    response: reqwest::Response,
    /// Bytes received and not yet consumed as complete lines
    buffer: Vec<u8>,
    /// A field line has been seen since the last blank line
    event_pending: bool,
}

impl WakeStream {
    fn new(response: reqwest::Response) -> Self {
        Self {
            response,
            buffer: Vec::new(),
            event_pending: false,
        }
    }

    /// Wait for the next wake. Errors when the stream ends, fails, or is
    /// silent for longer than the idle timeout; the caller reconnects.
    pub async fn next_wake(&mut self) -> Result<(), ClientError> {
        loop {
            // Consume complete lines already received
            while let Some(newline) = self.buffer.iter().position(|&b| b == b'\n') {
                let line: Vec<u8> = self.buffer.drain(..=newline).collect();
                let line = line.strip_suffix(b"\n").unwrap_or(&line);
                let line = line.strip_suffix(b"\r").unwrap_or(line);

                if line.is_empty() {
                    // Blank line: dispatch the event, if there was one
                    if std::mem::take(&mut self.event_pending) {
                        return Ok(());
                    }
                } else if !line.starts_with(b":") {
                    self.event_pending = true;
                }
            }

            // A server that never sends a newline must not grow the buffer
            if self.buffer.len() > 64 * 1024 {
                return Err(ClientError::WakeStream("line too long".to_string()));
            }

            match tokio::time::timeout(WAKE_STREAM_IDLE_TIMEOUT, self.response.chunk()).await {
                Ok(Ok(Some(chunk))) => self.buffer.extend_from_slice(&chunk),
                Ok(Ok(None)) => {
                    return Err(ClientError::WakeStream("ended by server".to_string()));
                }
                Ok(Err(e)) => return Err(ClientError::WakeStream(e.to_string())),
                Err(_) => {
                    return Err(ClientError::WakeStream(format!(
                        "silent for {}s",
                        WAKE_STREAM_IDLE_TIMEOUT.as_secs()
                    )));
                }
            }
        }
    }
}

/// Acknowledgment of a completion or failure report, with any non-fatal
/// warnings about reported usage (e.g., unknown provider/model recorded at
/// cost 0).
#[derive(Debug, Deserialize, Default)]
pub struct ReportAck {
    /// Whether the activity will be retried (failure reports only)
    #[serde(default)]
    pub will_retry: bool,
    /// Non-fatal warnings about reported usage
    #[serde(default)]
    pub warnings: Vec<String>,
}

impl WorkerApiClient {
    /// Create an unauthenticated client (for servers running in dev mode).
    pub fn new(api_url: impl Into<String>) -> Self {
        let api_url: String = api_url.into();
        Self {
            http: HttpClient::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .expect("HTTP client construction cannot fail with static config"),
            stream_http: HttpClient::builder()
                .connect_timeout(Duration::from_secs(10))
                .build()
                .expect("HTTP client construction cannot fail with static config"),
            api_url: api_url.trim_end_matches('/').to_string(),
            credentials: None,
            token: Arc::new(RwLock::new(None)),
        }
    }

    /// Create a client that authenticates with OAuth client credentials.
    pub fn with_credentials(
        api_url: impl Into<String>,
        client_id: impl Into<String>,
        client_secret: impl Into<String>,
    ) -> Self {
        let mut client = Self::new(api_url);
        client.credentials = Some(Credentials {
            client_id: client_id.into(),
            client_secret: client_secret.into(),
        });
        client
    }

    /// The API base URL this client talks to.
    pub fn api_url(&self) -> &str {
        &self.api_url
    }

    /// Get the current bearer token, obtaining one if credentials are
    /// configured. Returns `Ok(None)` when running without credentials.
    pub async fn get_token(&self) -> Result<Option<String>, ClientError> {
        let Some(credentials) = &self.credentials else {
            return Ok(None);
        };

        if let Some(token) = self.token.read().await.as_ref() {
            return Ok(Some(token.clone()));
        }

        let new_token = self.obtain_token(credentials).await?;
        *self.token.write().await = Some(new_token.clone());
        Ok(Some(new_token))
    }

    async fn obtain_token(&self, credentials: &Credentials) -> Result<String, ClientError> {
        #[derive(Serialize)]
        struct TokenRequest<'a> {
            grant_type: &'a str,
            client_id: &'a str,
            client_secret: &'a str,
        }

        #[derive(Deserialize)]
        struct TokenResponse {
            access_token: String,
        }

        let response = self
            .http
            .post(format!("{}/api/v1/oauth/token", self.api_url))
            .json(&TokenRequest {
                grant_type: "client_credentials",
                client_id: &credentials.client_id,
                client_secret: &credentials.client_secret,
            })
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(ClientError::Auth {
                status: response.status().as_u16(),
                body: response.text().await.unwrap_or_default(),
            });
        }

        let token: TokenResponse = response.json().await?;
        Ok(token.access_token)
    }

    /// POST a JSON body with bearer auth, refreshing the token and retrying
    /// once on 401.
    async fn post_json<B: Serialize>(
        &self,
        url: &str,
        body: &B,
    ) -> Result<reqwest::Response, ClientError> {
        for attempt in 0..2 {
            let mut request = self.http.post(url).json(body);
            if let Some(token) = self.get_token().await? {
                request = request.bearer_auth(token);
            }

            let response = request.send().await?;

            if response.status() == StatusCode::UNAUTHORIZED {
                if self.credentials.is_none() {
                    return Err(ClientError::AuthRequired);
                }
                if attempt == 0 {
                    tracing::warn!(url, "Token rejected (401), refreshing");
                    *self.token.write().await = None;
                    continue;
                }
            }

            return Ok(response);
        }
        unreachable!("loop always returns by the second attempt")
    }

    async fn check(response: reqwest::Response) -> Result<reqwest::Response, ClientError> {
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }
        let body = response.text().await.unwrap_or_default();
        if status == StatusCode::CONFLICT {
            Err(ClientError::Conflict { body })
        } else {
            Err(ClientError::Api {
                status: status.as_u16(),
                body,
            })
        }
    }

    /// Poll for activities of the given worker type.
    ///
    /// `POST /api/v1/workers/poll`
    pub async fn poll_activities(
        &self,
        worker: &str,
        worker_id: &str,
        max_activities: usize,
    ) -> Result<Vec<PendingActivity>, ClientError> {
        Ok(self
            .poll(worker, worker_id, max_activities)
            .await?
            .activities)
    }

    /// Poll for activities of the given worker type, and learn when to poll
    /// again if nothing was claimable.
    ///
    /// `POST /api/v1/workers/poll`
    pub async fn poll(
        &self,
        worker: &str,
        worker_id: &str,
        max_activities: usize,
    ) -> Result<PollOutcome, ClientError> {
        #[derive(Serialize)]
        struct PollRequest<'a> {
            worker: &'a str,
            worker_id: &'a str,
            max_activities: usize,
        }

        let response = self
            .post_json(
                &format!("{}/api/v1/workers/poll", self.api_url),
                &PollRequest {
                    worker,
                    worker_id,
                    max_activities,
                },
            )
            .await?;

        let response: PollActivitiesResponse = Self::check(response).await?.json().await?;
        Ok(PollOutcome {
            activities: response.activities,
            next_due: response.next_due_ms.map(Duration::from_millis),
        })
    }

    /// Open the wake stream for a worker type.
    ///
    /// `GET /api/v1/workers/wake?worker=<type>` (server-sent events)
    ///
    /// The stream tells the worker when to poll; it never carries work.
    /// Returns [`WakeConnect::Unavailable`] when the server has no such
    /// stream (older than 0.10, or running without a wake signal): the
    /// worker then polls on its timer as before.
    pub async fn open_wake_stream(&self, worker: &str) -> Result<WakeConnect, ClientError> {
        let url = format!("{}/api/v1/workers/wake", self.api_url);

        for attempt in 0..2 {
            let mut request = self
                .stream_http
                .get(&url)
                .query(&[("worker", worker)])
                .header(reqwest::header::ACCEPT, "text/event-stream");
            if let Some(token) = self.get_token().await? {
                request = request.bearer_auth(token);
            }

            let response = request.send().await?;
            let status = response.status();

            if status == StatusCode::UNAUTHORIZED {
                if self.credentials.is_none() {
                    return Err(ClientError::AuthRequired);
                }
                if attempt == 0 {
                    tracing::warn!(url, "Token rejected (401), refreshing");
                    *self.token.write().await = None;
                    continue;
                }
            }

            return match status {
                s if s.is_success() => Ok(WakeConnect::Stream(WakeStream::new(response))),
                StatusCode::NOT_FOUND
                | StatusCode::METHOD_NOT_ALLOWED
                | StatusCode::NOT_IMPLEMENTED
                | StatusCode::SERVICE_UNAVAILABLE => Ok(WakeConnect::Unavailable {
                    status: status.as_u16(),
                }),
                _ => Err(ClientError::Api {
                    status: status.as_u16(),
                    body: response.text().await.unwrap_or_default(),
                }),
            };
        }
        unreachable!("loop always returns by the second attempt")
    }

    /// Send a heartbeat for an in-flight activity.
    ///
    /// `POST /api/v1/activities/{id}/heartbeat`
    ///
    /// Returns [`ClientError::Conflict`] when the activity was completed or
    /// reassigned elsewhere — the caller should cancel local execution.
    pub async fn heartbeat(&self, activity_id: Uuid, worker_id: &str) -> Result<(), ClientError> {
        #[derive(Serialize)]
        struct HeartbeatRequest<'a> {
            worker_id: &'a str,
        }

        let response = self
            .post_json(
                &format!(
                    "{}/api/v1/activities/{}/heartbeat",
                    self.api_url, activity_id
                ),
                &HeartbeatRequest { worker_id },
            )
            .await?;

        Self::check(response).await?;
        Ok(())
    }

    /// Report successful completion.
    ///
    /// `POST /api/v1/activities/{id}/complete`
    pub async fn complete_activity(
        &self,
        activity_id: Uuid,
        worker_id: &str,
        output: Value,
        cost_usd: Option<Decimal>,
        usage: &[UsageEntry],
    ) -> Result<ReportAck, ClientError> {
        #[derive(Serialize)]
        struct CompleteRequest<'a> {
            worker_id: &'a str,
            output: Value,
            #[serde(skip_serializing_if = "Option::is_none")]
            cost_usd: Option<Decimal>,
            #[serde(skip_serializing_if = "Option::is_none")]
            usage: Option<&'a [UsageEntry]>,
        }

        let response = self
            .post_json(
                &format!(
                    "{}/api/v1/activities/{}/complete",
                    self.api_url, activity_id
                ),
                &CompleteRequest {
                    worker_id,
                    output,
                    cost_usd,
                    usage: (!usage.is_empty()).then_some(usage),
                },
            )
            .await?;

        Ok(Self::check(response)
            .await?
            .json()
            .await
            .unwrap_or_default())
    }

    /// Report failure.
    ///
    /// `POST /api/v1/activities/{id}/fail`
    pub async fn fail_activity(
        &self,
        activity_id: Uuid,
        worker_id: &str,
        error: &ActivityError,
    ) -> Result<ReportAck, ClientError> {
        #[derive(Serialize)]
        struct ErrorBody<'a> {
            code: &'a str,
            message: &'a str,
            retryable: bool,
        }

        #[derive(Serialize)]
        struct FailRequest<'a> {
            worker_id: &'a str,
            error: ErrorBody<'a>,
            #[serde(skip_serializing_if = "Option::is_none")]
            cost_usd: Option<Decimal>,
            #[serde(skip_serializing_if = "Option::is_none")]
            usage: Option<&'a [UsageEntry]>,
        }

        let response = self
            .post_json(
                &format!("{}/api/v1/activities/{}/fail", self.api_url, activity_id),
                &FailRequest {
                    worker_id,
                    error: ErrorBody {
                        code: &error.code,
                        message: &error.message,
                        retryable: error.retryable,
                    },
                    cost_usd: error.cost_usd,
                    usage: (!error.usage.is_empty()).then_some(&error.usage),
                },
            )
            .await?;

        Ok(Self::check(response)
            .await?
            .json()
            .await
            .unwrap_or_default())
    }
}
