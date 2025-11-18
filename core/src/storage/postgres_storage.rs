use super::{FileMetadata, Result, StorageError, WorkflowStorage};
use async_trait::async_trait;
use bytes::Bytes;
use futures::stream::{Stream, StreamExt};
use sqlx::{PgPool, Postgres, Transaction};
use std::pin::Pin;
use uuid::Uuid;

const CHUNK_SIZE: usize = 8192; // 8KB chunks for streaming
const INV_WRITE: i32 = 0x00020000; // PostgreSQL Large Object write mode
const INV_READ: i32 = 0x00040000; // PostgreSQL Large Object read mode

pub struct PostgresStorage {
    pool: PgPool,
}

impl PostgresStorage {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Helper to create a new Large Object and return its OID
    async fn create_large_object(tx: &mut Transaction<'_, Postgres>) -> Result<u32> {
        let oid: i32 = sqlx::query_scalar("SELECT lo_create(0)::int4")
            .fetch_one(&mut **tx)
            .await?;
        Ok(oid as u32)
    }

    /// Helper to open a Large Object for reading or writing
    async fn open_large_object(
        tx: &mut Transaction<'_, Postgres>,
        oid: u32,
        mode: i32,
    ) -> Result<i32> {
        let fd: i32 = sqlx::query_scalar("SELECT lo_open($1, $2)")
            .bind(oid as i32)
            .bind(mode)
            .fetch_one(&mut **tx)
            .await?;
        Ok(fd)
    }

    /// Helper to close a Large Object
    async fn close_large_object(tx: &mut Transaction<'_, Postgres>, fd: i32) -> Result<()> {
        sqlx::query("SELECT lo_close($1)")
            .bind(fd)
            .execute(&mut **tx)
            .await?;
        Ok(())
    }

    /// Helper to write data to a Large Object
    async fn write_to_large_object(
        tx: &mut Transaction<'_, Postgres>,
        fd: i32,
        data: &[u8],
    ) -> Result<()> {
        sqlx::query("SELECT lowrite($1, $2)")
            .bind(fd)
            .bind(data)
            .execute(&mut **tx)
            .await?;
        Ok(())
    }

    /// Helper to read data from a Large Object
    async fn read_from_large_object(
        tx: &mut Transaction<'_, Postgres>,
        fd: i32,
        len: i32,
    ) -> Result<Vec<u8>> {
        let data: Vec<u8> = sqlx::query_scalar("SELECT loread($1, $2)")
            .bind(fd)
            .bind(len)
            .fetch_one(&mut **tx)
            .await?;
        Ok(data)
    }

    /// Helper to delete a Large Object
    async fn delete_large_object(tx: &mut Transaction<'_, Postgres>, oid: u32) -> Result<()> {
        sqlx::query("SELECT lo_unlink($1)")
            .bind(oid as i32)
            .execute(&mut **tx)
            .await?;
        Ok(())
    }
}

#[async_trait]
impl WorkflowStorage for PostgresStorage {
    async fn upload_file(
        &self,
        workflow_id: Uuid,
        activity_key: &str,
        filename: &str,
        content_type: Option<&str>,
        mut data: Pin<
            Box<dyn Stream<Item = std::result::Result<Bytes, std::io::Error>> + Send + Unpin>,
        >,
    ) -> Result<FileMetadata> {
        let mut tx = self.pool.begin().await?;

        // Create Large Object
        let oid = Self::create_large_object(&mut tx).await?;

        // Open Large Object for writing
        let fd = Self::open_large_object(&mut tx, oid, INV_WRITE).await?;

        // Stream write data to Large Object
        let mut total_size = 0i64;

        while let Some(chunk_result) = data.next().await {
            let chunk = chunk_result?;
            let chunk_size = chunk.len() as i64;
            Self::write_to_large_object(&mut tx, fd, &chunk).await?;
            total_size += chunk_size;
        }

        // Close Large Object
        Self::close_large_object(&mut tx, fd).await?;

        // Insert metadata (upsert - replace if exists)
        let metadata = sqlx::query_as::<_, FileMetadata>(
            r#"
            INSERT INTO workflow_files
                (workflow_id, activity_key, filename, oid, size, content_type)
            VALUES ($1, $2, $3, $4, $5, $6)
            ON CONFLICT (workflow_id, activity_key, filename)
            DO UPDATE SET
                oid = EXCLUDED.oid,
                size = EXCLUDED.size,
                content_type = EXCLUDED.content_type,
                created_at = NOW()
            RETURNING workflow_id, activity_key, filename, size, content_type, created_at
            "#,
        )
        .bind(workflow_id)
        .bind(activity_key)
        .bind(filename)
        .bind(oid as i32)
        .bind(total_size)
        .bind(content_type)
        .fetch_one(&mut *tx)
        .await?;

        tx.commit().await?;

        Ok(metadata)
    }

    async fn download_file(
        &self,
        workflow_id: Uuid,
        activity_key: &str,
        filename: &str,
    ) -> Result<Pin<Box<dyn Stream<Item = std::result::Result<Bytes, std::io::Error>> + Send>>>
    {
        // Get OID from metadata
        let oid: i32 = sqlx::query_scalar(
            "SELECT oid::int4 FROM workflow_files
             WHERE workflow_id = $1 AND activity_key = $2 AND filename = $3",
        )
        .bind(workflow_id)
        .bind(activity_key)
        .bind(filename)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| StorageError::FileNotFound(filename.to_string()))?;

        // Create a stream that reads from the Large Object
        let pool = self.pool.clone();
        let stream = async_stream::try_stream! {
            let mut tx = pool.begin().await
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

            // Open Large Object for reading
            let fd = Self::open_large_object(&mut tx, oid as u32, INV_READ).await
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

            loop {
                // Read chunk from Large Object
                let chunk = Self::read_from_large_object(&mut tx, fd, CHUNK_SIZE as i32).await
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

                if chunk.is_empty() {
                    break;
                }

                // Here be macros pretending to be generators. Thanks, async-stream!
                yield Bytes::from(chunk);
            }

            // Close Large Object
            Self::close_large_object(&mut tx, fd).await
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

            tx.commit().await
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        };

        Ok(Box::pin(stream))
    }

    async fn get_file_metadata(
        &self,
        workflow_id: Uuid,
        activity_key: &str,
        filename: &str,
    ) -> Result<FileMetadata> {
        let metadata = sqlx::query_as::<_, FileMetadata>(
            "SELECT workflow_id, activity_key, filename, size, content_type, created_at
             FROM workflow_files
             WHERE workflow_id = $1 AND activity_key = $2 AND filename = $3",
        )
        .bind(workflow_id)
        .bind(activity_key)
        .bind(filename)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| StorageError::FileNotFound(filename.to_string()))?;

        Ok(metadata)
    }

    async fn list_files(&self, workflow_id: Uuid, activity_key: &str) -> Result<Vec<FileMetadata>> {
        let files = sqlx::query_as::<_, FileMetadata>(
            "SELECT workflow_id, activity_key, filename, size, content_type, created_at
             FROM workflow_files
             WHERE workflow_id = $1 AND activity_key = $2
             ORDER BY created_at",
        )
        .bind(workflow_id)
        .bind(activity_key)
        .fetch_all(&self.pool)
        .await?;

        Ok(files)
    }

    async fn delete_file(
        &self,
        workflow_id: Uuid,
        activity_key: &str,
        filename: &str,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;

        // Delete metadata and get OID in one query
        let oid: Option<i32> = sqlx::query_scalar(
            "DELETE FROM workflow_files
             WHERE workflow_id = $1 AND activity_key = $2 AND filename = $3
             RETURNING oid::int4",
        )
        .bind(workflow_id)
        .bind(activity_key)
        .bind(filename)
        .fetch_optional(&mut *tx)
        .await?;

        if let Some(oid) = oid {
            // Delete Large Object
            Self::delete_large_object(&mut tx, oid as u32).await?;
        }

        tx.commit().await?;
        Ok(())
    }

    async fn delete_workflow_files(&self, workflow_id: Uuid) -> Result<()> {
        let mut tx = self.pool.begin().await?;

        // Delete metadata and get all OIDs in one query
        let oids: Vec<i32> = sqlx::query_scalar(
            "DELETE FROM workflow_files WHERE workflow_id = $1 RETURNING oid::int4",
        )
        .bind(workflow_id)
        .fetch_all(&mut *tx)
        .await?;

        // Delete all Large Objects
        for oid in oids {
            Self::delete_large_object(&mut tx, oid as u32).await?;
        }

        tx.commit().await?;
        Ok(())
    }

    async fn get_file_reference(
        &self,
        workflow_id: Uuid,
        activity_key: &str,
        filename: &str,
    ) -> Result<String> {
        // Verify file exists
        self.get_file_metadata(workflow_id, activity_key, filename)
            .await?;

        // For PostgreSQL storage, return an internal reference
        // Format: postgres://{workflow_id}/{activity_key}/{filename}
        Ok(format!(
            "postgres://{}/{}/{}",
            workflow_id, activity_key, filename
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::stream;

    // Note: These tests require a running PostgreSQL instance
    // They are integration tests and should be run with:
    // cargo test --test storage_tests -- --ignored

    #[sqlx::test]
    #[ignore]
    async fn test_upload_and_download_file(pool: PgPool) -> Result<()> {
        let storage = PostgresStorage::new(pool);
        let workflow_id = Uuid::now_v7();
        let activity_key = "test_activity";
        let filename = "test.txt";
        let content = b"Hello, World!";

        // Upload file
        let data_stream: Pin<
            Box<
                dyn futures::Stream<Item = std::result::Result<Bytes, std::io::Error>>
                    + Send
                    + Unpin,
            >,
        > = Box::pin(stream::iter(vec![Ok(Bytes::from_static(content))]));
        let metadata = storage
            .upload_file(
                workflow_id,
                activity_key,
                filename,
                Some("text/plain"),
                data_stream,
            )
            .await?;

        assert_eq!(metadata.workflow_id, workflow_id);
        assert_eq!(metadata.activity_key, activity_key);
        assert_eq!(metadata.filename, filename);
        assert_eq!(metadata.size, content.len() as i64);

        // Download file
        let mut download_stream = storage
            .download_file(workflow_id, activity_key, filename)
            .await?;

        let mut downloaded = Vec::new();
        while let Some(chunk) = download_stream.next().await {
            let chunk = chunk?;
            downloaded.extend_from_slice(&chunk);
        }

        assert_eq!(downloaded, content);

        Ok(())
    }

    #[sqlx::test]
    #[ignore]
    async fn test_delete_file(pool: PgPool) -> Result<()> {
        let storage = PostgresStorage::new(pool);
        let workflow_id = Uuid::now_v7();
        let activity_key = "test_activity";
        let filename = "test.txt";
        let content = b"Hello, World!";

        // Upload file
        let data_stream: Pin<
            Box<
                dyn futures::Stream<Item = std::result::Result<Bytes, std::io::Error>>
                    + Send
                    + Unpin,
            >,
        > = Box::pin(stream::iter(vec![Ok(Bytes::from_static(content))]));
        storage
            .upload_file(
                workflow_id,
                activity_key,
                filename,
                Some("text/plain"),
                data_stream,
            )
            .await?;

        // Delete file
        storage
            .delete_file(workflow_id, activity_key, filename)
            .await?;

        // Verify file is gone
        let result = storage
            .get_file_metadata(workflow_id, activity_key, filename)
            .await;

        assert!(matches!(result, Err(StorageError::FileNotFound(_))));

        Ok(())
    }
}
