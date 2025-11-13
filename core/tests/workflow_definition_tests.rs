use serial_test::serial;
use sqlx::PgPool;
use streamflow_core::workflow::{
    ActivityDefinition, ActivityRelationship, WorkflowDefinition, WorkflowDefinitionRepository,
};

async fn setup_test_db() -> PgPool {
    let database_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://streamflow:streamflow_dev@127.0.0.1:5433/streamflow".to_string()
    });

    let pool = PgPool::connect(&database_url)
        .await
        .expect("Failed to connect to test database");

    // Run migrations
    sqlx::migrate!("../migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    pool
}

async fn clean_test_data(pool: &PgPool) {
    sqlx::query!("TRUNCATE workflow_definitions CASCADE")
        .execute(pool)
        .await
        .expect("Failed to clean test data");
}

#[tokio::test]
#[serial]
async fn test_store_and_get_workflow_definition() {
    let pool = setup_test_db().await;
    clean_test_data(&pool).await;

    let repo = WorkflowDefinitionRepository::new(pool.clone());

    let definition = WorkflowDefinition {
        name: "test_workflow".to_string(),
        activities: vec![ActivityDefinition {
            key: "activity1".to_string(),
            worker: "test".to_string(),
            name: Some("Test Activity".to_string()),
            parameters: None,
            preceding: None,
            following: None,
            settings: None,
        }],
    };

    // Store the definition
    let stored = repo.store(definition.clone()).await.unwrap();

    assert_eq!(stored.name, "test_workflow");
    assert!(!stored.version.is_empty());

    // Get the definition by name and version
    let retrieved = repo
        .get(&stored.name, &stored.version)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(retrieved.name, stored.name);
    assert_eq!(retrieved.version, stored.version);
    assert_eq!(retrieved.name, definition.name);
    assert_eq!(retrieved.activities.len(), 1);

    clean_test_data(&pool).await;
}

#[tokio::test]
#[serial]
async fn test_get_latest_workflow_definition() {
    let pool = setup_test_db().await;
    clean_test_data(&pool).await;

    let repo = WorkflowDefinitionRepository::new(pool.clone());

    let definition1 = WorkflowDefinition {
        name: "versioned_workflow".to_string(),
        activities: vec![ActivityDefinition {
            key: "activity1".to_string(),
            worker: "test".to_string(),
            name: None,
            parameters: None,
            preceding: None,
            following: None,
            settings: None,
        }],
    };

    let definition2 = WorkflowDefinition {
        name: "versioned_workflow".to_string(),
        activities: vec![
            ActivityDefinition {
                key: "activity1".to_string(),
                worker: "test".to_string(),
                name: None,
                parameters: None,
                preceding: None,
                following: None,
                settings: None,
            },
            ActivityDefinition {
                key: "activity2".to_string(),
                worker: "test".to_string(),
                name: None,
                parameters: None,
                preceding: None,
                following: None,
                settings: None,
            },
        ],
    };

    // Store two versions with a delay to ensure different timestamps
    let stored1 = repo.store(definition1).await.unwrap();
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    let stored2 = repo.store(definition2).await.unwrap();

    // Get latest should return version 2
    let latest = repo
        .get_latest("versioned_workflow")
        .await
        .unwrap()
        .unwrap();

    assert_eq!(latest.version, stored2.version);
    assert!(latest.version > stored1.version);
    assert_eq!(latest.activities.len(), 2);

    clean_test_data(&pool).await;
}

#[tokio::test]
#[serial]
async fn test_list_workflow_definitions() {
    let pool = setup_test_db().await;
    clean_test_data(&pool).await;

    let repo = WorkflowDefinitionRepository::new(pool.clone());

    // Store multiple workflow definitions
    let definition1 = WorkflowDefinition {
        name: "workflow_a".to_string(),
        activities: vec![ActivityDefinition {
            key: "activity1".to_string(),
            worker: "test".to_string(),
            name: None,
            parameters: None,
            preceding: None,
            following: None,
            settings: None,
        }],
    };

    let definition2 = WorkflowDefinition {
        name: "workflow_b".to_string(),
        activities: vec![ActivityDefinition {
            key: "activity1".to_string(),
            worker: "test".to_string(),
            name: None,
            parameters: None,
            preceding: None,
            following: None,
            settings: None,
        }],
    };

    repo.store(definition1).await.unwrap();
    repo.store(definition2).await.unwrap();

    // List all definitions
    let definitions = repo.list().await.unwrap();

    assert!(definitions.len() >= 2);
    let names: Vec<String> = definitions.iter().map(|d| d.name.clone()).collect();
    assert!(names.contains(&"workflow_a".to_string()));
    assert!(names.contains(&"workflow_b".to_string()));

    clean_test_data(&pool).await;
}

#[tokio::test]
#[serial]
async fn test_validation_error_on_invalid_definition() {
    let pool = setup_test_db().await;
    clean_test_data(&pool).await;

    let repo = WorkflowDefinitionRepository::new(pool.clone());

    // Create invalid definition (no activities)
    let definition = WorkflowDefinition {
        name: "invalid_workflow".to_string(),
        activities: vec![],
    };

    // Should fail validation
    let result = repo.store(definition).await;
    assert!(result.is_err());

    clean_test_data(&pool).await;
}

#[tokio::test]
#[serial]
async fn test_get_nonexistent_workflow() {
    let pool = setup_test_db().await;
    clean_test_data(&pool).await;

    let repo = WorkflowDefinitionRepository::new(pool.clone());

    let result = repo
        .get("nonexistent", "20240101.120000.000000")
        .await
        .unwrap();
    assert!(result.is_none());

    let result = repo.get_latest("nonexistent").await.unwrap();
    assert!(result.is_none());

    clean_test_data(&pool).await;
}

#[tokio::test]
#[serial]
async fn test_workflow_definition_with_dependencies() {
    let pool = setup_test_db().await;
    clean_test_data(&pool).await;

    let repo = WorkflowDefinitionRepository::new(pool.clone());

    let definition = WorkflowDefinition {
        name: "complex_workflow".to_string(),
        activities: vec![
            ActivityDefinition {
                key: "step1".to_string(),
                worker: "test".to_string(),
                name: None,
                parameters: None,
                preceding: None,
                following: Some(vec![ActivityRelationship {
                    activity_key: "step2".to_string(),
                    conditions: None,
                }]),
                settings: None,
            },
            ActivityDefinition {
                key: "step2".to_string(),
                worker: "test".to_string(),
                name: None,
                parameters: None,
                preceding: Some(vec![ActivityRelationship {
                    activity_key: "step1".to_string(),
                    conditions: None,
                }]),
                following: None,
                settings: None,
            },
        ],
    };

    let stored = repo.store(definition).await.unwrap();

    let retrieved = repo
        .get(&stored.name, &stored.version)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(retrieved.activities.len(), 2);
    assert!(retrieved.activities[0].following.as_ref().unwrap().len() > 0);

    clean_test_data(&pool).await;
}
