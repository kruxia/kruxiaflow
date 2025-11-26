//! WebSocket infrastructure for activity streaming.
//!
//! This module provides WebSocket support for streaming activity results,
//! particularly LLM token-by-token streaming for real-time AI responses.
//!
//! # Architecture
//!
//! ```text
//! Client ─────WebSocket────► API Server ◄────Events────► Worker
//!                               │
//!                      ConnectionManager
//!                     (activity_id → connections)
//! ```
//!
//! # Components
//!
//! - [`messages`]: Message protocol types for WebSocket communication
//! - [`connection_manager`]: Manages active connections per activity
//! - `handler`: Axum WebSocket route handler (Task 3)
//!
//! # Usage
//!
//! Clients connect via: `WS /api/v1/activities/{id}/stream?token=...`
//!
//! Messages are JSON-encoded [`StreamMessage`] variants:
//! - `Token`: Incremental LLM output
//! - `Complete`: Activity finished successfully
//! - `Error`: Activity failed
//! - `Ping`: Connection keepalive

pub mod connection_manager;
pub mod messages;

pub use connection_manager::{ConnectionId, ConnectionManager};
pub use messages::StreamMessage;
