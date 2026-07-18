//! HTTP client for the worker activity APIs.

use crate::error::{ActivityError, ClientError};
use crate::types::{PendingActivity, PollActivitiesResponse, UsageEntry};
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
    api_url: String,
    credentials: Option<Credentials>,
    token: Arc<RwLock<Option<String>>>,
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
        Ok(response.activities)
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
                &format!("{}/api/v1/activities/{}/heartbeat", self.api_url, activity_id),
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
                &format!("{}/api/v1/activities/{}/complete", self.api_url, activity_id),
                &CompleteRequest {
                    worker_id,
                    output,
                    cost_usd,
                    usage: (!usage.is_empty()).then_some(usage),
                },
            )
            .await?;

        Ok(Self::check(response).await?.json().await.unwrap_or_default())
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

        Ok(Self::check(response).await?.json().await.unwrap_or_default())
    }
}
