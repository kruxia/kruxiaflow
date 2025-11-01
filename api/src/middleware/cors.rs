use axum::http::{HeaderName, Method};
use std::time::Duration;
use tower_http::cors::{Any, CorsLayer};

/// Create CORS layer for API server
///
/// Configures CORS to:
/// - Allow all origins for development (should be restricted in production)
/// - Allow standard HTTP methods (GET, POST, PUT, DELETE, PATCH)
/// - Allow custom headers (X-Request-ID, Authorization)
/// - Cache preflight responses for 1 hour
///
/// Note: allow_credentials is intentionally NOT enabled with Any origin
/// as this is a browser security restriction. In production, configure
/// specific allowed origins and enable credentials as needed.
pub fn cors_layer() -> CorsLayer {
    CorsLayer::new()
        // Allow all origins for development (should be restricted in production)
        .allow_origin(Any)
        // Allow standard HTTP methods
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
            Method::OPTIONS,
        ])
        // Allow common headers
        .allow_headers([
            HeaderName::from_static("content-type"),
            HeaderName::from_static("authorization"),
            HeaderName::from_static("x-request-id"),
            HeaderName::from_static("accept"),
        ])
        // Expose custom headers to JavaScript
        .expose_headers([
            HeaderName::from_static("x-request-id"),
            HeaderName::from_static("content-type"),
            HeaderName::from_static("content-length"),
        ])
        // Cache preflight responses for 1 hour
        .max_age(Duration::from_secs(3600))
}

/// Configuration for CORS (future: make configurable)
#[derive(Debug, Clone)]
pub struct CorsConfig {
    /// Allowed origins (e.g., ["https://app.example.com"])
    pub allowed_origins: Vec<String>,

    /// Whether to allow credentials
    pub allow_credentials: bool,

    /// Max age for preflight cache
    pub max_age_seconds: u64,
}

impl Default for CorsConfig {
    fn default() -> Self {
        Self {
            allowed_origins: vec!["*".to_string()],
            allow_credentials: true,
            max_age_seconds: 3600,
        }
    }
}
