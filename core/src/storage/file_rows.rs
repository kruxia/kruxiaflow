//! Shared access to the `workflow_files` metadata table.
//!
//! Every storage provider keeps file metadata in PostgreSQL; only the content
//! location differs (a Large Object OID or an object storage key). Keeping the
//! row logic here lets each provider recognise rows written by another one,
//! so switching providers never strands existing files.

use super::{FileMetadata, Result, StorageError};
use sqlx::{PgExecutor, Postgres, Transaction};
use uuid::Uuid;

/// Where a file's content is stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum StoredLocation {
    /// PostgreSQL Large Object (postgresql provider)
    LargeObject(u32),
    /// Object key within the configured bucket (s3 provider)
    Object(String),
}

impl StoredLocation {
    fn from_columns(oid: Option<i32>, storage_key: Option<String>) -> Result<Self> {
        match (oid, storage_key) {
            (Some(oid), None) => Ok(Self::LargeObject(oid as u32)),
            (None, Some(key)) => Ok(Self::Object(key)),
            // Excluded by the workflow_files_single_location constraint
            (oid, key) => Err(StorageError::InvalidFileReference(format!(
                "workflow_files row has oid={oid:?} and storage_key={key:?}; exactly one is expected"
            ))),
        }
    }

    fn into_columns(self) -> (Option<i32>, Option<String>) {
        match self {
            Self::LargeObject(oid) => (Some(oid as i32), None),
            Self::Object(key) => (None, Some(key)),
        }
    }
}

type LocationRow = (Option<i32>, Option<String>);

/// Look up where a file's content is stored, or `None` if the file does not exist.
pub(super) async fn location<'e>(
    executor: impl PgExecutor<'e>,
    workflow_id: Uuid,
    activity_key: &str,
    filename: &str,
) -> Result<Option<StoredLocation>> {
    let row: Option<LocationRow> = sqlx::query_as(
        "SELECT oid::int4, storage_key FROM workflow_files
         WHERE workflow_id = $1 AND activity_key = $2 AND filename = $3",
    )
    .bind(workflow_id)
    .bind(activity_key)
    .bind(filename)
    .fetch_optional(executor)
    .await?;

    row.map(|(oid, key)| StoredLocation::from_columns(oid, key))
        .transpose()
}

/// Insert or replace a file's metadata row, returning the new metadata and the
/// location of the content it replaced (which the caller must clean up).
///
/// Concurrent writers of the same file are serialized with a transaction-scoped
/// advisory lock, so every replaced location is reported to exactly one writer.
pub(super) async fn replace(
    tx: &mut Transaction<'_, Postgres>,
    workflow_id: Uuid,
    activity_key: &str,
    filename: &str,
    location: StoredLocation,
    size: i64,
    content_type: Option<&str>,
) -> Result<(FileMetadata, Option<StoredLocation>)> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1 || '/' || $2 || '/' || $3, 0))")
        .bind(workflow_id.to_string())
        .bind(activity_key)
        .bind(filename)
        .execute(&mut **tx)
        .await?;

    let previous = self::location(&mut **tx, workflow_id, activity_key, filename).await?;

    let (oid, storage_key) = location.into_columns();
    let metadata = sqlx::query_as::<_, FileMetadata>(
        r#"
        INSERT INTO workflow_files
            (workflow_id, activity_key, filename, oid, storage_key, size, content_type)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        ON CONFLICT (workflow_id, activity_key, filename)
        DO UPDATE SET
            oid = EXCLUDED.oid,
            storage_key = EXCLUDED.storage_key,
            size = EXCLUDED.size,
            content_type = EXCLUDED.content_type,
            created_at = NOW()
        RETURNING workflow_id, activity_key, filename, size, content_type, created_at
        "#,
    )
    .bind(workflow_id)
    .bind(activity_key)
    .bind(filename)
    .bind(oid)
    .bind(storage_key)
    .bind(size)
    .bind(content_type)
    .fetch_one(&mut **tx)
    .await?;

    Ok((metadata, previous))
}

/// Delete a file's metadata row, returning where its content was stored.
pub(super) async fn delete(
    tx: &mut Transaction<'_, Postgres>,
    workflow_id: Uuid,
    activity_key: &str,
    filename: &str,
) -> Result<Option<StoredLocation>> {
    let row: Option<LocationRow> = sqlx::query_as(
        "DELETE FROM workflow_files
         WHERE workflow_id = $1 AND activity_key = $2 AND filename = $3
         RETURNING oid::int4, storage_key",
    )
    .bind(workflow_id)
    .bind(activity_key)
    .bind(filename)
    .fetch_optional(&mut **tx)
    .await?;

    row.map(|(oid, key)| StoredLocation::from_columns(oid, key))
        .transpose()
}

/// Delete every metadata row of a workflow, returning where their content was stored.
pub(super) async fn delete_workflow(
    tx: &mut Transaction<'_, Postgres>,
    workflow_id: Uuid,
) -> Result<Vec<StoredLocation>> {
    let rows: Vec<LocationRow> = sqlx::query_as(
        "DELETE FROM workflow_files WHERE workflow_id = $1 RETURNING oid::int4, storage_key",
    )
    .bind(workflow_id)
    .fetch_all(&mut **tx)
    .await?;

    rows.into_iter()
        .map(|(oid, key)| StoredLocation::from_columns(oid, key))
        .collect()
}

/// Get a file's metadata.
pub(super) async fn metadata<'e>(
    executor: impl PgExecutor<'e>,
    workflow_id: Uuid,
    activity_key: &str,
    filename: &str,
) -> Result<FileMetadata> {
    sqlx::query_as::<_, FileMetadata>(
        "SELECT workflow_id, activity_key, filename, size, content_type, created_at
         FROM workflow_files
         WHERE workflow_id = $1 AND activity_key = $2 AND filename = $3",
    )
    .bind(workflow_id)
    .bind(activity_key)
    .bind(filename)
    .fetch_optional(executor)
    .await?
    .ok_or_else(|| StorageError::FileNotFound(filename.to_string()))
}

/// List the metadata of every file of an activity, oldest first.
pub(super) async fn list<'e>(
    executor: impl PgExecutor<'e>,
    workflow_id: Uuid,
    activity_key: &str,
) -> Result<Vec<FileMetadata>> {
    Ok(sqlx::query_as::<_, FileMetadata>(
        "SELECT workflow_id, activity_key, filename, size, content_type, created_at
         FROM workflow_files
         WHERE workflow_id = $1 AND activity_key = $2
         ORDER BY created_at",
    )
    .bind(workflow_id)
    .bind(activity_key)
    .fetch_all(executor)
    .await?)
}

/// Unlink a Large Object.
pub(super) async fn unlink_large_object(
    tx: &mut Transaction<'_, Postgres>,
    oid: u32,
) -> Result<()> {
    sqlx::query("SELECT lo_unlink($1)")
        .bind(oid as i32)
        .execute(&mut **tx)
        .await?;
    Ok(())
}
