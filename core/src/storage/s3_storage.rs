//! S3-compatible WorkflowStorage backend.
//!
//! File content is stored as objects in a bucket (AWS S3, MinIO, Cloudflare R2,
//! Ceph RGW, ...); file metadata stays in the PostgreSQL `workflow_files`
//! table, so listing and metadata lookups never touch the bucket.
//!
//! Object keys have the form
//! `{prefix}/{workflow_id}/{activity_key}/{upload_id}/{filename}`. Each upload
//! writes a new object (`upload_id` is a fresh UUIDv7) and then swaps the
//! metadata row to point at it, so a row always describes exactly the object
//! it names: a download that races an overwrite reads either the complete old
//! file or the complete new one, never the new content with the old size.
//! Key segments are percent-encoded, so activity keys and filenames that
//! contain `/` cannot collide.
//!
//! Rows written by the postgresql provider (Large Objects) remain readable and
//! deletable, so a deployment can switch to this provider without migrating
//! existing files.

use super::file_rows::{self, StoredLocation};
use super::{FileMetadata, FileReference, PostgresStorage, Result, StorageError, WorkflowStorage};
use async_trait::async_trait;
use bytes::Bytes;
use futures::stream::{self, Stream, StreamExt};
use object_store::aws::AmazonS3Builder;
use object_store::path::Path;
use object_store::{
    Attribute, Attributes, GetOptions, ObjectStore, PutMultipartOptions, PutOptions, PutPayload,
    WriteMultipart,
};
use sqlx::PgPool;
use std::pin::Pin;
use std::sync::Arc;
use uuid::Uuid;

/// Multipart upload part size. Files smaller than one part are written with a
/// single PUT. S3 allows at most 10,000 parts, so this caps a file at ~78 GiB.
const PART_SIZE: usize = 8 * 1024 * 1024;

/// Parts uploaded concurrently per file. Bounds upload memory to roughly
/// `PART_SIZE * (MAX_CONCURRENT_PARTS + 1)` per in-flight upload.
const MAX_CONCURRENT_PARTS: usize = 2;

type ByteStream = Pin<Box<dyn Stream<Item = std::result::Result<Bytes, std::io::Error>> + Send>>;

/// Connection settings for an S3-compatible bucket.
///
/// Credentials are not part of this config: they come from the standard AWS
/// sources (`AWS_ACCESS_KEY_ID`/`AWS_SECRET_ACCESS_KEY`/`AWS_SESSION_TOKEN`,
/// web identity tokens, ECS task roles, or EC2 instance metadata). Other
/// `AWS_*` settings understood by the client (e.g.
/// `AWS_VIRTUAL_HOSTED_STYLE_REQUEST`) apply as well; the values here take
/// precedence over them.
#[derive(Debug, Clone, Default)]
pub struct S3Config {
    /// Bucket name
    pub bucket: String,
    /// Region (defaults to `AWS_REGION`/`AWS_DEFAULT_REGION`, then `us-east-1`)
    pub region: Option<String>,
    /// Endpoint URL for non-AWS services, e.g. `http://minio:9000`.
    /// Plain `http://` endpoints are allowed.
    pub endpoint: Option<String>,
    /// Key prefix for every object, e.g. `kruxiaflow/artifacts`
    pub prefix: Option<String>,
}

pub struct S3Storage {
    pool: PgPool,
    store: Arc<dyn ObjectStore>,
    prefix: Path,
}

impl S3Storage {
    /// Connect to the bucket described by `config`.
    ///
    /// No request is made here; credential or bucket problems surface on the
    /// first file operation.
    pub fn new(pool: PgPool, config: &S3Config) -> Result<Self> {
        if config.bucket.trim().is_empty() {
            return Err(StorageError::ConfigurationError(
                "S3 storage requires a bucket name".to_string(),
            ));
        }

        let mut builder = AmazonS3Builder::from_env().with_bucket_name(&config.bucket);
        if let Some(region) = &config.region {
            builder = builder.with_region(region);
        }
        if let Some(endpoint) = &config.endpoint {
            builder = builder.with_endpoint(endpoint);
            if endpoint.starts_with("http://") {
                builder = builder.with_allow_http(true);
            }
        }
        let store = builder.build()?;

        Self::with_store(pool, Arc::new(store), config.prefix.as_deref())
    }

    /// Use an already-configured object store, e.g. one built with custom
    /// client options.
    pub fn with_store(
        pool: PgPool,
        store: Arc<dyn ObjectStore>,
        prefix: Option<&str>,
    ) -> Result<Self> {
        let prefix = Path::parse(prefix.unwrap_or_default())
            .map_err(|e| StorageError::ConfigurationError(format!("Invalid S3 key prefix: {e}")))?;
        Ok(Self {
            pool,
            store,
            prefix,
        })
    }

    /// Key for a new upload of a file.
    fn new_object_path(&self, workflow_id: Uuid, activity_key: &str, filename: &str) -> Path {
        let workflow_id = workflow_id.to_string();
        let upload_id = Uuid::now_v7().to_string();
        self.prefix
            .clone()
            .join(workflow_id.as_str())
            .join(activity_key)
            .join(upload_id.as_str())
            .join(filename)
    }

    /// Stream `data` to `path`, returning the number of bytes written.
    async fn write_object(
        &self,
        path: &Path,
        content_type: Option<&str>,
        mut data: Pin<
            Box<dyn Stream<Item = std::result::Result<Bytes, std::io::Error>> + Send + Unpin>,
        >,
    ) -> Result<u64> {
        let mut attributes = Attributes::new();
        if let Some(content_type) = content_type {
            attributes.insert(Attribute::ContentType, content_type.to_string().into());
        }

        // Buffer up to one part: files that fit are written with a single PUT.
        let mut buffered = Vec::new();
        let mut buffered_len = 0usize;
        while buffered_len < PART_SIZE {
            match data.next().await {
                Some(chunk) => {
                    let chunk = chunk?;
                    buffered_len += chunk.len();
                    buffered.push(chunk);
                }
                None => {
                    let payload: PutPayload = buffered.into_iter().collect();
                    let opts = PutOptions {
                        attributes,
                        ..Default::default()
                    };
                    self.store.put_opts(path, payload, opts).await?;
                    return Ok(buffered_len as u64);
                }
            }
        }

        let opts = PutMultipartOptions {
            attributes,
            ..Default::default()
        };
        let upload = self.store.put_multipart_opts(path, opts).await?;
        let mut writer = WriteMultipart::new_with_chunk_size(upload, PART_SIZE);
        let mut total = buffered_len as u64;
        for chunk in buffered {
            writer.put(chunk);
        }

        let streamed: Result<()> = async {
            while let Some(chunk) = data.next().await {
                let chunk = chunk?;
                // Back-pressure: hold the producer while parts are in flight
                writer.wait_for_capacity(MAX_CONCURRENT_PARTS).await?;
                total += chunk.len() as u64;
                writer.put(chunk);
            }
            Ok(())
        }
        .await;

        match streamed {
            Ok(()) => {
                writer.finish().await?;
                Ok(total)
            }
            Err(e) => {
                if let Err(abort_error) = writer.abort().await {
                    tracing::warn!(
                        object_key = %path,
                        error = %abort_error,
                        "Failed to abort multipart upload; a bucket lifecycle rule for \
                         incomplete multipart uploads will remove its parts"
                    );
                }
                Err(e)
            }
        }
    }

    /// Delete objects by key. Missing objects count as deleted.
    async fn delete_objects(&self, keys: Vec<String>) -> Result<()> {
        if keys.is_empty() {
            return Ok(());
        }
        let paths = keys
            .into_iter()
            .map(|key| Path::parse(key).map_err(object_store::Error::from))
            .collect::<Vec<_>>();
        let mut results = self.store.delete_stream(stream::iter(paths).boxed());

        let mut first_error = None;
        while let Some(result) = results.next().await {
            match result {
                Ok(_) | Err(object_store::Error::NotFound { .. }) => {}
                Err(e) => {
                    tracing::warn!(error = %e, "Failed to delete object from S3 storage");
                    first_error.get_or_insert(e);
                }
            }
        }
        match first_error {
            Some(e) => Err(e.into()),
            None => Ok(()),
        }
    }

    /// Best-effort delete for cleanup paths where the caller already has a
    /// result to return.
    async fn delete_objects_logged(&self, keys: Vec<String>, reason: &str) {
        if let Err(e) = self.delete_objects(keys.clone()).await {
            tracing::warn!(
                object_keys = ?keys,
                error = %e,
                reason,
                "Object left in S3 storage after failed cleanup"
            );
        }
    }
}

#[async_trait]
impl WorkflowStorage for S3Storage {
    async fn upload_file(
        &self,
        workflow_id: Uuid,
        activity_key: &str,
        filename: &str,
        content_type: Option<&str>,
        data: Pin<
            Box<dyn Stream<Item = std::result::Result<Bytes, std::io::Error>> + Send + Unpin>,
        >,
    ) -> Result<FileMetadata> {
        let path = self.new_object_path(workflow_id, activity_key, filename);
        let key = path.to_string();
        let size = self.write_object(&path, content_type, data).await?;

        let recorded: Result<(FileMetadata, Option<StoredLocation>)> = async {
            let size = i64::try_from(size).map_err(|_| {
                StorageError::UploadFailed(format!("{filename}: size {size} exceeds i64"))
            })?;
            let mut tx = self.pool.begin().await?;
            let (metadata, previous) = file_rows::replace(
                &mut tx,
                workflow_id,
                activity_key,
                filename,
                StoredLocation::Object(key.clone()),
                size,
                content_type,
            )
            .await?;
            if let Some(StoredLocation::LargeObject(oid)) = previous {
                file_rows::unlink_large_object(&mut tx, oid).await?;
            }
            tx.commit().await?;
            Ok((metadata, previous))
        }
        .await;

        match recorded {
            Ok((metadata, previous)) => {
                if let Some(StoredLocation::Object(old_key)) = previous {
                    self.delete_objects_logged(vec![old_key], "replaced by a new upload")
                        .await;
                }
                Ok(metadata)
            }
            Err(e) => {
                self.delete_objects_logged(vec![key], "metadata insert failed")
                    .await;
                Err(e)
            }
        }
    }

    async fn download_file(
        &self,
        workflow_id: Uuid,
        activity_key: &str,
        filename: &str,
    ) -> Result<ByteStream> {
        match file_rows::location(&self.pool, workflow_id, activity_key, filename).await? {
            Some(StoredLocation::Object(key)) => {
                let path = Path::parse(&key).map_err(object_store::Error::from)?;
                let result = self
                    .store
                    .get_opts(&path, GetOptions::default())
                    .await
                    .map_err(|e| match e {
                        object_store::Error::NotFound { .. } => StorageError::FileNotFound(
                            format!("{filename} (object {key} is missing from the bucket)"),
                        ),
                        other => other.into(),
                    })?;
                Ok(Box::pin(
                    result
                        .into_stream()
                        .map(|chunk| chunk.map_err(std::io::Error::other)),
                ))
            }
            // Written by the postgresql provider before switching to s3
            Some(StoredLocation::LargeObject(oid)) => {
                Ok(PostgresStorage::large_object_stream(self.pool.clone(), oid))
            }
            None => Err(StorageError::FileNotFound(filename.to_string())),
        }
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
        // Remove the row first: a failed object delete then leaves an
        // unreferenced object (storage cost) rather than a row pointing at a
        // missing object (a broken file).
        let mut tx = self.pool.begin().await?;
        let location = file_rows::delete(&mut tx, workflow_id, activity_key, filename).await?;
        if let Some(StoredLocation::LargeObject(oid)) = location {
            file_rows::unlink_large_object(&mut tx, oid).await?;
        }
        tx.commit().await?;

        match location {
            Some(StoredLocation::Object(key)) => self.delete_objects(vec![key]).await,
            _ => Ok(()),
        }
    }

    async fn delete_workflow_files(&self, workflow_id: Uuid) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        let mut keys = Vec::new();
        for location in file_rows::delete_workflow(&mut tx, workflow_id).await? {
            match location {
                StoredLocation::LargeObject(oid) => {
                    file_rows::unlink_large_object(&mut tx, oid).await?;
                }
                StoredLocation::Object(key) => keys.push(key),
            }
        }
        tx.commit().await?;

        self.delete_objects(keys).await
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

        // Logical reference, resolved through this storage (not a bucket URL)
        Ok(FileReference::new(workflow_id, activity_key, filename).to_string("s3"))
    }
}
