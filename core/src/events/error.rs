use thiserror::Error;

#[derive(Error, Debug)]
pub enum EventError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Event not found: {0}")]
    NotFound(String),

    #[error("Invalid event: {0}")]
    Invalid(String),

    /// Events or consumer cursors carry transaction ids this database
    /// cluster cannot have assigned (see `events::rebase`)
    #[error(
        "Workflow event cursors were written by a different PostgreSQL cluster: \
         highest stored transaction id is {max_stored_tx}, but this cluster's next \
         transaction id is {next_xid}. This happens after a pg_dump/restore or a \
         logical-replication migration, and events would not be delivered. Stop every \
         Kruxia Flow process, then run `kruxiaflow rebase-events`."
    )]
    ForeignCluster { max_stored_tx: i64, next_xid: i64 },
}

pub type Result<T> = std::result::Result<T, EventError>;
