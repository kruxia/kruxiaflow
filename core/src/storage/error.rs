use thiserror::Error;

pub type Result<T> = std::result::Result<T, StorageError>;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("File not found: {0}")]
    FileNotFound(String),

    #[error("Upload failed: {0}")]
    UploadFailed(String),

    #[error("Download failed: {0}")]
    DownloadFailed(String),

    #[error("Database error: {0}")]
    DatabaseError(#[from] sqlx::Error),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Invalid file reference: {0}")]
    InvalidFileReference(String),

    #[error("Object storage error: {0}")]
    ObjectStoreError(#[from] object_store::Error),

    #[error("Storage configuration error: {0}")]
    ConfigurationError(String),

    /// The file exists but its content lives in a backend this storage
    /// provider cannot reach (e.g. an S3 object read by the postgresql provider).
    #[error("File {0} is stored by a different storage provider: {1}")]
    WrongProvider(String, String),
}
