pub mod dto;
pub mod error;
pub mod extractors;
pub mod handlers;
pub mod health;
pub mod middleware;
pub mod openapi;
pub mod routes;
pub mod state;
pub mod websocket;
pub mod workflow_events;

// Re-export commonly used items
pub use error::{ApiError, ApiErrorResponse, ApiResult, AppError, ErrorCode, ValidationErrors};
pub use routes::{app_router, protected_routes, public_routes};
pub use state::{AppState, AppStateBuild};
pub use websocket::StreamMessage;
