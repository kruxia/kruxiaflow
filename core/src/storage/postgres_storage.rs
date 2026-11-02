use super::file_rows::{self, StoredLocation};
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

    /// Stream a Large Object's content in its own read transaction.
    pub(super) fn large_object_stream(
        pool: PgPool,
        oid: u32,
    ) -> Pin<Box<dyn Stream<Item = std::result::Result<Bytes, std::io::Error>> + Send>> {
        let stream = async_stream::try_stream! {
            let mut tx = pool.begin().await
                .map_err(std::io::Error::other)?;

            // Open Large Object for reading
            let fd = Self::open_large_object(&mut tx, oid, INV_READ).await
                .map_err(std::io::Error::other)?;

            loop {
                // Read chunk from Large Object
                let chunk = Self::read_from_large_object(&mut tx, fd, CHUNK_SIZE as i32).await
                    .map_err(std::io::Error::other)?;

                if chunk.is_empty() {
                    break;
                }

                // Here be macros pretending to be generators. Thanks, async-stream!
                yield Bytes::from(chunk);
            }

            // Close Large Object
            Self::close_large_object(&mut tx, fd).await
                .map_err(std::io::Error::other)?;

            tx.commit().await
                .map_err(std::io::Error::other)?;
        };

        Box::pin(stream)
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

        // Insert metadata (replace if exists) and clean up the replaced content
        let (metadata, previous) = file_rows::replace(
            &mut tx,
            workflow_id,
            activity_key,
            filename,
            StoredLocation::LargeObject(oid),
            total_size,
            content_type,
        )
        .await?;

        match previous {
            Some(StoredLocation::LargeObject(old_oid)) => {
                file_rows::unlink_large_object(&mut tx, old_oid).await?;
            }
            Some(StoredLocation::Object(key)) => warn_unreachable_object(filename, &key),
            None => {}
        }

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
        let oid = match file_rows::location(&self.pool, workflow_id, activity_key, filename).await?
        {
            Some(StoredLocation::LargeObject(oid)) => oid,
            Some(StoredLocation::Object(key)) => {
                return Err(StorageError::WrongProvider(
                    filename.to_string(),
                    format!(
                        "content is object {key} in S3-compatible storage; \
                         set KRUXIAFLOW_STORAGE_PROVIDER=s3 to read it"
                    ),
                ));
            }
            None => return Err(StorageError::FileNotFound(filename.to_string())),
        };

        Ok(Self::large_object_stream(self.pool.clone(), oid))
    }

    async fn get_file_metadata(
        &self,
        workflow_id: Uuid,
        activity_key: &str,
        filename: &str,
    ) -> Result<FileMetadata> {
        file_rows::metadata(&self.pool, workflow_id, activity_key, filename).await
    }

    async fn list_files(&self, workflow_id: Uuid, activity_key: &str) -> Result<Vec<FileMetadata>> {
        file_rows::list(&self.pool, workflow_id, activity_key).await
    }

    async fn delete_file(
        &self,
        workflow_id: Uuid,
        activity_key: &str,
        filename: &str,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;

        match file_rows::delete(&mut tx, workflow_id, activity_key, filename).await? {
            Some(StoredLocation::LargeObject(oid)) => {
                file_rows::unlink_large_object(&mut tx, oid).await?;
            }
            Some(StoredLocation::Object(key)) => warn_unreachable_object(filename, &key),
            None => {}
        }

        tx.commit().await?;
        Ok(())
    }

    async fn delete_workflow_files(&self, workflow_id: Uuid) -> Result<()> {
        let mut tx = self.pool.begin().await?;

        for location in file_rows::delete_workflow(&mut tx, workflow_id).await? {
            match location {
                StoredLocation::LargeObject(oid) => {
                    file_rows::unlink_large_object(&mut tx, oid).await?;
                }
                StoredLocation::Object(key) => {
                    warn_unreachable_object(&workflow_id.to_string(), &key)
                }
            }
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

/// The postgresql provider has no object storage client, so content written by
/// the s3 provider is left in the bucket when its row is replaced or deleted.
fn warn_unreachable_object(file: &str, key: &str) {
    tracing::warn!(
        file,
        object_key = key,
        "Removed metadata for a file stored in S3-compatible storage; the postgresql \
         storage provider cannot delete the object, so it remains in the bucket"
    );
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
