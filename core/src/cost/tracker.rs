use rust_decimal::Decimal;
use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

pub struct CostTracker {
    pool: PgPool,
}

#[derive(Debug, Clone)]
pub struct ActivityCostRecord {
    pub workflow_id: Uuid,
    pub activity_key: String,
    pub attempt: u32,
    pub cost_usd: Decimal,
    pub estimated_cost_usd: Option<Decimal>,
    pub prompt_tokens: Option<u32>,
    pub output_tokens: Option<u32>,
    pub total_tokens: Option<u32>,
    pub cached_tokens: Option<u32>,
    /// None for lump-sum and non-LLM cost line items
    pub provider: Option<String>,
    /// None for lump-sum and non-LLM cost line items
    pub model: Option<String>,
    pub activity_budget_limit_usd: Option<Decimal>,
    pub workflow_budget_limit_usd: Option<Decimal>,
    pub budget_exceeded: bool,
    pub budget_action: Option<String>,
    /// Enforcement outcome: `"abort"` (pre-execution abort, zero-cost row) or
    /// `"downgrade"` (fallback chain skipped models for budget reasons before
    /// a cheaper model succeeded). None for ordinary cost rows.
    pub budget_event: Option<String>,
}

#[derive(Debug, Clone)]
pub struct BudgetStatus {
    pub activity_cost: Decimal,
    pub workflow_cost: Decimal,
    pub activity_limit: Option<Decimal>,
    pub workflow_limit: Option<Decimal>,
    pub activity_budget_ok: bool,
    pub workflow_budget_ok: bool,
}

#[derive(Debug, Clone)]
pub struct BudgetCheckResult {
    pub can_execute: bool,
    pub activity_budget_ok: bool,
    pub workflow_budget_ok: bool,
    pub projected_activity_cost: Decimal,
    pub projected_workflow_cost: Decimal,
    pub estimated_cost: Decimal,
}

pub type Result<T> = std::result::Result<T, CostError>;

#[derive(Debug, Error)]
pub enum CostError {
    #[error("Database error: {0}")]
    DatabaseError(#[from] sqlx::Error),

    #[error("Budget exceeded")]
    BudgetExceeded,
}

impl CostTracker {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Record activity cost
    pub async fn record_cost(&self, record: ActivityCostRecord) -> Result<()> {
        sqlx::query!(
            r#"
            INSERT INTO activity_costs
                (workflow_id, activity_key, attempt, cost_usd, estimated_cost_usd,
                 prompt_tokens, output_tokens, total_tokens, cached_tokens,
                 provider, model, activity_budget_limit_usd, workflow_budget_limit_usd,
                 budget_exceeded, budget_action, budget_event)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)
            "#,
            record.workflow_id,
            record.activity_key,
            record.attempt as i32,
            record.cost_usd,
            record.estimated_cost_usd,
            record.prompt_tokens.map(|t| t as i32),
            record.output_tokens.map(|t| t as i32),
            record.total_tokens.map(|t| t as i32),
            record.cached_tokens.map(|t| t as i32),
            record.provider,
            record.model,
            record.activity_budget_limit_usd,
            record.workflow_budget_limit_usd,
            record.budget_exceeded,
            record.budget_action,
            record.budget_event,
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Get current budget status
    pub async fn get_budget_status(
        &self,
        workflow_id: Uuid,
        activity_key: &str,
        activity_limit: Option<Decimal>,
        workflow_limit: Option<Decimal>,
    ) -> Result<BudgetStatus> {
        // Use compile-time verified queries
        let activity_cost = sqlx::query_scalar!(
            "SELECT get_activity_cost($1, $2)",
            workflow_id,
            activity_key
        )
        .fetch_one(&self.pool)
        .await?
        .unwrap_or(Decimal::ZERO);

        let workflow_cost = sqlx::query_scalar!("SELECT get_workflow_cost($1)", workflow_id)
            .fetch_one(&self.pool)
            .await?
            .unwrap_or(Decimal::ZERO);

        let activity_budget_ok = activity_limit.is_none_or(|limit| activity_cost < limit);
        let workflow_budget_ok = workflow_limit.is_none_or(|limit| workflow_cost < limit);

        Ok(BudgetStatus {
            activity_cost,
            workflow_cost,
            activity_limit,
            workflow_limit,
            activity_budget_ok,
            workflow_budget_ok,
        })
    }

    /// Check if activity can execute within budget
    pub async fn check_budget_before_execution(
        &self,
        workflow_id: Uuid,
        activity_key: &str,
        estimated_cost: Decimal,
        activity_limit: Option<Decimal>,
        workflow_limit: Option<Decimal>,
    ) -> Result<BudgetCheckResult> {
        let status = self
            .get_budget_status(workflow_id, activity_key, activity_limit, workflow_limit)
            .await?;

        let projected_activity_cost = status.activity_cost + estimated_cost;
        let projected_workflow_cost = status.workflow_cost + estimated_cost;

        let activity_ok = activity_limit.is_none_or(|limit| projected_activity_cost <= limit);
        let workflow_ok = workflow_limit.is_none_or(|limit| projected_workflow_cost <= limit);

        Ok(BudgetCheckResult {
            can_execute: activity_ok && workflow_ok,
            activity_budget_ok: activity_ok,
            workflow_budget_ok: workflow_ok,
            projected_activity_cost,
            projected_workflow_cost,
            estimated_cost,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    async fn create_test_workflow(pool: &PgPool) -> Uuid {
        let workflow_id = Uuid::now_v7();

        // Create workflow definition first
        sqlx::query!(
            r#"INSERT INTO workflow_definitions (name, activities)
               VALUES ('test_workflow', '[]'::jsonb)"#
        )
        .execute(pool)
        .await
        .expect("Failed to insert workflow definition");

        let definition_id =
            sqlx::query_scalar!("SELECT id FROM workflow_definitions WHERE name = 'test_workflow'")
                .fetch_one(pool)
                .await
                .expect("Failed to get definition ID");

        // Create workflow record
        sqlx::query!(
            r#"
            INSERT INTO workflows (id, definition_name, workflow_definition_id, input, status, activities, state_data)
            VALUES ($1, 'test_workflow', $2, '{}'::jsonb, 'running', '{}'::jsonb, '{}'::jsonb)
            "#,
            workflow_id,
            definition_id
        )
        .execute(pool)
        .await
        .expect("Failed to create test workflow");

        workflow_id
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn test_record_cost_basic(pool: PgPool) {
        let tracker = CostTracker::new(pool.clone());
        let workflow_id = create_test_workflow(&pool).await;

        let record = ActivityCostRecord {
            workflow_id,
            activity_key: "test_activity".to_string(),
            attempt: 1,
            cost_usd: dec!(0.0015),
            estimated_cost_usd: Some(dec!(0.002)),
            prompt_tokens: Some(100),
            output_tokens: Some(50),
            total_tokens: Some(150),
            cached_tokens: None,
            provider: Some("anthropic".to_string()),
            model: Some("claude-3-5-haiku-20241022".to_string()),
            activity_budget_limit_usd: None,
            workflow_budget_limit_usd: None,
            budget_exceeded: false,
            budget_action: None,
            budget_event: None,
        };

        let result = tracker.record_cost(record).await;
        assert!(result.is_ok());
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn test_record_cost_with_caching(pool: PgPool) {
        let tracker = CostTracker::new(pool.clone());
        let workflow_id = create_test_workflow(&pool).await;

        let record = ActivityCostRecord {
            workflow_id,
            activity_key: "test_activity_cached".to_string(),
            attempt: 1,
            cost_usd: dec!(0.0012),
            estimated_cost_usd: Some(dec!(0.002)),
            prompt_tokens: Some(1000),
            output_tokens: Some(500),
            total_tokens: Some(1800),
            cached_tokens: Some(300),
            provider: Some("anthropic".to_string()),
            model: Some("claude-3-5-sonnet-20241022".to_string()),
            activity_budget_limit_usd: Some(dec!(0.50)),
            workflow_budget_limit_usd: Some(dec!(1.00)),
            budget_exceeded: false,
            budget_action: Some("abort".to_string()),
            budget_event: None,
        };

        let result = tracker.record_cost(record).await;
        assert!(result.is_ok());
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn test_get_budget_status_no_costs(pool: PgPool) {
        let tracker = CostTracker::new(pool.clone());
        let workflow_id = create_test_workflow(&pool).await;

        let status = tracker
            .get_budget_status(
                workflow_id,
                "new_activity",
                Some(dec!(0.50)),
                Some(dec!(1.00)),
            )
            .await
            .unwrap();

        assert_eq!(status.activity_cost, Decimal::ZERO);
        assert_eq!(status.workflow_cost, Decimal::ZERO);
        assert_eq!(status.activity_limit, Some(dec!(0.50)));
        assert_eq!(status.workflow_limit, Some(dec!(1.00)));
        assert!(status.activity_budget_ok);
        assert!(status.workflow_budget_ok);
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn test_get_budget_status_with_costs(pool: PgPool) {
        let tracker = CostTracker::new(pool.clone());
        let workflow_id = create_test_workflow(&pool).await;

        // Record two costs for the same activity
        let record1 = ActivityCostRecord {
            workflow_id,
            activity_key: "activity1".to_string(),
            attempt: 1,
            cost_usd: dec!(0.10),
            estimated_cost_usd: None,
            prompt_tokens: Some(1000),
            output_tokens: Some(500),
            total_tokens: Some(1500),
            cached_tokens: None,
            provider: Some("openai".to_string()),
            model: Some("gpt-4o".to_string()),
            activity_budget_limit_usd: None,
            workflow_budget_limit_usd: None,
            budget_exceeded: false,
            budget_action: None,
            budget_event: None,
        };

        let record2 = ActivityCostRecord {
            workflow_id,
            activity_key: "activity1".to_string(),
            attempt: 2,
            cost_usd: dec!(0.15),
            ..record1.clone()
        };

        tracker.record_cost(record1).await.unwrap();
        tracker.record_cost(record2).await.unwrap();

        let status = tracker
            .get_budget_status(workflow_id, "activity1", Some(dec!(0.50)), Some(dec!(1.00)))
            .await
            .unwrap();

        // Activity cost should be sum of both attempts
        assert_eq!(status.activity_cost, dec!(0.25));
        assert!(status.activity_budget_ok);
        assert!(status.workflow_budget_ok);
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn test_get_budget_status_exceeds_activity_budget(pool: PgPool) {
        let tracker = CostTracker::new(pool.clone());
        let workflow_id = create_test_workflow(&pool).await;

        // Record cost that exceeds activity budget
        let record = ActivityCostRecord {
            workflow_id,
            activity_key: "expensive_activity".to_string(),
            attempt: 1,
            cost_usd: dec!(0.60),
            estimated_cost_usd: None,
            prompt_tokens: Some(10000),
            output_tokens: Some(5000),
            total_tokens: Some(15000),
            cached_tokens: None,
            provider: Some("anthropic".to_string()),
            model: Some("claude-3-5-sonnet-20241022".to_string()),
            activity_budget_limit_usd: Some(dec!(0.50)),
            workflow_budget_limit_usd: None,
            budget_exceeded: true,
            budget_action: Some("abort".to_string()),
            budget_event: None,
        };

        tracker.record_cost(record).await.unwrap();

        let status = tracker
            .get_budget_status(
                workflow_id,
                "expensive_activity",
                Some(dec!(0.50)),
                Some(dec!(10.00)),
            )
            .await
            .unwrap();

        assert_eq!(status.activity_cost, dec!(0.60));
        assert!(!status.activity_budget_ok); // Exceeds activity budget
        assert!(status.workflow_budget_ok); // Within workflow budget
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn test_check_budget_before_execution_can_execute(pool: PgPool) {
        let tracker = CostTracker::new(pool.clone());
        let workflow_id = create_test_workflow(&pool).await;

        // No costs yet, should be able to execute
        let result = tracker
            .check_budget_before_execution(
                workflow_id,
                "new_activity",
                dec!(0.10),
                Some(dec!(0.50)),
                Some(dec!(1.00)),
            )
            .await
            .unwrap();

        assert!(result.can_execute);
        assert!(result.activity_budget_ok);
        assert!(result.workflow_budget_ok);
        assert_eq!(result.projected_activity_cost, dec!(0.10));
        assert_eq!(result.projected_workflow_cost, dec!(0.10));
        assert_eq!(result.estimated_cost, dec!(0.10));
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn test_check_budget_before_execution_exceeds_activity_budget(pool: PgPool) {
        let tracker = CostTracker::new(pool.clone());
        let workflow_id = create_test_workflow(&pool).await;

        // Record existing cost
        let record = ActivityCostRecord {
            workflow_id,
            activity_key: "activity1".to_string(),
            attempt: 1,
            cost_usd: dec!(0.40),
            estimated_cost_usd: None,
            prompt_tokens: Some(5000),
            output_tokens: Some(2000),
            total_tokens: Some(7000),
            cached_tokens: None,
            provider: Some("openai".to_string()),
            model: Some("gpt-4o".to_string()),
            activity_budget_limit_usd: Some(dec!(0.50)),
            workflow_budget_limit_usd: None,
            budget_exceeded: false,
            budget_action: None,
            budget_event: None,
        };

        tracker.record_cost(record).await.unwrap();

        // Try to execute with another $0.20 (would exceed $0.50 limit)
        let result = tracker
            .check_budget_before_execution(
                workflow_id,
                "activity1",
                dec!(0.20),
                Some(dec!(0.50)),
                Some(dec!(10.00)),
            )
            .await
            .unwrap();

        assert!(!result.can_execute); // Cannot execute
        assert!(!result.activity_budget_ok); // Exceeds activity budget
        assert!(result.workflow_budget_ok); // Within workflow budget
        assert_eq!(result.projected_activity_cost, dec!(0.60));
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn test_check_budget_before_execution_exceeds_workflow_budget(pool: PgPool) {
        let tracker = CostTracker::new(pool.clone());
        let workflow_id = create_test_workflow(&pool).await;

        // Record existing costs for different activities
        let record1 = ActivityCostRecord {
            workflow_id,
            activity_key: "activity1".to_string(),
            attempt: 1,
            cost_usd: dec!(0.60),
            estimated_cost_usd: None,
            prompt_tokens: Some(10000),
            output_tokens: Some(5000),
            total_tokens: Some(15000),
            cached_tokens: None,
            provider: Some("openai".to_string()),
            model: Some("gpt-4o".to_string()),
            activity_budget_limit_usd: None,
            workflow_budget_limit_usd: Some(dec!(1.00)),
            budget_exceeded: false,
            budget_action: None,
            budget_event: None,
        };

        tracker.record_cost(record1).await.unwrap();

        // Try to execute activity2 with $0.50 (would exceed $1.00 workflow limit)
        let result = tracker
            .check_budget_before_execution(
                workflow_id,
                "activity2",
                dec!(0.50),
                Some(dec!(1.00)),
                Some(dec!(1.00)),
            )
            .await
            .unwrap();

        assert!(!result.can_execute); // Cannot execute
        assert!(result.activity_budget_ok); // Within activity budget
        assert!(!result.workflow_budget_ok); // Exceeds workflow budget
        assert_eq!(result.projected_workflow_cost, dec!(1.10));
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn test_check_budget_before_execution_no_limits(pool: PgPool) {
        let tracker = CostTracker::new(pool.clone());
        let workflow_id = create_test_workflow(&pool).await;

        // No budget limits - should always be able to execute
        let result = tracker
            .check_budget_before_execution(
                workflow_id,
                "unlimited_activity",
                dec!(100.00),
                None,
                None,
            )
            .await
            .unwrap();

        assert!(result.can_execute);
        assert!(result.activity_budget_ok);
        assert!(result.workflow_budget_ok);
    }
}
