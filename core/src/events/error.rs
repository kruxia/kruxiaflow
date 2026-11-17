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

impl EventError {
    /// Whether this is the database being out of reach (see
    /// [`is_connection_error`]), which passes, rather than a fault in the
    /// events or the query
    pub fn is_connection_error(&self) -> bool {
        match self {
            EventError::Database(e) => is_connection_error(e),
            _ => false,
        }
    }
}

/// Whether a database error means the database could not be reached or went
/// away mid-query: a restarted or recreated PostgreSQL, a network drop, or a
/// pool with no connection free in time. Retrying the same statement later
/// can succeed. Every other error (a bad query, a constraint, a decode) is
/// not of this kind and would fail again.
///
/// `Protocol` is included because a peer that is not (yet) a PostgreSQL
/// server produces it: while the database's container is gone, a port
/// proxy in front of it accepts the connection and closes it, and the TLS
/// negotiation reads "unexpected response from SSLRequest".
pub fn is_connection_error(e: &sqlx::Error) -> bool {
    match e {
        sqlx::Error::Io(_)
        | sqlx::Error::Tls(_)
        | sqlx::Error::Protocol(_)
        | sqlx::Error::PoolTimedOut
        | sqlx::Error::WorkerCrashed => true,
        // Class 08 is connection_exception; 57P01-57P03 are the server
        // shutting down (admin_shutdown, crash_shutdown) or still starting
        // (cannot_connect_now)
        sqlx::Error::Database(db) => db
            .code()
            .is_some_and(|code| code.starts_with("08") || code.starts_with("57P0")),
        _ => false,
    }
}
