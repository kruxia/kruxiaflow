use bytes::Bytes;
use futures::Stream;
use futures::stream::{self, StreamExt};
use kruxiaflow_core::storage::{PostgresStorage, StorageError, WorkflowStorage};
use serial_test::serial;
use sqlx::PgPool;
use std::pin::Pin;
use uuid::Uuid;

/// Helper to create a test stream from static content
fn create_test_stream(
    content: &'static [u8],
) -> Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send + Unpin>> {
    let chunks = vec![Ok(Bytes::from_static(content))];
    Box::pin(stream::iter(chunks))
}

/// Helper to create a test stream from owned content
fn create_owned_stream(
    content: Vec<u8>,
) -> Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send + Unpin>> {
    let chunks = vec![Ok(Bytes::from(content))];
    Box::pin(stream::iter(chunks))
}

/// Helper to create a chunked test stream
fn create_chunked_stream(
    chunks: Vec<Bytes>,
) -> Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send + Unpin>> {
    let ok_chunks: Vec<Result<Bytes, std::io::Error>> = chunks.into_iter().map(Ok).collect();
    Box::pin(stream::iter(ok_chunks))
}

/// Helper to create test database pool
async fn setup_test_pool() -> PgPool {
    let database_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://kruxiaflow:kruxiaflow_dev@127.0.0.1:5432/kruxiaflow".to_string()
    });

    let pool = PgPool::connect(&database_url)
        .await
        .expect("Failed to connect to test database");

    // Run migrations from workspace root
    sqlx::migrate!("../migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    pool
}

/// Helper to clean up test data
async fn cleanup_files(pool: &PgPool, workflow_id: Uuid) {
    // First delete all Large Objects
    let oids: Vec<i32> =
        sqlx::query_scalar("SELECT oid::int4 FROM workflow_files WHERE workflow_id = $1")
            .bind(workflow_id)
            .fetch_all(pool)
            .await
            .expect("Failed to fetch OIDs");

    for oid in oids {
        let _ = sqlx::query("SELECT lo_unlink($1)")
            .bind(oid)
            .execute(pool)
            .await;
    }

    // Then delete metadata
    sqlx::query("DELETE FROM workflow_files WHERE workflow_id = $1")
        .bind(workflow_id)
        .execute(pool)
        .await
        .expect("Failed to cleanup test data");
}

#[tokio::test]
#[serial]
async fn test_upload_small_file() {
    let pool = setup_test_pool().await;
    let storage = PostgresStorage::new(pool.clone());
    let workflow_id = Uuid::now_v7();
    let activity_key = "test_activity";
    let filename = "small.txt";
    let content = b"Hello, World!";

    // Upload file
    let data_stream = create_test_stream(content);

    let metadata = storage
        .upload_file(
            workflow_id,
            activity_key,
            filename,
            Some("text/plain"),
            data_stream,
        )
        .await
        .expect("Failed to upload file");

    assert_eq!(metadata.workflow_id, workflow_id);
    assert_eq!(metadata.activity_key, activity_key);
    assert_eq!(metadata.filename, filename);
    assert_eq!(metadata.size, content.len() as i64);
    assert_eq!(metadata.content_type, Some("text/plain".to_string()));

    // Cleanup
    cleanup_files(&pool, workflow_id).await;
}

#[tokio::test]
#[serial]
async fn test_upload_and_download_file() {
    let pool = setup_test_pool().await;
    let storage = PostgresStorage::new(pool.clone());
    let workflow_id = Uuid::now_v7();
    let activity_key = "test_activity";
    let filename = "test.txt";
    let content = b"Hello, World!";

    // Upload file
    let data_stream = create_test_stream(content);

    storage
        .upload_file(
            workflow_id,
            activity_key,
            filename,
            Some("text/plain"),
            data_stream,
        )
        .await
        .expect("Failed to upload file");

    // Download file
    let mut download_stream = storage
        .download_file(workflow_id, activity_key, filename)
        .await
        .expect("Failed to download file");

    let mut downloaded = Vec::new();
    while let Some(chunk_result) = download_stream.next().await {
        let chunk = chunk_result.expect("Failed to read chunk");
        downloaded.extend_from_slice(&chunk);
    }

    assert_eq!(downloaded, content);

    // Cleanup
    cleanup_files(&pool, workflow_id).await;
}

#[tokio::test]
#[serial]
async fn test_upload_large_file_streaming() {
    let pool = setup_test_pool().await;
    let storage = PostgresStorage::new(pool.clone());
    let workflow_id = Uuid::now_v7();
    let activity_key = "test_activity";
    let filename = "large.bin";

    // Create a large file (1MB) split into chunks
    let chunk_size = 8192;
    let num_chunks = 128; // 1MB total
    let total_size = chunk_size * num_chunks;

    // Generate chunks with different patterns to verify content
    let chunks: Vec<Bytes> = (0..num_chunks)
        .map(|i| {
            let mut chunk = vec![0u8; chunk_size];
            for byte in &mut chunk {
                *byte = (i % 256) as u8;
            }
            Bytes::from(chunk)
        })
        .collect();

    // Upload via stream
    let data_stream = create_chunked_stream(chunks.clone());

    let metadata = storage
        .upload_file(
            workflow_id,
            activity_key,
            filename,
            Some("application/octet-stream"),
            data_stream,
        )
        .await
        .expect("Failed to upload large file");

    assert_eq!(metadata.size, total_size as i64);

    // Download and verify content
    let mut download_stream = storage
        .download_file(workflow_id, activity_key, filename)
        .await
        .expect("Failed to download large file");

    let mut downloaded = Vec::new();
    while let Some(chunk_result) = download_stream.next().await {
        let chunk = chunk_result.expect("Failed to read chunk");
        downloaded.extend_from_slice(&chunk);
    }

    // Verify size
    assert_eq!(downloaded.len(), total_size);

    // Verify content pattern
    for (i, chunk_data) in downloaded.chunks(chunk_size).enumerate() {
        let expected_byte = (i % 256) as u8;
        assert!(
            chunk_data.iter().all(|&b| b == expected_byte),
            "Chunk {} has incorrect pattern",
            i
        );
    }

    // Cleanup
    cleanup_files(&pool, workflow_id).await;
}

#[tokio::test]
#[serial]
async fn test_get_file_metadata() {
    let pool = setup_test_pool().await;
    let storage = PostgresStorage::new(pool.clone());
    let workflow_id = Uuid::now_v7();
    let activity_key = "test_activity";
    let filename = "metadata_test.txt";
    let content = b"Test content";

    // Upload file
    let data_stream = create_test_stream(content);

    storage
        .upload_file(
            workflow_id,
            activity_key,
            filename,
            Some("text/plain"),
            data_stream,
        )
        .await
        .expect("Failed to upload file");

    // Get metadata
    let metadata = storage
        .get_file_metadata(workflow_id, activity_key, filename)
        .await
        .expect("Failed to get metadata");

    assert_eq!(metadata.workflow_id, workflow_id);
    assert_eq!(metadata.activity_key, activity_key);
    assert_eq!(metadata.filename, filename);
    assert_eq!(metadata.size, content.len() as i64);
    assert_eq!(metadata.content_type, Some("text/plain".to_string()));

    // Cleanup
    cleanup_files(&pool, workflow_id).await;
}

#[tokio::test]
#[serial]
async fn test_list_files_for_activity() {
    let pool = setup_test_pool().await;
    let storage = PostgresStorage::new(pool.clone());
    let workflow_id = Uuid::now_v7();
    let activity_key = "test_activity";

    // Upload multiple files
    for i in 0..3 {
        let filename = format!("file_{}.txt", i);
        let content = format!("Content {}", i);
        let data_stream = create_owned_stream(content.into_bytes());

        storage
            .upload_file(
                workflow_id,
                activity_key,
                &filename,
                Some("text/plain"),
                data_stream,
            )
            .await
            .expect("Failed to upload file");
    }

    // List files
    let files = storage
        .list_files(workflow_id, activity_key)
        .await
        .expect("Failed to list files");

    assert_eq!(files.len(), 3);
    for (i, file) in files.iter().enumerate() {
        assert_eq!(file.filename, format!("file_{}.txt", i));
        assert_eq!(file.workflow_id, workflow_id);
        assert_eq!(file.activity_key, activity_key);
    }

    // Cleanup
    cleanup_files(&pool, workflow_id).await;
}

#[tokio::test]
#[serial]
async fn test_list_files_empty() {
    let pool = setup_test_pool().await;
    let storage = PostgresStorage::new(pool.clone());
    let workflow_id = Uuid::now_v7();
    let activity_key = "empty_activity";

    // List files for non-existent activity
    let files = storage
        .list_files(workflow_id, activity_key)
        .await
        .expect("Failed to list files");

    assert_eq!(files.len(), 0);
}

#[tokio::test]
#[serial]
async fn test_delete_file() {
    let pool = setup_test_pool().await;
    let storage = PostgresStorage::new(pool.clone());
    let workflow_id = Uuid::now_v7();
    let activity_key = "test_activity";
    let filename = "delete_me.txt";
    let content = b"Delete this";

    // Upload file
    let data_stream = create_test_stream(content);

    storage
        .upload_file(
            workflow_id,
            activity_key,
            filename,
            Some("text/plain"),
            data_stream,
        )
        .await
        .expect("Failed to upload file");

    // Delete file
    storage
        .delete_file(workflow_id, activity_key, filename)
        .await
        .expect("Failed to delete file");

    // Verify file is gone
    let result = storage
        .get_file_metadata(workflow_id, activity_key, filename)
        .await;

    assert!(matches!(result, Err(StorageError::FileNotFound(_))));

    // Cleanup
    cleanup_files(&pool, workflow_id).await;
}

#[tokio::test]
#[serial]
async fn test_delete_nonexistent_file() {
    let pool = setup_test_pool().await;
    let storage = PostgresStorage::new(pool.clone());
    let workflow_id = Uuid::now_v7();
    let activity_key = "test_activity";
    let filename = "nonexistent.txt";

    // Delete non-existent file (should succeed without error)
    storage
        .delete_file(workflow_id, activity_key, filename)
        .await
        .expect("Failed to delete non-existent file");
}

#[tokio::test]
#[serial]
async fn test_delete_workflow_files() {
    let pool = setup_test_pool().await;
    let storage = PostgresStorage::new(pool.clone());
    let workflow_id = Uuid::now_v7();

    // Upload files for multiple activities
    for i in 0..2 {
        let activity_key = format!("activity_{}", i);
        for j in 0..2 {
            let filename = format!("file_{}.txt", j);
            let content = format!("Content {}-{}", i, j);
            let data_stream = create_owned_stream(content.into_bytes());

            storage
                .upload_file(
                    workflow_id,
                    &activity_key,
                    &filename,
                    Some("text/plain"),
                    data_stream,
                )
                .await
                .expect("Failed to upload file");
        }
    }

    // Verify files exist
    let files = storage
        .list_files(workflow_id, "activity_0")
        .await
        .expect("Failed to list files");
    assert_eq!(files.len(), 2);

    // Delete all workflow files
    storage
        .delete_workflow_files(workflow_id)
        .await
        .expect("Failed to delete workflow files");

    // Verify all files are gone
    for i in 0..2 {
        let activity_key = format!("activity_{}", i);
        let files = storage
            .list_files(workflow_id, &activity_key)
            .await
            .expect("Failed to list files");
        assert_eq!(files.len(), 0);
    }
}

#[tokio::test]
#[serial]
async fn test_duplicate_upload_upsert_behavior() {
    let pool = setup_test_pool().await;
    let storage = PostgresStorage::new(pool.clone());
    let workflow_id = Uuid::now_v7();
    let activity_key = "test_activity";
    let filename = "upsert_test.txt";

    // Upload first version
    let content1 = b"Version 1";
    let data_stream = create_test_stream(content1);

    let metadata1 = storage
        .upload_file(
            workflow_id,
            activity_key,
            filename,
            Some("text/plain"),
            data_stream,
        )
        .await
        .expect("Failed to upload first version");

    assert_eq!(metadata1.size, content1.len() as i64);

    // Upload second version (should replace)
    let content2 = b"Version 2 with more content";
    let data_stream = create_test_stream(content2);

    let metadata2 = storage
        .upload_file(
            workflow_id,
            activity_key,
            filename,
            Some("text/plain"),
            data_stream,
        )
        .await
        .expect("Failed to upload second version");

    assert_eq!(metadata2.size, content2.len() as i64);
    assert!(metadata2.created_at > metadata1.created_at);

    // Download and verify it's the second version
    let mut download_stream = storage
        .download_file(workflow_id, activity_key, filename)
        .await
        .expect("Failed to download file");

    let mut downloaded = Vec::new();
    while let Some(chunk_result) = download_stream.next().await {
        let chunk = chunk_result.expect("Failed to read chunk");
        downloaded.extend_from_slice(&chunk);
    }

    assert_eq!(downloaded, content2);

    // Cleanup
    cleanup_files(&pool, workflow_id).await;
}

#[tokio::test]
#[serial]
async fn test_get_file_reference() {
    let pool = setup_test_pool().await;
    let storage = PostgresStorage::new(pool.clone());
    let workflow_id = Uuid::now_v7();
    let activity_key = "test_activity";
    let filename = "reference_test.txt";
    let content = b"Reference test";

    // Upload file
    let data_stream = create_test_stream(content);

    storage
        .upload_file(
            workflow_id,
            activity_key,
            filename,
            Some("text/plain"),
            data_stream,
        )
        .await
        .expect("Failed to upload file");

    // Get file reference
    let reference = storage
        .get_file_reference(workflow_id, activity_key, filename)
        .await
        .expect("Failed to get file reference");

    let expected = format!("postgres://{}/{}/{}", workflow_id, activity_key, filename);
    assert_eq!(reference, expected);

    // Cleanup
    cleanup_files(&pool, workflow_id).await;
}

#[tokio::test]
#[serial]
async fn test_get_reference_for_nonexistent_file() {
    let pool = setup_test_pool().await;
    let storage = PostgresStorage::new(pool.clone());
    let workflow_id = Uuid::now_v7();
    let activity_key = "test_activity";
    let filename = "nonexistent.txt";

    // Try to get reference for non-existent file
    let result = storage
        .get_file_reference(workflow_id, activity_key, filename)
        .await;

    assert!(matches!(result, Err(StorageError::FileNotFound(_))));
}

#[tokio::test]
#[serial]
async fn test_download_nonexistent_file() {
    let pool = setup_test_pool().await;
    let storage = PostgresStorage::new(pool.clone());
    let workflow_id = Uuid::now_v7();
    let activity_key = "test_activity";
    let filename = "nonexistent.txt";

    // Try to download non-existent file
    let result = storage
        .download_file(workflow_id, activity_key, filename)
        .await;

    assert!(matches!(result, Err(StorageError::FileNotFound(_))));
}
