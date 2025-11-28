use crate::activity_result::ActivityResult;
use crate::registry::ActivityImpl;
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::postgres::{PgPool, PgPoolOptions, PgRow};
use sqlx::{Column, Executor, Postgres, Row};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

// ============================================================================
// Shared Pool Management
// ============================================================================

/// Connection pool cache type - shared across all PostgreSQL activities
pub type PoolCache = Arc<RwLock<HashMap<String, PgPool>>>;

/// Create a new empty pool cache
pub fn new_pool_cache() -> PoolCache {
    Arc::new(RwLock::new(HashMap::new()))
}

/// Pool configuration with defaults from environment variables
#[derive(Debug, Clone)]
struct PoolConfig {
    max_connections: u32,
    min_connections: Option<u32>,
    acquire_timeout_secs: u64,
    max_lifetime_secs: Option<u64>,
    idle_timeout_secs: Option<u64>,
}

impl PoolConfig {
    /// Load pool config from environment variables with sensible defaults
    fn from_env() -> Self {
        Self {
            max_connections: std::env::var("STREAMFLOW_POSTGRES_POOL_MAX_CONNECTIONS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(5),
            min_connections: std::env::var("STREAMFLOW_POSTGRES_POOL_MIN_CONNECTIONS")
                .ok()
                .and_then(|s| s.parse().ok()),
            acquire_timeout_secs: std::env::var("STREAMFLOW_POSTGRES_POOL_ACQUIRE_TIMEOUT_SECS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(30),
            max_lifetime_secs: std::env::var("STREAMFLOW_POSTGRES_POOL_MAX_LIFETIME_SECS")
                .ok()
                .and_then(|s| s.parse().ok()),
            idle_timeout_secs: std::env::var("STREAMFLOW_POSTGRES_POOL_IDLE_TIMEOUT_SECS")
                .ok()
                .and_then(|s| s.parse().ok()),
        }
    }
}

/// Get or create a connection pool for the given database URL
///
/// Uses the shared pool cache to avoid connection overhead.
/// Pools are cached by db_url.
async fn get_or_create_pool(
    cache: &PoolCache,
    db_url: &str,
    config: &PoolConfig,
) -> Result<PgPool> {
    // Check if pool exists in cache
    {
        let cache_read = cache.read().await;
        if let Some(pool) = cache_read.get(db_url) {
            return Ok(pool.clone());
        }
    }

    // Create new pool with system configuration
    let mut options = PgPoolOptions::new()
        .max_connections(config.max_connections)
        .acquire_timeout(Duration::from_secs(config.acquire_timeout_secs));

    if let Some(min_connections) = config.min_connections {
        options = options.min_connections(min_connections);
    }

    if let Some(max_lifetime_secs) = config.max_lifetime_secs {
        options = options.max_lifetime(Duration::from_secs(max_lifetime_secs));
    }

    if let Some(idle_timeout_secs) = config.idle_timeout_secs {
        options = options.idle_timeout(Duration::from_secs(idle_timeout_secs));
    }

    let pool = options
        .connect(db_url)
        .await
        .context("Failed to connect to PostgreSQL database")?;

    // Store in cache
    {
        let mut cache_write = cache.write().await;
        cache_write.insert(db_url.to_string(), pool.clone());
    }

    Ok(pool)
}

// ============================================================================
// Shared Statement Execution
// ============================================================================

/// Result from executing a single statement
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatementResult {
    /// Result rows (for SELECT queries)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rows: Option<Vec<Value>>,

    /// Number of rows affected (for INSERT/UPDATE/DELETE)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rows_affected: Option<u64>,
}

/// Query type enum
enum QueryType {
    Select,
    Insert,
    Update,
    Delete,
}

/// Determine query type from SQL statement
fn determine_query_type(sql: &str) -> QueryType {
    let sql_upper = sql.trim_start().to_uppercase();

    if sql_upper.starts_with("SELECT") || sql_upper.starts_with("WITH") {
        QueryType::Select
    } else if sql_upper.starts_with("INSERT") {
        // Check for RETURNING clause - treat as SELECT to fetch rows
        if sql_upper.contains(" RETURNING ") || sql_upper.ends_with(" RETURNING") {
            QueryType::Select
        } else {
            QueryType::Insert
        }
    } else if sql_upper.starts_with("UPDATE") {
        // Check for RETURNING clause
        if sql_upper.contains(" RETURNING ") || sql_upper.ends_with(" RETURNING") {
            QueryType::Select
        } else {
            QueryType::Update
        }
    } else if sql_upper.starts_with("DELETE") {
        // Check for RETURNING clause
        if sql_upper.contains(" RETURNING ") || sql_upper.ends_with(" RETURNING") {
            QueryType::Select
        } else {
            QueryType::Delete
        }
    } else {
        // Default to DML for unknown queries (CREATE, DROP, etc.)
        QueryType::Insert
    }
}

/// Convert a PostgreSQL row to JSON
fn row_to_json(row: &PgRow) -> Result<Value> {
    let mut map = serde_json::Map::new();

    for column in row.columns() {
        let column_name = column.name();

        // Try to get value as different types
        let value: Value = if let Ok(v) = row.try_get::<String, _>(column_name) {
            Value::String(v)
        } else if let Ok(v) = row.try_get::<i32, _>(column_name) {
            json!(v)
        } else if let Ok(v) = row.try_get::<i64, _>(column_name) {
            json!(v)
        } else if let Ok(v) = row.try_get::<f64, _>(column_name) {
            json!(v)
        } else if let Ok(v) = row.try_get::<bool, _>(column_name) {
            Value::Bool(v)
        } else if let Ok(v) = row.try_get::<Value, _>(column_name) {
            // Try as JSON/JSONB
            v
        } else if let Ok(Some(v)) = row.try_get::<Option<String>, _>(column_name) {
            Value::String(v)
        } else {
            // NULL or unsupported type
            Value::Null
        };

        map.insert(column_name.to_string(), value);
    }

    Ok(Value::Object(map))
}

/// Execute a single statement, generic over executor (pool or transaction)
async fn execute_statement<'e, E>(
    executor: E,
    query_str: &str,
    params: Option<&[Value]>,
) -> Result<StatementResult>
where
    E: Executor<'e, Database = Postgres>,
{
    // Build parameterized query
    let mut query = sqlx::query(query_str);

    // Bind parameters
    if let Some(params) = params {
        for param in params {
            query = match param {
                Value::String(s) => query.bind(s.clone()),
                Value::Number(n) => {
                    if let Some(i) = n.as_i64() {
                        query.bind(i)
                    } else if let Some(f) = n.as_f64() {
                        query.bind(f)
                    } else {
                        return Err(anyhow::anyhow!("Invalid number parameter"));
                    }
                }
                Value::Bool(b) => query.bind(*b),
                Value::Null => query.bind(Option::<String>::None),
                _ => {
                    return Err(anyhow::anyhow!(
                        "Unsupported parameter type: {}",
                        param.to_string()
                    ));
                }
            };
        }
    }

    let query_type = determine_query_type(query_str);

    match query_type {
        QueryType::Select => {
            // Execute SELECT query and fetch results
            let rows = query
                .fetch_all(executor)
                .await
                .context("Failed to execute SELECT query")?;

            // Convert rows to JSON
            let results: Vec<Value> = rows.iter().map(row_to_json).collect::<Result<Vec<_>>>()?;

            Ok(StatementResult {
                rows: Some(results),
                rows_affected: None,
            })
        }
        QueryType::Insert | QueryType::Update | QueryType::Delete => {
            // Execute DML query
            let result = query
                .execute(executor)
                .await
                .context("Failed to execute query")?;

            Ok(StatementResult {
                rows: None,
                rows_affected: Some(result.rows_affected()),
            })
        }
    }
}

// ============================================================================
// PostgreSQL Query Activity
// ============================================================================

/// PostgreSQL query parameters
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostgresQueryParams {
    /// Database connection URL
    pub db_url: String,

    /// SQL query to execute
    pub query: String,

    /// Query parameters (for parameterized queries)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Vec<Value>>,
}

/// PostgreSQL query activity (built-in worker)
///
/// Executes SQL queries with parameterized binding
pub struct PostgresQueryActivity {
    pool_cache: PoolCache,
    pool_config: PoolConfig,
}

impl PostgresQueryActivity {
    /// Create a new PostgresQueryActivity with a shared pool cache
    pub fn new(pool_cache: PoolCache) -> Self {
        Self {
            pool_cache,
            pool_config: PoolConfig::from_env(),
        }
    }
}

#[async_trait]
impl ActivityImpl for PostgresQueryActivity {
    async fn execute(&self, parameters: Value) -> Result<ActivityResult> {
        tracing::debug!(
            "Executing postgres_query activity with parameters: {:?}",
            parameters
        );

        // Parse parameters from JSON
        let params: PostgresQueryParams = serde_json::from_value(parameters)
            .context("Failed to parse PostgreSQL query parameters")?;

        // Get or create pool
        let pool = get_or_create_pool(&self.pool_cache, &params.db_url, &self.pool_config).await?;

        // Execute query using shared function
        let result = execute_statement(&pool, &params.query, params.params.as_deref()).await?;

        // Serialize result to JSON for output
        let output =
            serde_json::to_value(&result).context("Failed to serialize PostgreSQL query result")?;

        tracing::debug!("PostgreSQL query completed");

        Ok(ActivityResult::value("result", output))
    }

    fn name(&self) -> &str {
        "postgres_query"
    }

    fn worker(&self) -> &str {
        "builtin"
    }
}

// ============================================================================
// PostgreSQL Transaction Activity
// ============================================================================

/// Transaction statement with query and parameters
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionStatement {
    /// SQL query to execute
    pub query: String,

    /// Query parameters (for parameterized queries)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Vec<Value>>,
}

/// Transaction isolation level
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum IsolationLevel {
    #[default]
    ReadCommitted,
    RepeatableRead,
    Serializable,
}

/// PostgreSQL transaction parameters
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionParams {
    /// Database connection URL
    pub db_url: String,

    /// Statements to execute in the transaction
    pub statements: Vec<TransactionStatement>,

    /// Transaction isolation level (default: read_committed)
    #[serde(default)]
    pub isolation_level: IsolationLevel,
}

/// PostgreSQL transaction result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionResult {
    /// Whether the transaction succeeded
    pub success: bool,

    /// Number of statements executed
    pub statements_executed: usize,

    /// Results from each statement
    pub results: Vec<StatementResult>,
}

/// PostgreSQL transaction activity (built-in worker)
///
/// Executes multiple SQL statements within an atomic transaction
pub struct PostgresTransactionActivity {
    pool_cache: PoolCache,
    pool_config: PoolConfig,
}

impl PostgresTransactionActivity {
    /// Create a new PostgresTransactionActivity with a shared pool cache
    pub fn new(pool_cache: PoolCache) -> Self {
        Self {
            pool_cache,
            pool_config: PoolConfig::from_env(),
        }
    }
}

#[async_trait]
impl ActivityImpl for PostgresTransactionActivity {
    async fn execute(&self, parameters: Value) -> Result<ActivityResult> {
        tracing::debug!(
            "Executing postgres_transaction activity with parameters: {:?}",
            parameters
        );

        // Parse parameters from JSON
        let params: TransactionParams = serde_json::from_value(parameters)
            .context("Failed to parse PostgreSQL transaction parameters")?;

        // Get or create pool
        let pool = get_or_create_pool(&self.pool_cache, &params.db_url, &self.pool_config).await?;

        // Begin transaction
        let mut tx = pool.begin().await.context("Failed to begin transaction")?;

        // Set isolation level
        let isolation_sql = match params.isolation_level {
            IsolationLevel::ReadCommitted => "SET TRANSACTION ISOLATION LEVEL READ COMMITTED",
            IsolationLevel::RepeatableRead => "SET TRANSACTION ISOLATION LEVEL REPEATABLE READ",
            IsolationLevel::Serializable => "SET TRANSACTION ISOLATION LEVEL SERIALIZABLE",
        };
        sqlx::query(isolation_sql)
            .execute(&mut *tx)
            .await
            .context("Failed to set isolation level")?;

        // Execute statements
        let mut results = Vec::new();

        for (idx, stmt) in params.statements.iter().enumerate() {
            match execute_statement(&mut *tx, &stmt.query, stmt.params.as_deref()).await {
                Ok(result) => results.push(result),
                Err(e) => {
                    // Rollback on error
                    tx.rollback().await.ok(); // Best effort rollback
                    return Err(anyhow::anyhow!(
                        "Statement {} failed: {}. Transaction rolled back.",
                        idx,
                        e
                    ));
                }
            }
        }

        // Commit transaction
        tx.commit().await.context("Failed to commit transaction")?;

        let output = serde_json::to_value(TransactionResult {
            success: true,
            statements_executed: results.len(),
            results,
        })
        .context("Failed to serialize transaction result")?;

        tracing::debug!("PostgreSQL transaction completed");

        Ok(ActivityResult::value("result", output))
    }

    fn name(&self) -> &str {
        "postgres_transaction"
    }

    fn worker(&self) -> &str {
        "builtin"
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // Helper to get test database URL
    fn test_db_url() -> String {
        std::env::var("DATABASE_URL").unwrap_or_else(|_| {
            "postgres://streamflow:streamflow_dev@127.0.0.1:5433/streamflow".to_string()
        })
    }

    // Helper to create a shared pool cache for tests
    fn test_pool_cache() -> PoolCache {
        new_pool_cache()
    }

    // ========================================================================
    // PostgresQueryActivity Tests
    // ========================================================================

    #[tokio::test]
    async fn test_postgres_query_select() {
        let activity = PostgresQueryActivity::new(test_pool_cache());

        let params = json!({
            "db_url": test_db_url(),
            "query": "SELECT 1 as num, 'test' as text, true as flag"
        });

        let result = activity.execute(params).await.unwrap();

        let output_value = result.to_json_value();
        let result_obj = output_value.get("result").unwrap();
        assert!(result_obj.get("rows").is_some());
        let rows = result_obj.get("rows").unwrap().as_array().unwrap();
        assert_eq!(rows.len(), 1);

        let row = &rows[0];
        assert_eq!(row.get("num").unwrap(), 1);
        assert_eq!(row.get("text").unwrap(), "test");
        assert_eq!(row.get("flag").unwrap(), true);
    }

    #[tokio::test]
    async fn test_postgres_query_with_params() {
        let activity = PostgresQueryActivity::new(test_pool_cache());

        let params = json!({
            "db_url": test_db_url(),
            "query": "SELECT $1::text as name, $2::int as age",
            "params": ["Alice", 30]
        });

        let result = activity.execute(params).await.unwrap();

        let output_value = result.to_json_value();
        let result_obj = output_value.get("result").unwrap();
        assert!(result_obj.get("rows").is_some());
        let rows = result_obj.get("rows").unwrap().as_array().unwrap();
        assert_eq!(rows.len(), 1);

        let row = &rows[0];
        assert_eq!(row.get("name").unwrap(), "Alice");
        assert_eq!(row.get("age").unwrap(), 30);
    }

    #[tokio::test]
    async fn test_postgres_query_insert() {
        let pool_cache = test_pool_cache();
        let activity = PostgresQueryActivity::new(pool_cache);

        // Create a test table using a unique name to avoid conflicts
        let table_name = format!("test_users_{}", uuid::Uuid::now_v7().simple());
        let setup = json!({
            "db_url": test_db_url(),
            "query": format!("CREATE TABLE {} (id SERIAL PRIMARY KEY, name TEXT, email TEXT)", table_name)
        });
        activity.execute(setup).await.unwrap();

        // Insert a row
        let params = json!({
            "db_url": test_db_url(),
            "query": format!("INSERT INTO {} (name, email) VALUES ($1, $2)", table_name),
            "params": ["Alice", "alice@example.com"]
        });

        let result = activity.execute(params).await.unwrap();

        let output_value = result.to_json_value();
        let result_obj = output_value.get("result").unwrap();
        assert!(result_obj.get("rows_affected").is_some());
        assert_eq!(result_obj.get("rows_affected").unwrap(), 1);

        // Cleanup
        let cleanup = json!({
            "db_url": test_db_url(),
            "query": format!("DROP TABLE {}", table_name)
        });
        activity.execute(cleanup).await.unwrap();
    }

    #[tokio::test]
    async fn test_postgres_query_update() {
        let pool_cache = test_pool_cache();
        let activity = PostgresQueryActivity::new(pool_cache);

        // Create test table with unique name
        let table_name = format!("test_products_{}", uuid::Uuid::now_v7().simple());
        let setup = json!({
            "db_url": test_db_url(),
            "query": format!("CREATE TABLE {} (id SERIAL PRIMARY KEY, name TEXT, price INT)", table_name)
        });
        activity.execute(setup).await.unwrap();

        // Populate test table
        let populate = json!({
            "db_url": test_db_url(),
            "query": format!("INSERT INTO {} (name, price) VALUES ('Widget', 100), ('Gadget', 200)", table_name)
        });
        activity.execute(populate).await.unwrap();

        // Update rows
        let params = json!({
            "db_url": test_db_url(),
            "query": format!("UPDATE {} SET price = $1 WHERE name = $2", table_name),
            "params": [150, "Widget"]
        });

        let result = activity.execute(params).await.unwrap();

        let output_value = result.to_json_value();
        let result_obj = output_value.get("result").unwrap();
        assert!(result_obj.get("rows_affected").is_some());
        assert_eq!(result_obj.get("rows_affected").unwrap(), 1);

        // Cleanup
        let cleanup = json!({
            "db_url": test_db_url(),
            "query": format!("DROP TABLE {}", table_name)
        });
        activity.execute(cleanup).await.unwrap();
    }

    #[tokio::test]
    async fn test_postgres_pool_reuse() {
        // Use shared pool cache to test reuse
        let pool_cache = test_pool_cache();
        let activity = PostgresQueryActivity::new(pool_cache.clone());

        // First query creates pool
        let params1 = json!({
            "db_url": test_db_url(),
            "query": "SELECT 'first' as result"
        });
        let result1 = activity.execute(params1).await.unwrap();
        let output_value1 = result1.to_json_value();
        let result_obj1 = output_value1.get("result").unwrap();
        assert!(result_obj1.get("rows").is_some());

        // Pool should be cached now
        {
            let cache = pool_cache.read().await;
            assert_eq!(cache.len(), 1);
        }

        // Second query should reuse cached pool
        let params2 = json!({
            "db_url": test_db_url(),
            "query": "SELECT 'second' as result"
        });
        let result2 = activity.execute(params2).await.unwrap();
        let output_value = result2.to_json_value();
        let result_obj = output_value.get("result").unwrap();
        assert!(result_obj.get("rows").is_some());

        let rows = result_obj.get("rows").unwrap().as_array().unwrap();
        assert_eq!(rows[0].get("result").unwrap(), "second");

        // Still only one pool in cache
        {
            let cache = pool_cache.read().await;
            assert_eq!(cache.len(), 1);
        }
    }

    // ========================================================================
    // PostgresTransactionActivity Tests
    // ========================================================================

    #[tokio::test]
    async fn test_postgres_transaction_commit() {
        let pool_cache = test_pool_cache();
        let query_activity = PostgresQueryActivity::new(pool_cache.clone());
        let tx_activity = PostgresTransactionActivity::new(pool_cache);

        // Create test table
        let table_name = format!("test_tx_commit_{}", uuid::Uuid::now_v7().simple());
        let setup = json!({
            "db_url": test_db_url(),
            "query": format!("CREATE TABLE {} (id SERIAL PRIMARY KEY, name TEXT, balance INT)", table_name)
        });
        query_activity.execute(setup).await.unwrap();

        // Execute transaction with multiple statements
        let params = json!({
            "db_url": test_db_url(),
            "statements": [
                {
                    "query": format!("INSERT INTO {} (name, balance) VALUES ($1, $2)", table_name),
                    "params": ["Alice", 1000]
                },
                {
                    "query": format!("INSERT INTO {} (name, balance) VALUES ($1, $2)", table_name),
                    "params": ["Bob", 500]
                },
                {
                    "query": format!("UPDATE {} SET balance = balance - $1 WHERE name = $2", table_name),
                    "params": [100, "Alice"]
                },
                {
                    "query": format!("UPDATE {} SET balance = balance + $1 WHERE name = $2", table_name),
                    "params": [100, "Bob"]
                }
            ]
        });

        let result = tx_activity.execute(params).await.unwrap();
        let output_value = result.to_json_value();
        let result_obj = output_value.get("result").unwrap();

        assert_eq!(result_obj.get("success").unwrap(), true);
        assert_eq!(result_obj.get("statements_executed").unwrap(), 4);

        // Verify final state
        let verify = json!({
            "db_url": test_db_url(),
            "query": format!("SELECT name, balance FROM {} ORDER BY name", table_name)
        });
        let verify_result = query_activity.execute(verify).await.unwrap();
        let verify_output = verify_result.to_json_value();
        let rows = verify_output
            .get("result")
            .unwrap()
            .get("rows")
            .unwrap()
            .as_array()
            .unwrap();

        assert_eq!(rows[0].get("name").unwrap(), "Alice");
        assert_eq!(rows[0].get("balance").unwrap(), 900); // 1000 - 100
        assert_eq!(rows[1].get("name").unwrap(), "Bob");
        assert_eq!(rows[1].get("balance").unwrap(), 600); // 500 + 100

        // Cleanup
        let cleanup = json!({
            "db_url": test_db_url(),
            "query": format!("DROP TABLE {}", table_name)
        });
        query_activity.execute(cleanup).await.unwrap();
    }

    #[tokio::test]
    async fn test_postgres_transaction_rollback() {
        let pool_cache = test_pool_cache();
        let query_activity = PostgresQueryActivity::new(pool_cache.clone());
        let tx_activity = PostgresTransactionActivity::new(pool_cache);

        // Create test table with unique constraint
        let table_name = format!("test_tx_rollback_{}", uuid::Uuid::now_v7().simple());
        let setup = json!({
            "db_url": test_db_url(),
            "query": format!("CREATE TABLE {} (id SERIAL PRIMARY KEY, name TEXT UNIQUE, balance INT)", table_name)
        });
        query_activity.execute(setup).await.unwrap();

        // Insert initial data
        let initial = json!({
            "db_url": test_db_url(),
            "query": format!("INSERT INTO {} (name, balance) VALUES ('Alice', 1000)", table_name)
        });
        query_activity.execute(initial).await.unwrap();

        // Execute transaction that will fail due to duplicate key
        let params = json!({
            "db_url": test_db_url(),
            "statements": [
                {
                    "query": format!("UPDATE {} SET balance = balance - 500 WHERE name = 'Alice'", table_name)
                },
                {
                    // This will fail - duplicate name
                    "query": format!("INSERT INTO {} (name, balance) VALUES ('Alice', 500)", table_name)
                }
            ]
        });

        let result = tx_activity.execute(params).await;
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("Statement 1 failed"));
        assert!(err_msg.contains("rolled back"));

        // Verify Alice's balance was NOT changed (rollback worked)
        let verify = json!({
            "db_url": test_db_url(),
            "query": format!("SELECT balance FROM {} WHERE name = 'Alice'", table_name)
        });
        let verify_result = query_activity.execute(verify).await.unwrap();
        let verify_output = verify_result.to_json_value();
        let rows = verify_output
            .get("result")
            .unwrap()
            .get("rows")
            .unwrap()
            .as_array()
            .unwrap();

        assert_eq!(rows[0].get("balance").unwrap(), 1000); // Unchanged!

        // Cleanup
        let cleanup = json!({
            "db_url": test_db_url(),
            "query": format!("DROP TABLE {}", table_name)
        });
        query_activity.execute(cleanup).await.unwrap();
    }

    #[tokio::test]
    async fn test_postgres_transaction_mixed_queries() {
        let pool_cache = test_pool_cache();
        let query_activity = PostgresQueryActivity::new(pool_cache.clone());
        let tx_activity = PostgresTransactionActivity::new(pool_cache);

        // Create test table
        let table_name = format!("test_tx_mixed_{}", uuid::Uuid::now_v7().simple());
        let setup = json!({
            "db_url": test_db_url(),
            "query": format!("CREATE TABLE {} (id SERIAL PRIMARY KEY, name TEXT, value INT)", table_name)
        });
        query_activity.execute(setup).await.unwrap();

        // Execute transaction with SELECT and DML mixed
        let params = json!({
            "db_url": test_db_url(),
            "statements": [
                {
                    "query": format!("INSERT INTO {} (name, value) VALUES ($1, $2) RETURNING id, name", table_name),
                    "params": ["test", 42]
                },
                {
                    "query": format!("SELECT * FROM {} WHERE name = $1", table_name),
                    "params": ["test"]
                },
                {
                    "query": format!("UPDATE {} SET value = value * 2 WHERE name = $1", table_name),
                    "params": ["test"]
                }
            ]
        });

        let result = tx_activity.execute(params).await.unwrap();
        let output_value = result.to_json_value();
        let result_obj = output_value.get("result").unwrap();

        assert_eq!(result_obj.get("success").unwrap(), true);
        assert_eq!(result_obj.get("statements_executed").unwrap(), 3);

        let results = result_obj.get("results").unwrap().as_array().unwrap();

        // First statement: INSERT RETURNING should have rows
        assert!(results[0].get("rows").is_some());
        let insert_rows = results[0].get("rows").unwrap().as_array().unwrap();
        assert_eq!(insert_rows.len(), 1);
        assert_eq!(insert_rows[0].get("name").unwrap(), "test");

        // Second statement: SELECT should have rows
        assert!(results[1].get("rows").is_some());
        let select_rows = results[1].get("rows").unwrap().as_array().unwrap();
        assert_eq!(select_rows.len(), 1);
        assert_eq!(select_rows[0].get("value").unwrap(), 42);

        // Third statement: UPDATE should have rows_affected
        assert_eq!(results[2].get("rows_affected").unwrap(), 1);

        // Cleanup
        let cleanup = json!({
            "db_url": test_db_url(),
            "query": format!("DROP TABLE {}", table_name)
        });
        query_activity.execute(cleanup).await.unwrap();
    }

    #[tokio::test]
    async fn test_postgres_transaction_isolation_level() {
        let pool_cache = test_pool_cache();
        let tx_activity = PostgresTransactionActivity::new(pool_cache.clone());
        let query_activity = PostgresQueryActivity::new(pool_cache);

        // Create test table
        let table_name = format!("test_tx_isolation_{}", uuid::Uuid::now_v7().simple());
        let setup = json!({
            "db_url": test_db_url(),
            "query": format!("CREATE TABLE {} (id SERIAL PRIMARY KEY, counter INT)", table_name)
        });
        query_activity.execute(setup).await.unwrap();

        // Execute transaction with serializable isolation
        let params = json!({
            "db_url": test_db_url(),
            "isolation_level": "serializable",
            "statements": [
                {
                    "query": format!("INSERT INTO {} (counter) VALUES (1)", table_name)
                }
            ]
        });

        let result = tx_activity.execute(params).await.unwrap();
        let output_value = result.to_json_value();
        let result_obj = output_value.get("result").unwrap();
        assert_eq!(result_obj.get("success").unwrap(), true);

        // Cleanup
        let cleanup = json!({
            "db_url": test_db_url(),
            "query": format!("DROP TABLE {}", table_name)
        });
        query_activity.execute(cleanup).await.unwrap();
    }

    #[tokio::test]
    async fn test_postgres_shared_pool_between_activities() {
        // Both activities should share the same pool
        let pool_cache = test_pool_cache();
        let query_activity = PostgresQueryActivity::new(pool_cache.clone());
        let tx_activity = PostgresTransactionActivity::new(pool_cache.clone());

        // Query activity creates pool
        let params1 = json!({
            "db_url": test_db_url(),
            "query": "SELECT 1 as num"
        });
        query_activity.execute(params1).await.unwrap();

        // Pool should be cached
        {
            let cache = pool_cache.read().await;
            assert_eq!(cache.len(), 1);
        }

        // Transaction activity should reuse the same pool
        let params2 = json!({
            "db_url": test_db_url(),
            "statements": [
                { "query": "SELECT 2 as num" }
            ]
        });
        tx_activity.execute(params2).await.unwrap();

        // Still only one pool (shared)
        {
            let cache = pool_cache.read().await;
            assert_eq!(cache.len(), 1);
        }
    }
}
