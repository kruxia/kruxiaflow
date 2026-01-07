use crate::activity_result::ActivityResult;
use crate::registry::ActivityImpl;
use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest::{Client, Method};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::time::Duration;

// ============================================================================
// HTTP Executor (low-level HTTP client)
// ============================================================================

/// HTTP activity executor
struct HttpExecutor {
    client: Client,
}

impl HttpExecutor {
    fn new() -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("Failed to build HTTP client");

        Self { client }
    }

    /// Execute an HTTP request activity
    ///
    /// Returns a reqwest::Response with the body stream unconsumed.
    /// Use HttpResponse::from_response() to consume the stream as needed.
    async fn execute(&self, params: HttpRequestParams) -> Result<reqwest::Response> {
        // Build request
        let method = params
            .method
            .parse::<Method>()
            .context("Invalid HTTP method")?;

        let mut request = self.client.request(method, &params.url);

        // Set default User-Agent if not provided (polite for API usage, required by some services like weather.gov)
        let has_user_agent = params
            .headers
            .as_ref()
            .map(|h| h.keys().any(|k| k.eq_ignore_ascii_case("user-agent")))
            .unwrap_or(false);

        if !has_user_agent {
            request = request.header(
                "User-Agent",
                "Kruxia Flow/0.2 (https://github.com/kruxia/kruxiaflow)",
            );
        }

        // Add custom headers
        if let Some(headers) = params.headers {
            for (key, value) in headers {
                request = request.header(key, value);
            }
        }

        // Add query parameters
        if let Some(query) = params.query {
            request = request.query(&query);
        }

        // Add request body as JSON. **TODO:** support other body types
        if let Some(body) = params.body {
            request = request.json(&body);
        }

        // Override timeout if specified
        if let Some(timeout_secs) = params.timeout_seconds {
            request = request.timeout(Duration::from_secs(timeout_secs));
        }

        // Execute request and return with unconsumed body stream
        request.send().await.context("Failed to send HTTP request")
    }
}

impl Default for HttpExecutor {
    fn default() -> Self {
        Self::new()
    }
}

/// HTTP request parameters
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpRequestParams {
    /// HTTP method (GET, POST, PUT, DELETE, PATCH, etc.)
    pub method: String,

    /// Request URL
    pub url: String,

    /// Request headers
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headers: Option<HashMap<String, String>>,

    /// Query parameters
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<HashMap<String, String>>,

    /// Request body (JSON)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<Value>,

    /// Request timeout in seconds (overrides default 30s)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<u64>,

    /// Whether to include response body (default: true)
    /// Set to false for HEAD requests or when only status/headers are needed
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub include_body: Option<bool>,

    /// Optional filename to download response body to
    /// When set, streams response to file instead of loading into memory
    /// File will be created in the current working directory
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download_to_file: Option<String>,
}

/// HTTP response (serializable snapshot)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpResponse {
    /// HTTP status code
    pub status: u16,

    /// Whether request was successful (2xx status)
    pub success: bool,

    /// Response body (only populated if explicitly consumed via text() or json())
    /// In the future, large responses will stream to object storage instead
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<Value>,
}

impl HttpResponse {
    /// Create response from reqwest::Response, consuming body as JSON
    ///
    /// This loads the entire response body into memory and parses as JSON.
    /// Use this only when you need the JSON data in workflow state.
    async fn from_response_json(response: reqwest::Response) -> Result<Self> {
        let status = response.status().as_u16();
        let success = response.status().is_success();

        let body_json = response
            .json::<Value>()
            .await
            .context("Failed to parse response as JSON")?;

        Ok(HttpResponse {
            status,
            success,
            body: Some(body_json),
        })
    }

    /// Create response from reqwest::Response, consuming body as text
    ///
    /// This loads the entire response body into memory as a string.
    /// Use this only when you need the text data in workflow state.
    async fn from_response_text(response: reqwest::Response) -> Result<Self> {
        let status = response.status().as_u16();
        let success = response.status().is_success();

        let body_text = response
            .text()
            .await
            .context("Failed to read response as text")?;

        Ok(HttpResponse {
            status,
            success,
            body: Some(Value::String(body_text)),
        })
    }

    /// Create response metadata only (no body)
    ///
    /// Use this when you only need status/headers, not the body.
    /// Body stream is discarded without loading into memory.
    async fn from_response_metadata(response: reqwest::Response) -> Result<Self> {
        let status = response.status().as_u16();
        let success = response.status().is_success();

        // Drop the response (and its unconsumed body stream)
        drop(response);

        Ok(HttpResponse {
            status,
            success,
            body: None,
        })
    }

    // TODO: Add from_response_stream_to_storage() when object storage is implemented
    // This will stream large responses directly to S3/object storage without loading into memory
}

// ============================================================================
// HTTP Activity (ActivityImpl wrapper for built-in worker)
// ============================================================================

/// HTTP request activity (built-in worker)
///
/// Executes HTTP requests with configurable method, headers, body, etc.
pub struct HttpRequestActivity {
    executor: HttpExecutor,
}

impl HttpRequestActivity {
    pub fn new() -> Self {
        Self {
            executor: HttpExecutor::new(),
        }
    }
}

impl Default for HttpRequestActivity {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ActivityImpl for HttpRequestActivity {
    async fn execute(&self, parameters: Value) -> Result<ActivityResult> {
        tracing::debug!(
            "Executing http_request activity with parameters: {:?}",
            parameters
        );

        // Extract temp directory if provided (internal parameter injected by worker)
        let temp_dir = parameters
            .get("_kruxiaflow_temp_dir")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        // Parse parameters from JSON
        let params: HttpRequestParams = serde_json::from_value(parameters)
            .context("Failed to parse HTTP request parameters")?;

        tracing::info!(
            method = %params.method,
            url = %params.url,
            "HTTP request starting"
        );

        // Check if we should download to file
        let download_filename = params.download_to_file.clone();
        if let Some(filename) = download_filename {
            tracing::debug!("Downloading response to file: {}", filename);

            // Execute HTTP request
            let response = self.executor.execute(params).await?;
            let status = response.status();
            let success = status.is_success();

            tracing::info!(
                status = %status,
                success = success,
                "HTTP request completed (downloading to file)"
            );

            // Stream response body to file
            use tokio::fs::File;
            use tokio::io::AsyncWriteExt;

            // Determine output directory (use temp_dir if provided, otherwise current dir)
            let output_path = if let Some(ref dir) = temp_dir {
                std::path::PathBuf::from(dir).join(&filename)
            } else {
                std::path::PathBuf::from(&filename)
            };

            let mut file = File::create(&output_path)
                .await
                .context("Failed to create output file")?;

            // Read response bytes and write to file
            let bytes = response
                .bytes()
                .await
                .context("Failed to read response bytes")?;

            let total_bytes = bytes.len() as u64;
            file.write_all(&bytes)
                .await
                .context("Failed to write bytes to file")?;

            file.sync_all().await.context("Failed to sync file")?;

            tracing::debug!(
                "Downloaded {} bytes to file: {}",
                total_bytes,
                output_path.display()
            );

            // Return metadata only (file will be picked up by FileExecutor)
            let http_response = HttpResponse {
                status: status.as_u16(),
                success,
                body: None,
            };

            return Ok(ActivityResult::value("response", json!(http_response)));
        }

        // Normal flow (no file download)
        // Check if we should include body (default: true, except for HEAD requests)
        let include_body =
            params.include_body.unwrap_or(true) && !params.method.eq_ignore_ascii_case("HEAD");

        // Execute HTTP request
        let response = self.executor.execute(params).await?;

        // Convert response based on whether body should be included
        let http_response = if !include_body {
            // Metadata only (status/headers) - don't load body into memory
            HttpResponse::from_response_metadata(response).await?
        } else if let Some(content_type) = response.headers().get("content-type") {
            // Body requested - parse based on content type
            let content_type_str = content_type.to_str().unwrap_or("");
            if content_type_str.contains("application/json") {
                HttpResponse::from_response_json(response).await?
            } else {
                HttpResponse::from_response_text(response).await?
            }
        } else {
            // No content-type, try JSON first, fall back to metadata only
            let status = response.status();
            match response.json::<Value>().await {
                Ok(json_body) => HttpResponse {
                    status: status.as_u16(),
                    success: status.is_success(),
                    body: Some(json_body),
                },
                Err(_) => {
                    // If JSON parsing fails, we can't get the body again (stream consumed)
                    // Return metadata only
                    HttpResponse {
                        status: status.as_u16(),
                        success: status.is_success(),
                        body: None,
                    }
                }
            }
        };

        tracing::info!(
            status = http_response.status,
            success = http_response.success,
            "HTTP request completed"
        );

        // Return as ActivityResult
        Ok(ActivityResult::value("response", json!(http_response)))
    }

    fn name(&self) -> &str {
        "http_request"
    }

    fn worker(&self) -> &str {
        "builtin"
    }
}

// ============================================================================
// Tests
// ============================================================================

// Note: HTTP activity tests are in tests/http_activity_integration_test.rs
// These integration tests use the local Kruxia Flow API server instead of
// external services to ensure reliable, isolated testing.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_http_executor_creation() {
        let executor = HttpExecutor::new();
        // Verify executor can be created
        assert!(std::mem::size_of_val(&executor) > 0);
    }

    #[test]
    fn test_http_request_params_serialization() {
        let params = HttpRequestParams {
            method: "GET".to_string(),
            url: "http://example.com".to_string(),
            headers: Some(HashMap::from([(
                "Content-Type".to_string(),
                "application/json".to_string(),
            )])),
            query: Some(HashMap::from([("key".to_string(), "value".to_string())])),
            body: Some(json!({"test": "data"})),
            timeout_seconds: Some(30),
            include_body: Some(true),
            download_to_file: None,
        };

        // Verify params can be serialized and deserialized
        let json = serde_json::to_string(&params).unwrap();
        let deserialized: HttpRequestParams = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.method, "GET");
        assert_eq!(deserialized.url, "http://example.com");
        assert_eq!(deserialized.timeout_seconds, Some(30));
        assert_eq!(deserialized.include_body, Some(true));
    }

    #[test]
    fn test_http_response_creation() {
        let response = HttpResponse {
            status: 200,
            success: true,
            body: Some(json!({"message": "success"})),
        };

        assert_eq!(response.status, 200);
        assert!(response.success);
        assert!(response.body.is_some());
    }

    #[test]
    fn test_http_request_activity_name_and_worker() {
        let activity = HttpRequestActivity::new();

        assert_eq!(activity.name(), "http_request");
        assert_eq!(activity.worker(), "builtin");
    }
}
