use kruxiaflow_core::cost::{ActivityCostRecord, CostCalculator, CostTracker, ModelPricing};
use rust_decimal_macros::dec;
use serial_test::serial;
use sqlx::PgPool;
use uuid::Uuid;

async fn setup_test_db() -> PgPool {
    let database_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://kruxiaflow:kruxiaflow_dev@127.0.0.1:5432/kruxiaflow".to_string()
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
    sqlx::query!(
        "TRUNCATE activity_costs, workflows, workflow_definitions, llm_models, llm_providers CASCADE"
    )
    .execute(pool)
    .await
    .expect("Failed to clean test data");
}

async fn seed_llm_pricing(pool: &PgPool) {
    // Insert providers
    sqlx::query!(
        r#"INSERT INTO llm_providers (name, display_name, supports_completion)
           VALUES
               ('anthropic', 'Anthropic', true),
               ('ollama', 'Ollama', true)
           ON CONFLICT (name) DO NOTHING"#
    )
    .execute(pool)
    .await
    .expect("Failed to insert providers");

    // Insert models with pricing
    sqlx::query!(
        r#"INSERT INTO llm_models (provider, name, display_name, input_price_per_million, output_price_per_million, cached_input_price_per_million, supports_completion)
           VALUES
               ('anthropic', 'claude-3-5-sonnet-20241022', 'Claude 3.5 Sonnet', 3.00, 15.00, 0.30, true),
               ('anthropic', 'claude-3-5-haiku-20241022', 'Claude 3.5 Haiku', 0.80, 4.00, 0.08, true),
               ('ollama', 'llama3.2', 'Llama 3.2', 0.00, 0.00, NULL, true)
           ON CONFLICT (provider, name) DO UPDATE SET
               input_price_per_million = EXCLUDED.input_price_per_million,
               output_price_per_million = EXCLUDED.output_price_per_million,
               cached_input_price_per_million = EXCLUDED.cached_input_price_per_million"#
    )
    .execute(pool)
    .await
    .expect("Failed to insert models");
}

#[tokio::test]
#[serial]
async fn test_cost_calculator_batch_get_pricing() {
    let pool = setup_test_db().await;
    clean_test_data(&pool).await;
    seed_llm_pricing(&pool).await;

    let calculator = CostCalculator::new(pool.clone());

    // Query pricing for multiple models (as tuples of provider, model)
    let models = vec![
        (
            "anthropic".to_string(),
            "claude-3-5-sonnet-20241022".to_string(),
        ),
        (
            "anthropic".to_string(),
            "claude-3-5-haiku-20241022".to_string(),
        ),
        ("ollama".to_string(), "llama3.2".to_string()),
    ];

    let pricing = calculator
        .batch_get_pricing(&models)
        .await
        .expect("Failed to get pricing");

    // Verify all models have pricing
    assert_eq!(pricing.len(), 3);

    // Verify Sonnet pricing (keys now in "provider/model" format)
    let sonnet_key = "anthropic/claude-3-5-sonnet-20241022";
    let sonnet = pricing.get(sonnet_key).unwrap();
    assert_eq!(sonnet.input_price_per_million, dec!(3.00));
    assert_eq!(sonnet.output_price_per_million, dec!(15.00));
    assert_eq!(sonnet.cached_input_price_per_million, Some(dec!(0.30)));

    // Verify Haiku pricing
    let haiku_key = "anthropic/claude-3-5-haiku-20241022";
    let haiku = pricing.get(haiku_key).unwrap();
    assert_eq!(haiku.input_price_per_million, dec!(0.80));
    assert_eq!(haiku.output_price_per_million, dec!(4.00));

    // Verify Ollama pricing (free)
    let ollama_key = "ollama/llama3.2";
    let ollama = pricing.get(ollama_key).unwrap();
    assert_eq!(ollama.input_price_per_million, dec!(0.00));
    assert_eq!(ollama.output_price_per_million, dec!(0.00));
}

#[tokio::test]
#[serial]
async fn test_batch_get_pricing_json_serialization() {
    // Regression test for bug: Model Pricing HashMap Tuple Keys Cannot Be Serialized to JSON
    // See: docs/bugs/2026-01-04-model-pricing-tuple-key-serialization.md
    let pool = setup_test_db().await;
    clean_test_data(&pool).await;
    seed_llm_pricing(&pool).await;

    let calculator = CostCalculator::new(pool.clone());

    let models = vec![
        (
            "anthropic".to_string(),
            "claude-3-5-sonnet-20241022".to_string(),
        ),
        (
            "anthropic".to_string(),
            "claude-3-5-haiku-20241022".to_string(),
        ),
    ];

    let pricing = calculator
        .batch_get_pricing(&models)
        .await
        .expect("Failed to get pricing");

    // This should succeed - verifies HashMap keys are strings not tuples
    let json_result = serde_json::to_value(&pricing);
    assert!(
        json_result.is_ok(),
        "Failed to serialize pricing to JSON: {:?}",
        json_result.err()
    );

    let json_value = json_result.unwrap();
    assert!(json_value.is_object(), "JSON should be an object");

    // Verify the JSON structure contains the expected keys
    let json_obj = json_value.as_object().unwrap();
    assert!(
        json_obj.contains_key("anthropic/claude-3-5-sonnet-20241022"),
        "JSON should contain sonnet pricing"
    );
    assert!(
        json_obj.contains_key("anthropic/claude-3-5-haiku-20241022"),
        "JSON should contain haiku pricing"
    );

    // Verify we can roundtrip the JSON
    let roundtrip: std::collections::HashMap<String, ModelPricing> =
        serde_json::from_value(json_value).expect("Failed to deserialize JSON");
    assert_eq!(roundtrip.len(), 2);
}

#[tokio::test]
#[serial]
async fn test_cost_tracker_record_and_retrieve() {
    let pool = setup_test_db().await;
    clean_test_data(&pool).await;

    let tracker = CostTracker::new(pool.clone());

    // Create a workflow
    let workflow_id = Uuid::now_v7();
    sqlx::query!(
        r#"INSERT INTO workflow_definitions (name, activities)
           VALUES ('test_workflow', '[]'::jsonb)"#
    )
    .execute(&pool)
    .await
    .expect("Failed to insert workflow definition");

    let definition_id =
        sqlx::query_scalar!("SELECT id FROM workflow_definitions WHERE name = 'test_workflow'")
            .fetch_one(&pool)
            .await
            .expect("Failed to get definition ID");

    sqlx::query!(
        r#"INSERT INTO workflows (id, definition_name, workflow_definition_id, input, status, activities, state_data)
           VALUES ($1, 'test_workflow', $2, '{}'::jsonb, 'running', '{}'::jsonb, '{}'::jsonb)"#,
        workflow_id,
        definition_id
    )
    .execute(&pool)
    .await
    .expect("Failed to insert workflow");

    // Record a cost
    let activity_key = "test_activity";

    tracker
        .record_cost(ActivityCostRecord {
            workflow_id,
            activity_key: activity_key.to_string(),
            attempt: 1,
            cost_usd: dec!(0.05),
            estimated_cost_usd: Some(dec!(0.05)),
            prompt_tokens: Some(1000),
            output_tokens: Some(2000),
            total_tokens: Some(3000),
            cached_tokens: Some(500),
            provider: "anthropic".to_string(),
            model: "claude-3-5-sonnet-20241022".to_string(),
            activity_budget_limit_usd: None,
            workflow_budget_limit_usd: None,
            budget_exceeded: false,
            budget_action: None,
        })
        .await
        .expect("Failed to record cost");

    // Retrieve budget status
    let budget_status = tracker
        .get_budget_status(
            workflow_id,
            activity_key,
            Some(dec!(0.10)),
            Some(dec!(0.20)),
        )
        .await
        .expect("Failed to get budget status");

    assert_eq!(budget_status.workflow_cost, dec!(0.05));
    assert_eq!(budget_status.activity_cost, dec!(0.05));
    assert!(budget_status.activity_budget_ok); // Within $0.10 limit
    assert!(budget_status.workflow_budget_ok); // Within $0.20 limit

    // Record another cost for the same activity (simulating retry)
    tracker
        .record_cost(ActivityCostRecord {
            workflow_id,
            activity_key: activity_key.to_string(),
            attempt: 2,
            cost_usd: dec!(0.03),
            estimated_cost_usd: Some(dec!(0.03)),
            prompt_tokens: Some(500),
            output_tokens: Some(1000),
            total_tokens: Some(1500),
            cached_tokens: None,
            provider: "anthropic".to_string(),
            model: "claude-3-5-sonnet-20241022".to_string(),
            activity_budget_limit_usd: None,
            workflow_budget_limit_usd: None,
            budget_exceeded: false,
            budget_action: None,
        })
        .await
        .expect("Failed to record second cost");

    // Verify cumulative costs
    let budget_status = tracker
        .get_budget_status(
            workflow_id,
            activity_key,
            Some(dec!(0.10)),
            Some(dec!(0.20)),
        )
        .await
        .expect("Failed to get updated budget status");

    assert_eq!(budget_status.workflow_cost, dec!(0.08));
    assert_eq!(budget_status.activity_cost, dec!(0.08));
    assert!(budget_status.activity_budget_ok); // Still within $0.10 limit
}

#[tokio::test]
#[serial]
async fn test_budget_check_before_execution() {
    let pool = setup_test_db().await;
    clean_test_data(&pool).await;
    seed_llm_pricing(&pool).await;

    let calculator = CostCalculator::new(pool.clone());
    let tracker = CostTracker::new(pool.clone());

    // Create a workflow with budget
    let workflow_id = Uuid::now_v7();
    sqlx::query!(
        r#"INSERT INTO workflow_definitions (name, activities)
           VALUES ('budget_test_workflow', '[]'::jsonb)"#
    )
    .execute(&pool)
    .await
    .expect("Failed to insert workflow definition");

    let definition_id = sqlx::query_scalar!(
        "SELECT id FROM workflow_definitions WHERE name = 'budget_test_workflow'"
    )
    .fetch_one(&pool)
    .await
    .expect("Failed to get definition ID");

    sqlx::query!(
        r#"INSERT INTO workflows (id, definition_name, workflow_definition_id, input, status, activities, state_data, budget_limit_usd)
           VALUES ($1, 'budget_test_workflow', $2, '{}'::jsonb, 'running', '{}'::jsonb, '{}'::jsonb, 0.10)"#,
        workflow_id,
        definition_id
    )
    .execute(&pool)
    .await
    .expect("Failed to insert workflow");

    // Test 1: Can execute when budget sufficient
    // Estimate cost for Haiku with 1000 input + 1000 output tokens
    let estimated_cost = calculator
        .estimate_llm_cost(
            "anthropic",
            "claude-3-5-haiku-20241022",
            &"x".repeat(4000), // ~1000 tokens
            1000,
        )
        .await
        .expect("Failed to estimate cost");

    let check_result = tracker
        .check_budget_before_execution(
            workflow_id,
            "activity1",
            estimated_cost,
            Some(dec!(0.05)), // activity budget
            Some(dec!(0.10)), // workflow budget
        )
        .await
        .expect("Failed to check budget");

    assert!(check_result.can_execute);
    assert!(check_result.estimated_cost < dec!(0.01)); // Haiku is cheap

    // Record some costs to approach budget limit
    tracker
        .record_cost(ActivityCostRecord {
            workflow_id,
            activity_key: "activity1".to_string(),
            attempt: 1,
            cost_usd: dec!(0.09),
            estimated_cost_usd: Some(dec!(0.09)),
            prompt_tokens: Some(10000),
            output_tokens: Some(10000),
            total_tokens: Some(20000),
            cached_tokens: None,
            provider: "anthropic".to_string(),
            model: "claude-3-5-haiku-20241022".to_string(),
            activity_budget_limit_usd: Some(dec!(0.05)),
            workflow_budget_limit_usd: Some(dec!(0.10)),
            budget_exceeded: false,
            budget_action: None,
        })
        .await
        .expect("Failed to record cost");

    // Test 2: Cannot execute when budget exceeded
    // Estimate cost for expensive Sonnet model
    let estimated_cost = calculator
        .estimate_llm_cost(
            "anthropic",
            "claude-3-5-sonnet-20241022",
            &"x".repeat(4000), // ~1000 tokens
            1000,
        )
        .await
        .expect("Failed to estimate Sonnet cost");

    let check_result = tracker
        .check_budget_before_execution(
            workflow_id,
            "activity2",
            estimated_cost,
            Some(dec!(0.05)),
            Some(dec!(0.10)),
        )
        .await
        .expect("Failed to check budget");

    assert!(!check_result.can_execute); // Should fail - $0.09 used + ~$0.015 estimated > $0.10
}

#[tokio::test]
#[serial]
async fn test_token_estimation() {
    let pool = setup_test_db().await;
    let _calculator = CostCalculator::new(pool.clone());

    // Test Anthropic token estimation (3.5 chars/token, 0.85 words/token)
    let text = "Hello, world! This is a test."; // 6 words, 30 chars
    let tokens = CostCalculator::estimate_tokens("anthropic", text);

    // Should be around 8-9 tokens (average of 30/3.5≈8.5 and 6/0.85≈7)
    assert!((7..=10).contains(&tokens), "Anthropic tokens: {}", tokens);

    // Test OpenAI token estimation (4.0 chars/token, 0.75 words/token)
    let tokens = CostCalculator::estimate_tokens("openai", text);

    // Should be around 7-8 tokens (average of 30/4≈7.5 and 6/0.75=8)
    assert!((7..=9).contains(&tokens), "OpenAI tokens: {}", tokens);
}

#[tokio::test]
#[serial]
async fn test_cost_estimation() {
    let pool = setup_test_db().await;
    clean_test_data(&pool).await;
    seed_llm_pricing(&pool).await;

    let calculator = CostCalculator::new(pool.clone());

    // Estimate cost for Sonnet
    let estimated_cost = calculator
        .estimate_llm_cost(
            "anthropic",
            "claude-3-5-sonnet-20241022",
            "Hello, world!",
            1000, // max_tokens
        )
        .await
        .expect("Failed to estimate cost");

    // Should be small for short input + 1000 output tokens
    // ~10 input tokens * $3/M = $0.00003
    // 1000 output tokens * $15/M = $0.015
    // Total ≈ $0.01503
    assert!(estimated_cost > dec!(0.010) && estimated_cost < dec!(0.020));

    // Estimate cost for Ollama (free)
    let estimated_cost = calculator
        .estimate_llm_cost("ollama", "llama3.2", "Hello, world!", 1000)
        .await
        .expect("Failed to estimate Ollama cost");

    assert_eq!(estimated_cost, dec!(0.00));
}

#[tokio::test]
#[serial]
async fn test_budget_enforcement_with_activity_settings() {
    let pool = setup_test_db().await;
    clean_test_data(&pool).await;
    seed_llm_pricing(&pool).await;

    // Test that activity budget limit takes precedence when lower than workflow limit
    let workflow_id = Uuid::now_v7();

    sqlx::query!(
        r#"INSERT INTO workflow_definitions (name, activities)
           VALUES ('budget_activity_test', '[]'::jsonb)"#
    )
    .execute(&pool)
    .await
    .expect("Failed to insert workflow definition");

    let definition_id = sqlx::query_scalar!(
        "SELECT id FROM workflow_definitions WHERE name = 'budget_activity_test'"
    )
    .fetch_one(&pool)
    .await
    .expect("Failed to get definition ID");

    // Workflow has $10 budget, but activity will have $0.01 budget
    sqlx::query!(
        r#"INSERT INTO workflows (id, definition_name, workflow_definition_id, input, status, activities, state_data, budget_limit_usd)
           VALUES ($1, 'budget_activity_test', $2, '{}'::jsonb, 'running', '{}'::jsonb, '{}'::jsonb, 10.00)"#,
        workflow_id,
        definition_id
    )
    .execute(&pool)
    .await
    .expect("Failed to insert workflow");

    let calculator = CostCalculator::new(pool.clone());
    let tracker = CostTracker::new(pool.clone());

    // Estimate cost for large Sonnet request
    let estimated_cost = calculator
        .estimate_llm_cost(
            "anthropic",
            "claude-3-5-sonnet-20241022",
            &"x".repeat(40000), // ~10000 tokens
            10000,
        )
        .await
        .expect("Failed to estimate cost");

    // Activity budget is lower - should use activity budget
    let check_result = tracker
        .check_budget_before_execution(
            workflow_id,
            "expensive_activity",
            estimated_cost,
            Some(dec!(0.01)),  // Activity budget: $0.01
            Some(dec!(10.00)), // Workflow budget: $10.00
        )
        .await
        .expect("Failed to check budget");

    // Should fail because estimated cost > $0.01 activity budget
    assert!(!check_result.can_execute);
    // The activity budget should be the limiting factor
    assert!(!check_result.activity_budget_ok);
    assert!(check_result.workflow_budget_ok); // Workflow budget is $10, plenty
}
