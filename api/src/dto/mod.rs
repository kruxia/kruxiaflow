//! API Data Transfer Objects (DTOs)
//!
//! This module contains transparent wrappers around core types to isolate
//! API-specific concerns (like OpenAPI schema generation) from the core domain.

pub mod workflow;

pub use workflow::*;
