use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Error codes for API responses
///
/// Uses serde's `rename_all = "SCREAMING_SNAKE_CASE"` to automatically
/// convert variant names to uppercase with underscores (e.g., ValidationError -> VALIDATION_ERROR).
///
/// This provides:
/// - Type-safe error codes (not strings)
/// - Automatic case conversion via serde
/// - Pattern matching support for clients
/// - Clean OpenAPI enum schema generation
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    BadRequest,
    Unauthorized,
    Forbidden,
    NotFound,
    Conflict,
    ValidationError,
    RateLimitExceeded,
    InternalError,
    DatabaseError,
    ServiceUnavailable,
}

/// Standard API error response format
///
/// All API errors follow this format for consistency:
/// ```json
/// {
///   "error": {
///     "code": "VALIDATION_ERROR",
///     "message": "Request validation failed",
///     "details": {
///       "field_errors": {
///         "email": ["Invalid email format"],
///         "amount": ["Must be positive"]
///       }
///     }
///   }
/// }
/// ```
#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ApiErrorResponse {
    pub error: ApiError,
}

#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ApiError {
    /// Machine-readable error code (automatically serialized as SCREAMING_SNAKE_CASE)
    pub code: ErrorCode,

    /// Human-readable error message
    #[schema(example = "Request validation failed")]
    pub message: String,

    /// Optional additional error details
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

impl ApiErrorResponse {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            error: ApiError {
                code,
                message: message.into(),
                details: None,
            },
        }
    }

    pub fn with_details(mut self, details: serde_json::Value) -> Self {
        self.error.details = Some(details);
        self
    }
}

/// Application error type that maps to HTTP responses
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    /// 400 Bad Request - Client sent invalid request
    #[error("Bad request: {0}")]
    BadRequest(String),

    /// 401 Unauthorized - Authentication required or failed
    #[error("Unauthorized: {0}")]
    Unauthorized(String),

    /// 403 Forbidden - Authenticated but not authorized
    #[error("Forbidden: {0}")]
    Forbidden(String),

    /// 404 Not Found - Resource does not exist
    #[error("Not found: {0}")]
    NotFound(String),

    /// 409 Conflict - Request conflicts with current state
    #[error("Conflict: {0}")]
    Conflict(String),

    /// 422 Unprocessable Entity - Validation failed
    #[error("Validation failed")]
    ValidationError(ValidationErrors),

    /// 429 Too Many Requests - Rate limit exceeded
    #[error("Rate limit exceeded: {0}")]
    RateLimitExceeded(String),

    /// 500 Internal Server Error - Unexpected server error
    #[error("Internal server error")]
    InternalError(#[from] anyhow::Error),

    /// 503 Service Unavailable - Service temporarily unavailable
    #[error("Service unavailable: {0}")]
    ServiceUnavailable(String),

    /// Database errors
    #[error("Database error: {0}")]
    DatabaseError(#[from] sqlx::Error),
}

/// Field-level validation errors
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationErrors {
    pub field_errors: HashMap<String, Vec<String>>,
}

impl ValidationErrors {
    pub fn new() -> Self {
        Self {
            field_errors: HashMap::new(),
        }
    }

    pub fn add(&mut self, field: impl Into<String>, error: impl Into<String>) {
        self.field_errors
            .entry(field.into())
            .or_default()
            .push(error.into());
    }

    pub fn is_empty(&self) -> bool {
        self.field_errors.is_empty()
    }

    /// Check if there's an error for a specific field (useful for testing)
    #[cfg(test)]
    pub fn has_error(&self, field: &str) -> bool {
        self.field_errors.contains_key(field)
    }
}

impl Default for ValidationErrors {
    fn default() -> Self {
        Self::new()
    }
}

impl ValidationErrors {
    /// Convert workflow validation errors to API validation errors
    pub fn from_workflow_validation(ve: streamflow_core::workflow::ValidationError) -> Self {
        use streamflow_core::workflow::ValidationError as WfValidationError;

        match ve {
            WfValidationError::SingleError(msg) => {
                let mut errors = Self::new();
                errors.add("definition", msg);
                errors
            }
            WfValidationError::MultipleErrors(errs) => {
                let mut errors = Self::new();
                for (field, messages) in errs.errors() {
                    for message in messages {
                        errors.add(field, message);
                    }
                }
                errors
            }
        }
    }
}

impl AppError {
    /// Get HTTP status code for this error
    pub fn status_code(&self) -> StatusCode {
        match self {
            AppError::BadRequest(_) => StatusCode::BAD_REQUEST,
            AppError::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            AppError::Forbidden(_) => StatusCode::FORBIDDEN,
            AppError::NotFound(_) => StatusCode::NOT_FOUND,
            AppError::Conflict(_) => StatusCode::CONFLICT,
            AppError::ValidationError(_) => StatusCode::UNPROCESSABLE_ENTITY,
            AppError::RateLimitExceeded(_) => StatusCode::TOO_MANY_REQUESTS,
            AppError::InternalError(_) | AppError::DatabaseError(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
            AppError::ServiceUnavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
        }
    }

    /// Get machine-readable error code
    ///
    /// Maps AppError variants to ErrorCode enum, which serde automatically
    /// serializes as SCREAMING_SNAKE_CASE (e.g., ValidationError -> VALIDATION_ERROR).
    pub fn error_code(&self) -> ErrorCode {
        match self {
            AppError::BadRequest(_) => ErrorCode::BadRequest,
            AppError::Unauthorized(_) => ErrorCode::Unauthorized,
            AppError::Forbidden(_) => ErrorCode::Forbidden,
            AppError::NotFound(_) => ErrorCode::NotFound,
            AppError::Conflict(_) => ErrorCode::Conflict,
            AppError::ValidationError(_) => ErrorCode::ValidationError,
            AppError::RateLimitExceeded(_) => ErrorCode::RateLimitExceeded,
            AppError::InternalError(_) => ErrorCode::InternalError,
            AppError::DatabaseError(_) => ErrorCode::DatabaseError,
            AppError::ServiceUnavailable(_) => ErrorCode::ServiceUnavailable,
        }
    }

    /// Convert to API error response
    pub fn to_response(&self) -> ApiErrorResponse {
        let mut response = ApiErrorResponse::new(self.error_code(), self.to_string());

        // Add field-level details for validation errors
        if let AppError::ValidationError(validation_errors) = self {
            response = response.with_details(serde_json::json!({
                "field_errors": validation_errors.field_errors
            }));
        }

        response
    }
}

/// Implement IntoResponse for AppError to integrate with Axum
impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = self.status_code();
        let body = self.to_response();

        // Log internal errors but don't expose details to clients
        if matches!(
            self,
            AppError::InternalError(_) | AppError::DatabaseError(_)
        ) {
            tracing::error!("Internal error: {:?}", self);
        }

        (status, Json(body)).into_response()
    }
}

/// Helper type alias for API results
pub type ApiResult<T> = Result<T, AppError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_response_format() {
        let error = AppError::NotFound("Workflow not found".to_string());
        let response = error.to_response();

        assert_eq!(response.error.code, ErrorCode::NotFound);
        assert_eq!(response.error.message, "Not found: Workflow not found");
        assert!(response.error.details.is_none());
    }

    #[test]
    fn test_validation_error_with_details() {
        let mut validation_errors = ValidationErrors::new();
        validation_errors.add("email", "Invalid email format");
        validation_errors.add("amount", "Must be positive");

        let error = AppError::ValidationError(validation_errors);
        let response = error.to_response();

        assert_eq!(response.error.code, ErrorCode::ValidationError);
        assert!(response.error.details.is_some());

        let details = response.error.details.unwrap();
        let field_errors = &details["field_errors"];
        assert!(field_errors["email"].as_array().unwrap().len() == 1);
        assert!(field_errors["amount"].as_array().unwrap().len() == 1);
    }

    #[test]
    fn test_error_code_serialization() {
        // Test that ErrorCode serializes to SCREAMING_SNAKE_CASE
        let error = ApiErrorResponse::new(ErrorCode::ValidationError, "Validation failed");

        let json = serde_json::to_string(&error).unwrap();
        assert!(json.contains("\"code\":\"VALIDATION_ERROR\""));
    }

    #[test]
    fn test_error_status_codes() {
        assert_eq!(
            AppError::BadRequest("test".into()).status_code(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            AppError::Unauthorized("test".into()).status_code(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            AppError::NotFound("test".into()).status_code(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            AppError::Conflict("test".into()).status_code(),
            StatusCode::CONFLICT
        );
        assert_eq!(
            AppError::ValidationError(ValidationErrors::new()).status_code(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }

    #[test]
    fn test_validation_errors_default() {
        let errors = ValidationErrors::default();
        assert!(errors.is_empty());
        assert_eq!(errors.field_errors.len(), 0);
    }

    #[test]
    fn test_validation_errors_add() {
        let mut errors = ValidationErrors::new();
        errors.add("email", "Invalid format");
        errors.add("email", "Already exists");
        errors.add("name", "Required");

        assert!(!errors.is_empty());
        assert_eq!(errors.field_errors.len(), 2);
        assert_eq!(errors.field_errors["email"].len(), 2);
        assert_eq!(errors.field_errors["name"].len(), 1);
    }
}
