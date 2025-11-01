use crate::error::{ApiError, ApiErrorResponse, ErrorCode};
use crate::health::{LivenessResponse, ReadinessResponse, ServiceInfo};
use utoipa::OpenApi;

/// OpenAPI specification for StreamFlow API
///
/// This struct defines the complete API documentation using utoipa macros.
/// Schemas are automatically generated from Rust types at compile time.
///
/// Note: Contact info should match workspace authors in Cargo.toml
#[derive(OpenApi)]
#[openapi(
    info(
        title = "StreamFlow API",
        version = env!("CARGO_PKG_VERSION"),
        description = env!("CARGO_PKG_DESCRIPTION"),
        contact(
            name = "Sean Harrison",
            email = "sah@kruxia.com",
        )
    ),
    servers(
        (url = "http://localhost:8080", description = "Local development server")
    ),
    paths(
        crate::handlers::health::liveness_handler,
        crate::handlers::health::readiness_handler,
        crate::handlers::health::service_info_handler,
    ),
    components(
        schemas(
            // Health check schemas
            LivenessResponse,
            ReadinessResponse,
            ServiceInfo,

            // Error response schemas
            ApiErrorResponse,
            ApiError,
            ErrorCode,
        )
    ),
    tags(
        (name = "Health", description = "Health check and service information endpoints"),
        (name = "Service", description = "Service metadata and capabilities"),
    ),
    modifiers(&SecurityAddon)
)]
pub struct ApiDoc;

/// Add security scheme to OpenAPI spec
struct SecurityAddon;

impl utoipa::Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        if let Some(components) = openapi.components.as_mut() {
            use utoipa::openapi::security::{Http, HttpAuthScheme, SecurityScheme};

            let mut http = Http::new(HttpAuthScheme::Bearer);
            http.bearer_format = Some("JWT".to_string());
            http.description = Some("JWT Bearer token authentication".to_string());

            components.add_security_scheme("bearer_auth", SecurityScheme::Http(http))
        }
    }
}
