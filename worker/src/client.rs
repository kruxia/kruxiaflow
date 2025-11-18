use anyhow::{Context, Result};
use reqwest::{Client as HttpClient, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

/// API client for worker operations
#[derive(Clone)]
pub struct WorkerApiClient {
    http: HttpClient,
    api_url: String,
    client_id: String,
    client_secret: String,
    token: Arc<RwLock<Option<String>>>,
}

impl WorkerApiClient {
    pub fn new(api_url: String, client_id: String, client_secret: String) -> Self {
        Self {
            http: HttpClient::new(),
            api_url,
            client_id,
            client_secret,
            token: Arc::new(RwLock::new(None)),
        }
    }

    /// Obtain access token via OAuth client credentials flow
    async fn obtain_token(&self) -> Result<String> {
        #[derive(Serialize)]
        struct TokenRequest {
            grant_type: String,
            client_id: String,
            client_secret: String,
        }

        #[derive(Deserialize)]
        struct TokenResponse {
            access_token: String,
        }

        let response = self
            .http
            .post(format!("{}/api/v1/oauth/token", self.api_url))
            .json(&TokenRequest {
                grant_type: "client_credentials".to_string(),
                client_id: self.client_id.clone(),
                client_secret: self.client_secret.clone(),
            })
            .send()
            .await
            .context("Failed to request access token")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Token request failed: {} - {}", status, body);
        }

        let token_response: TokenResponse = response
            .json()
            .await
            .context("Failed to parse token response")?;

        Ok(token_response.access_token)
    }

    /// Get current token or obtain new one
    async fn get_token(&self) -> Result<String> {
        let token_lock = self.token.read().await;
        if let Some(token) = token_lock.as_ref() {
            return Ok(token.clone());
        }
        drop(token_lock);

        // Token not available, obtain new one
        let new_token = self.obtain_token().await?;
        let mut token_lock = self.token.write().await;
        *token_lock = Some(new_token.clone());
        Ok(new_token)
    }

    /// Poll for activities
    pub async fn poll_activities(
        &self,
        worker_id: &str,
        activity_types: Vec<String>,
        max_activities: usize,
    ) -> Result<PollActivitiesResponse> {
        let token = self.get_token().await?;

        #[derive(Serialize)]
        struct PollRequest {
            activity_types: Vec<String>,
            worker_id: String,
            max_activities: usize,
        }

        let response = self
            .http
            .post(format!("{}/api/v1/workers/poll", self.api_url))
            .bearer_auth(&token)
            .json(&PollRequest {
                activity_types: activity_types.clone(),
                worker_id: worker_id.to_string(),
                max_activities,
            })
            .send()
            .await
            .context("Failed to poll activities")?;

        // Handle 401 by refreshing token
        if response.status() == StatusCode::UNAUTHORIZED {
            tracing::warn!("Token expired, obtaining new token");
            let mut token_lock = self.token.write().await;
            *token_lock = None;
            drop(token_lock);
            // Retry once with new token
            return Box::pin(self.poll_activities(worker_id, activity_types, max_activities)).await;
        }

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Poll failed: {} - {}", status, body);
        }

        let poll_response: PollActivitiesResponse = response
            .json()
            .await
            .context("Failed to parse poll response")?;

        Ok(poll_response)
    }

    /// Send heartbeat for activity
    pub async fn heartbeat(&self, activity_id: Uuid, worker_id: &str) -> Result<()> {
        let token = self.get_token().await?;

        #[derive(Serialize)]
        struct HeartbeatRequest {
            worker_id: String,
        }

        let response = self
            .http
            .post(format!(
                "{}/api/v1/activities/{}/heartbeat",
                self.api_url, activity_id
            ))
            .bearer_auth(&token)
            .json(&HeartbeatRequest {
                worker_id: worker_id.to_string(),
            })
            .send()
            .await
            .context("Failed to send heartbeat")?;

        if response.status() == StatusCode::UNAUTHORIZED {
            let mut token_lock = self.token.write().await;
            *token_lock = None;
            drop(token_lock);
            return Box::pin(self.heartbeat(activity_id, worker_id)).await;
        }

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Heartbeat failed: {} - {}", status, body);
        }

        Ok(())
    }

    /// Complete activity successfully
    pub async fn complete_activity(
        &self,
        activity_id: Uuid,
        worker_id: &str,
        output: Value,
        cost_usd: Option<f64>,
    ) -> Result<()> {
        let token = self.get_token().await?;

        #[derive(Serialize)]
        struct CompleteRequest {
            worker_id: String,
            output: Value,
            #[serde(skip_serializing_if = "Option::is_none")]
            cost_usd: Option<f64>,
        }

        let response = self
            .http
            .post(format!(
                "{}/api/v1/activities/{}/complete",
                self.api_url, activity_id
            ))
            .bearer_auth(&token)
            .json(&CompleteRequest {
                worker_id: worker_id.to_string(),
                output: output.clone(),
                cost_usd,
            })
            .send()
            .await
            .context("Failed to complete activity")?;

        if response.status() == StatusCode::UNAUTHORIZED {
            let mut token_lock = self.token.write().await;
            *token_lock = None;
            drop(token_lock);
            return Box::pin(self.complete_activity(activity_id, worker_id, output, cost_usd))
                .await;
        }

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Complete failed: {} - {}", status, body);
        }

        Ok(())
    }

    /// Fail activity
    pub async fn fail_activity(
        &self,
        activity_id: Uuid,
        worker_id: &str,
        error_code: String,
        error_message: String,
        retryable: bool,
    ) -> Result<()> {
        let token = self.get_token().await?;

        #[derive(Serialize)]
        struct FailRequest {
            worker_id: String,
            error: ActivityError,
        }

        #[derive(Serialize)]
        struct ActivityError {
            code: String,
            message: String,
            retryable: bool,
        }

        let response = self
            .http
            .post(format!(
                "{}/api/v1/activities/{}/fail",
                self.api_url, activity_id
            ))
            .bearer_auth(&token)
            .json(&FailRequest {
                worker_id: worker_id.to_string(),
                error: ActivityError {
                    code: error_code.clone(),
                    message: error_message.clone(),
                    retryable,
                },
            })
            .send()
            .await
            .context("Failed to fail activity")?;

        if response.status() == StatusCode::UNAUTHORIZED {
            let mut token_lock = self.token.write().await;
            *token_lock = None;
            drop(token_lock);
            return Box::pin(self.fail_activity(
                activity_id,
                worker_id,
                error_code,
                error_message,
                retryable,
            ))
            .await;
        }

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Failed to fail activity: {} - {}", status, body);
        }

        Ok(())
    }
}

#[derive(Debug, Deserialize)]
pub struct PollActivitiesResponse {
    pub activities: Vec<PendingActivity>,
    pub count: usize,
}

#[derive(Debug, Deserialize)]
pub struct PendingActivity {
    pub activity_id: Uuid,
    pub workflow_id: Uuid,
    pub activity_key: String,
    pub worker: String,
    pub activity_name: String,
    pub parameters: Value,
    pub settings: Option<Value>,
    pub timeout_seconds: Option<i64>,
    pub output_definitions: Option<Value>,
}

#[cfg(test)]
#[path = "client_test.rs"]
mod client_test;
