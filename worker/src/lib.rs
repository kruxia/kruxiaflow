//! Kruxia Flow std worker: built-in activities (echo, HTTP, PostgreSQL,
//! LLM, email) with caching and streaming, running on the public
//! [`kruxiaflow-worker`](https://docs.rs/kruxiaflow-worker) SDK's poll loop.

pub mod activities;
pub mod executor;
pub mod file_executor;
pub mod llm;
pub mod manager;
pub mod registry;
pub mod std_worker;
pub mod streaming;

pub use activities::{
    EchoActivity, EmailSendActivity, EmbeddingActivity, HttpRequestActivity, LLMPromptActivity,
    PostgresQueryActivity, new_pool_cache,
};
pub use executor::StdActivityExecutor;
pub use manager::WorkerManager;
pub use registry::{ActivityImpl, ActivityRegistry};
pub use std_worker::register_std_activities;
pub use streaming::{
    CollectingStreamSender, HttpStreamSender, NoOpStreamSender, StreamError, StreamSender,
    StreamToken, StreamingActivity,
};

// Re-exports from the SDK crate so consumers of the std worker use the same
// protocol types.
pub use kruxiaflow_worker::{
    ActivityOutput, ActivityResult, OutputType, PendingActivity, UsageEntry, WorkerApiClient,
    WorkerConfig, WorkerPoller,
};
