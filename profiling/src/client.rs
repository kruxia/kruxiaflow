use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use uuid::Uuid;

#[derive(Clone)]
pub struct StreamFlowClient {
    client: Client,
    base_url: String,
    access_token: Arc<RwLock<Option<String>>>,
    client_id: String,
    client_secret: String,
}

#[derive(Debug, Serialize)]
pub struct CreateWorkflowRequest {
    pub definition_name: String,
    pub input: Value,
}

#[derive(Debug, Deserialize)]
pub struct WorkflowResponse {
    pub workflow_id: Uuid,
    pub definition_name: String,
    pub definition_version: String,
    pub status: String,
    pub created_at: String,
}

#[derive(Debug, Deserialize)]
pub struct WorkflowStatusResponse {
    pub id: Uuid,
    pub status: String,
    pub state_data: Value,
    pub activities: Value,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
}

impl StreamFlowClient {
    pub fn new(base_url: String, client_id: String, client_secret: String) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("Failed to build HTTP client");

        Self {
            client,
            base_url,
            access_token: Arc::new(RwLock::new(None)),
            client_id,
            client_secret,
        }
    }

    async fn get_access_token(&self) -> Result<String, reqwest::Error> {
        // Check if we already have a token
        {
            let token = self.access_token.read().await;
            if let Some(t) = token.as_ref() {
                return Ok(t.clone());
            }
        }

        // Get new token
        let url = format!("{}/api/v1/oauth/token", self.base_url);
        let response = self
            .client
            .post(&url)
            .form(&[
                ("grant_type", "client_credentials"),
                ("client_id", &self.client_id),
                ("client_secret", &self.client_secret),
            ])
            .send()
            .await?
            .error_for_status()?;

        let token_data: TokenResponse = response.json().await?;

        // Store token
        {
            let mut token = self.access_token.write().await;
            *token = Some(token_data.access_token.clone());
        }

        Ok(token_data.access_token)
    }

    /// Create a new workflow via HTTP API
    pub async fn create_workflow(
        &self,
        definition_name: &str,
        input: Value,
    ) -> Result<WorkflowResponse, reqwest::Error> {
        let token = self.get_access_token().await?;
        let url = format!("{}/api/v1/workflows", self.base_url);
        let request = CreateWorkflowRequest {
            definition_name: definition_name.to_string(),
            input,
        };

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", token))
            .json(&request)
            .send()
            .await?
            .error_for_status()?;

        response.json::<WorkflowResponse>().await
    }

    /// Get workflow status via HTTP API
    pub async fn get_workflow_status(
        &self,
        workflow_id: Uuid,
    ) -> Result<WorkflowStatusResponse, reqwest::Error> {
        let token = self.get_access_token().await?;
        let url = format!("{}/api/v1/workflows/{}", self.base_url, workflow_id);

        self.client
            .get(&url)
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await?
            .error_for_status()?
            .json::<WorkflowStatusResponse>()
            .await
    }

    /// Poll for workflow completion
    pub async fn wait_for_completion(
        &self,
        workflow_id: Uuid,
        timeout: Duration,
    ) -> Result<WorkflowStatusResponse, Box<dyn std::error::Error>> {
        let start = std::time::Instant::now();
        let poll_interval = Duration::from_millis(50);

        loop {
            if start.elapsed() > timeout {
                return Err("Workflow completion timeout".into());
            }

            let status = self.get_workflow_status(workflow_id).await?;

            if status.status == "completed" || status.status == "failed" {
                return Ok(status);
            }

            tokio::time::sleep(poll_interval).await;
        }
    }
}
