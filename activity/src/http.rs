use anyhow::{Context, Result};
use reqwest::{Client, Method};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::time::Duration;

/// HTTP activity executor
pub struct HttpActivity {
    client: Client,
}

impl HttpActivity {
    pub fn new() -> Self {
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
    pub async fn execute(&self, params: HttpRequestParams) -> Result<reqwest::Response> {
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
                "StreamFlow/0.2 (https://github.com/kruxia/streamflow)",
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

impl Default for HttpActivity {
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
    pub async fn from_response_json(response: reqwest::Response) -> Result<Self> {
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
    pub async fn from_response_text(response: reqwest::Response) -> Result<Self> {
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
    pub async fn from_response_metadata(response: reqwest::Response) -> Result<Self> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_http_get_request() {
        let activity = HttpActivity::new();

        let params = HttpRequestParams {
            method: "GET".to_string(),
            url: "https://httpbin.org/get".to_string(),
            headers: None,
            query: Some(HashMap::from([("test".to_string(), "value".to_string())])),
            body: None,
            timeout_seconds: None,
        };

        let response = activity.execute(params).await.unwrap();
        let http_response = HttpResponse::from_response_json(response).await.unwrap();

        assert_eq!(http_response.status, 200);
        assert!(http_response.success);
        assert!(http_response.body.is_some());
    }

    #[tokio::test]
    async fn test_http_post_request() {
        let activity = HttpActivity::new();

        let params = HttpRequestParams {
            method: "POST".to_string(),
            url: "https://httpbin.org/post".to_string(),
            headers: Some(HashMap::from([(
                "Content-Type".to_string(),
                "application/json".to_string(),
            )])),
            query: None,
            body: Some(serde_json::json!({
                "test": "data",
                "number": 42
            })),
            timeout_seconds: None,
        };

        let response = activity.execute(params).await.unwrap();
        let http_response = HttpResponse::from_response_json(response).await.unwrap();

        assert_eq!(http_response.status, 200);
        assert!(http_response.success);
        assert!(http_response.body.is_some());
    }

    #[tokio::test]
    async fn test_http_request_with_headers() {
        let activity = HttpActivity::new();

        let params = HttpRequestParams {
            method: "GET".to_string(),
            url: "https://httpbin.org/headers".to_string(),
            headers: Some(HashMap::from([
                ("User-Agent".to_string(), "StreamFlow/0.2".to_string()),
                ("Authorization".to_string(), "Bearer test_token".to_string()),
            ])),
            query: None,
            body: None,
            timeout_seconds: None,
        };

        let response = activity.execute(params).await.unwrap();
        let http_response = HttpResponse::from_response_json(response).await.unwrap();

        assert_eq!(http_response.status, 200);
        assert!(http_response.success);

        // Verify headers were sent
        let body = http_response.body.as_ref().unwrap();
        let headers = body["headers"].as_object().unwrap();
        assert!(headers.contains_key("Authorization"));
        assert!(headers.contains_key("User-Agent"));
    }

    #[tokio::test]
    async fn test_http_response_metadata_only() {
        let activity = HttpActivity::new();

        let params = HttpRequestParams {
            method: "GET".to_string(),
            url: "https://httpbin.org/get".to_string(),
            headers: None,
            query: None,
            body: None,
            timeout_seconds: None,
        };

        let response = activity.execute(params).await.unwrap();
        let http_response = HttpResponse::from_response_metadata(response)
            .await
            .unwrap();

        assert_eq!(http_response.status, 200);
        assert!(http_response.success);
        // Body should not be loaded
        assert!(http_response.body.is_none());
    }

    #[tokio::test]
    async fn test_default_user_agent() {
        let activity = HttpActivity::new();

        let params = HttpRequestParams {
            method: "GET".to_string(),
            url: "https://httpbin.org/headers".to_string(),
            headers: None,
            query: None,
            body: None,
            timeout_seconds: None,
        };

        let response = activity.execute(params).await.unwrap();
        let http_response = HttpResponse::from_response_json(response).await.unwrap();

        assert_eq!(http_response.status, 200);

        // Verify default User-Agent was sent
        let body = http_response.body.as_ref().unwrap();
        let headers = body["headers"].as_object().unwrap();
        let user_agent = headers.get("User-Agent").unwrap().as_str().unwrap();
        assert!(user_agent.contains("StreamFlow"));
    }

    #[tokio::test]
    async fn test_user_agent_can_be_overridden() {
        let activity = HttpActivity::new();

        let params = HttpRequestParams {
            method: "GET".to_string(),
            url: "https://httpbin.org/headers".to_string(),
            headers: Some(HashMap::from([(
                "User-Agent".to_string(),
                "CustomAgent/1.0".to_string(),
            )])),
            query: None,
            body: None,
            timeout_seconds: None,
        };

        let response = activity.execute(params).await.unwrap();
        let http_response = HttpResponse::from_response_json(response).await.unwrap();

        assert_eq!(http_response.status, 200);

        // Verify custom User-Agent overrode default
        let body = http_response.body.as_ref().unwrap();
        let headers = body["headers"].as_object().unwrap();
        let user_agent = headers.get("User-Agent").unwrap().as_str().unwrap();
        assert_eq!(user_agent, "CustomAgent/1.0");
    }
}
