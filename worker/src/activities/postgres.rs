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
            max_connections: std::env::var("KRUXIAFLOW_POSTGRES_POOL_MAX_CONNECTIONS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(5),
            min_connections: std::env::var("KRUXIAFLOW_POSTGRES_POOL_MIN_CONNECTIONS")
                .ok()
                .and_then(|s| s.parse().ok()),
            acquire_timeout_secs: std::env::var("KRUXIAFLOW_POSTGRES_POOL_ACQUIRE_TIMEOUT_SECS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(30),
            max_lifetime_secs: std::env::var("KRUXIAFLOW_POSTGRES_POOL_MAX_LIFETIME_SECS")
                .ok()
                .and_then(|s| s.parse().ok()),
            idle_timeout_secs: std::env::var("KRUXIAFLOW_POSTGRES_POOL_IDLE_TIMEOUT_SECS")
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

/// Check if SQL contains a RETURNING clause
/// Handles multiline queries where RETURNING may be on its own line
fn has_returning_clause(sql_upper: &str) -> bool {
    // Check for RETURNING followed by whitespace or end of string
    // This handles cases like:
    // - "... RETURNING id"
    // - "...\nRETURNING id"
    // - "... RETURNING *"
    sql_upper.contains(" RETURNING ")
        || sql_upper.contains(" RETURNING\n")
        || sql_upper.contains("\nRETURNING ")
        || sql_upper.contains("\nRETURNING\n")
        || sql_upper.ends_with(" RETURNING")
        || sql_upper.ends_with("\nRETURNING")
}

/// Determine query type from SQL statement
fn determine_query_type(sql: &str) -> QueryType {
    let sql_upper = sql.trim_start().to_uppercase();

    if sql_upper.starts_with("SELECT") || sql_upper.starts_with("WITH") {
        QueryType::Select
    } else if sql_upper.starts_with("INSERT") {
        // Check for RETURNING clause - treat as SELECT to fetch rows
        if has_returning_clause(&sql_upper) {
            QueryType::Select
        } else {
            QueryType::Insert
        }
    } else if sql_upper.starts_with("UPDATE") {
        // Check for RETURNING clause
        if has_returning_clause(&sql_upper) {
            QueryType::Select
        } else {
            QueryType::Update
        }
    } else if sql_upper.starts_with("DELETE") {
        // Check for RETURNING clause
        if has_returning_clause(&sql_upper) {
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
///
/// Handles both nullable and non-nullable columns for all supported types.
/// For nullable columns, sqlx requires using Option<T> even when the value is non-null.
fn row_to_json(row: &PgRow) -> Result<Value> {
    let mut map = serde_json::Map::new();

    for column in row.columns() {
        let column_name = column.name();

        // Try to get value as different types
        // Order matters: try non-nullable first, then nullable variants
        // UUID must be tried before String since UUIDs are often displayed as strings
        let value: Value =
            // UUID (non-nullable, then nullable)
            if let Ok(v) = row.try_get::<uuid::Uuid, _>(column_name) {
                Value::String(v.to_string())
            } else if let Ok(opt) = row.try_get::<Option<uuid::Uuid>, _>(column_name) {
                match opt {
                    Some(v) => Value::String(v.to_string()),
                    None => Value::Null,
                }
            }
            // String (non-nullable, then nullable)
            else if let Ok(v) = row.try_get::<String, _>(column_name) {
                Value::String(v)
            } else if let Ok(opt) = row.try_get::<Option<String>, _>(column_name) {
                match opt {
                    Some(v) => Value::String(v),
                    None => Value::Null,
                }
            }
            // i16 (SMALLINT) - try before i32
            else if let Ok(v) = row.try_get::<i16, _>(column_name) {
                json!(v)
            } else if let Ok(opt) = row.try_get::<Option<i16>, _>(column_name) {
                match opt {
                    Some(v) => json!(v),
                    None => Value::Null,
                }
            }
            // i32 (INTEGER)
            else if let Ok(v) = row.try_get::<i32, _>(column_name) {
                json!(v)
            } else if let Ok(opt) = row.try_get::<Option<i32>, _>(column_name) {
                match opt {
                    Some(v) => json!(v),
                    None => Value::Null,
                }
            }
            // i64 (BIGINT)
            else if let Ok(v) = row.try_get::<i64, _>(column_name) {
                json!(v)
            } else if let Ok(opt) = row.try_get::<Option<i64>, _>(column_name) {
                match opt {
                    Some(v) => json!(v),
                    None => Value::Null,
                }
            }
            // f32 (REAL)
            else if let Ok(v) = row.try_get::<f32, _>(column_name) {
                json!(v)
            } else if let Ok(opt) = row.try_get::<Option<f32>, _>(column_name) {
                match opt {
                    Some(v) => json!(v),
                    None => Value::Null,
                }
            }
            // f64 (DOUBLE PRECISION)
            else if let Ok(v) = row.try_get::<f64, _>(column_name) {
                json!(v)
            } else if let Ok(opt) = row.try_get::<Option<f64>, _>(column_name) {
                match opt {
                    Some(v) => json!(v),
                    None => Value::Null,
                }
            }
            // bool
            else if let Ok(v) = row.try_get::<bool, _>(column_name) {
                Value::Bool(v)
            } else if let Ok(opt) = row.try_get::<Option<bool>, _>(column_name) {
                match opt {
                    Some(v) => Value::Bool(v),
                    None => Value::Null,
                }
            }
            // JSON/JSONB
            else if let Ok(v) = row.try_get::<Value, _>(column_name) {
                v
            } else if let Ok(opt) = row.try_get::<Option<Value>, _>(column_name) {
                opt.unwrap_or(Value::Null)
            }
            // Fallback: NULL or unsupported type
            else {
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
                Value::Array(_) | Value::Object(_) => {
                    // Serialize arrays and objects as JSON strings for PostgreSQL JSONB
                    // Use in SQL with ::jsonb cast, e.g.: SELECT * FROM jsonb_array_elements($1::jsonb)
                    let json_str = serde_json::to_string(param).map_err(|e| {
                        anyhow::anyhow!("Failed to serialize JSON parameter: {}", e)
                    })?;
                    query.bind(json_str)
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
            "postgres://kruxiaflow:kruxiaflow_dev@127.0.0.1:5433/kruxiaflow".to_string()
        })
    }

    // Helper to create a shared pool cache for tests
    fn test_pool_cache() -> PoolCache {
        new_pool_cache()
    }

    // ========================================================================
    // Query Type Detection Tests
    // ========================================================================

    #[test]
    fn test_has_returning_clause() {
        // Note: has_returning_clause expects uppercase input
        // Space before RETURNING
        assert!(has_returning_clause(
            &"INSERT INTO x VALUES (1) RETURNING id".to_uppercase()
        ));

        // Newline before RETURNING (multiline query)
        assert!(has_returning_clause(
            &"INSERT INTO x VALUES (1)\nRETURNING id".to_uppercase()
        ));
        assert!(has_returning_clause(
            &"INSERT INTO x\nVALUES (1)\nRETURNING id".to_uppercase()
        ));

        // RETURNING at end
        assert!(has_returning_clause(
            &"INSERT INTO x VALUES (1) RETURNING".to_uppercase()
        ));
        assert!(has_returning_clause(
            &"INSERT INTO x VALUES (1)\nRETURNING".to_uppercase()
        ));

        // Should NOT match - RETURNING is part of table/column name
        assert!(!has_returning_clause(
            &"INSERT INTO returning_table VALUES (1)".to_uppercase()
        ));
        assert!(!has_returning_clause(
            &"INSERT INTO x_returning VALUES (1)".to_uppercase()
        ));
    }

    #[test]
    fn test_determine_query_type_insert_with_multiline_returning() {
        // This is the exact format that was failing - multiline with RETURNING on its own line
        let sql = r#"INSERT INTO orders
            (customer_id, product_id, quantity)
            VALUES ($1, $2, $3)
            RETURNING id as order_id"#;

        matches!(determine_query_type(sql), QueryType::Select);
    }

    #[test]
    fn test_determine_query_type_basic() {
        // SELECT
        assert!(matches!(
            determine_query_type("SELECT 1"),
            QueryType::Select
        ));
        assert!(matches!(
            determine_query_type("WITH cte AS (SELECT 1) SELECT * FROM cte"),
            QueryType::Select
        ));

        // INSERT without RETURNING
        assert!(matches!(
            determine_query_type("INSERT INTO x VALUES (1)"),
            QueryType::Insert
        ));

        // INSERT with RETURNING
        assert!(matches!(
            determine_query_type("INSERT INTO x VALUES (1) RETURNING id"),
            QueryType::Select
        ));

        // UPDATE without RETURNING
        assert!(matches!(
            determine_query_type("UPDATE x SET y = 1"),
            QueryType::Update
        ));

        // UPDATE with RETURNING
        assert!(matches!(
            determine_query_type("UPDATE x SET y = 1 RETURNING *"),
            QueryType::Select
        ));

        // DELETE without RETURNING
        assert!(matches!(
            determine_query_type("DELETE FROM x WHERE id = 1"),
            QueryType::Delete
        ));

        // DELETE with RETURNING
        assert!(matches!(
            determine_query_type("DELETE FROM x WHERE id = 1 RETURNING id"),
            QueryType::Select
        ));
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

    // ========================================================================
    // Type Handling Tests (Bug fix: null values for UUID/nullable types)
    // See docs/bugs/2026-01-06-postgres-query-output-null-values.md
    // ========================================================================

    #[tokio::test]
    async fn test_postgres_query_uuid_columns() {
        let pool_cache = test_pool_cache();
        let activity = PostgresQueryActivity::new(pool_cache);

        // Create table with UUID column
        let table_name = format!("test_uuid_{}", uuid::Uuid::now_v7().simple());
        let setup = json!({
            "db_url": test_db_url(),
            "query": format!(
                "CREATE TABLE {} (id UUID PRIMARY KEY DEFAULT gen_random_uuid(), name TEXT)",
                table_name
            )
        });
        activity.execute(setup).await.unwrap();

        // Insert a row with explicit UUID (cast string to uuid)
        let test_uuid = uuid::Uuid::now_v7();
        let insert = json!({
            "db_url": test_db_url(),
            "query": format!("INSERT INTO {} (id, name) VALUES ($1::uuid, $2)", table_name),
            "params": [test_uuid.to_string(), "Test User"]
        });
        activity.execute(insert).await.unwrap();

        // Query the UUID column - this is the bug scenario
        let query = json!({
            "db_url": test_db_url(),
            "query": format!("SELECT id, name FROM {}", table_name)
        });
        let result = activity.execute(query).await.unwrap();

        let output_value = result.to_json_value();
        let rows = output_value
            .get("result")
            .unwrap()
            .get("rows")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(rows.len(), 1);

        // UUID should be serialized as string, NOT null
        let id_value = rows[0].get("id").unwrap();
        assert!(
            id_value.is_string(),
            "UUID should be a string, got: {:?}",
            id_value
        );
        assert_eq!(id_value.as_str().unwrap(), test_uuid.to_string());

        // Cleanup
        let cleanup = json!({
            "db_url": test_db_url(),
            "query": format!("DROP TABLE {}", table_name)
        });
        activity.execute(cleanup).await.unwrap();
    }

    #[tokio::test]
    async fn test_postgres_query_nullable_uuid() {
        let pool_cache = test_pool_cache();
        let activity = PostgresQueryActivity::new(pool_cache);

        // Create table with nullable UUID column
        let table_name = format!("test_nullable_uuid_{}", uuid::Uuid::now_v7().simple());
        let setup = json!({
            "db_url": test_db_url(),
            "query": format!(
                "CREATE TABLE {} (id SERIAL PRIMARY KEY, ref_id UUID, name TEXT)",
                table_name
            )
        });
        activity.execute(setup).await.unwrap();

        // Insert rows - one with UUID, one with NULL
        let test_uuid = uuid::Uuid::now_v7();
        let insert1 = json!({
            "db_url": test_db_url(),
            "query": format!("INSERT INTO {} (ref_id, name) VALUES ($1::uuid, $2)", table_name),
            "params": [test_uuid.to_string(), "With UUID"]
        });
        activity.execute(insert1).await.unwrap();

        let insert2 = json!({
            "db_url": test_db_url(),
            "query": format!("INSERT INTO {} (ref_id, name) VALUES (NULL, $1)", table_name),
            "params": ["Without UUID"]
        });
        activity.execute(insert2).await.unwrap();

        // Query both rows
        let query = json!({
            "db_url": test_db_url(),
            "query": format!("SELECT ref_id, name FROM {} ORDER BY id", table_name)
        });
        let result = activity.execute(query).await.unwrap();

        let output_value = result.to_json_value();
        let rows = output_value
            .get("result")
            .unwrap()
            .get("rows")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(rows.len(), 2);

        // First row: UUID should be present as string
        let ref_id1 = rows[0].get("ref_id").unwrap();
        assert!(
            ref_id1.is_string(),
            "ref_id should be a string, got: {:?}",
            ref_id1
        );
        assert_eq!(ref_id1.as_str().unwrap(), test_uuid.to_string());

        // Second row: UUID should be null
        let ref_id2 = rows[1].get("ref_id").unwrap();
        assert!(
            ref_id2.is_null(),
            "ref_id should be null, got: {:?}",
            ref_id2
        );

        // Cleanup
        let cleanup = json!({
            "db_url": test_db_url(),
            "query": format!("DROP TABLE {}", table_name)
        });
        activity.execute(cleanup).await.unwrap();
    }

    #[tokio::test]
    async fn test_postgres_query_nullable_integer() {
        let pool_cache = test_pool_cache();
        let activity = PostgresQueryActivity::new(pool_cache);

        // Create table with nullable integer columns
        let table_name = format!("test_nullable_int_{}", uuid::Uuid::now_v7().simple());
        let setup = json!({
            "db_url": test_db_url(),
            "query": format!(
                "CREATE TABLE {} (id SERIAL PRIMARY KEY, page_start INTEGER, page_end INTEGER)",
                table_name
            )
        });
        activity.execute(setup).await.unwrap();

        // Insert rows with various nullable integer states
        let insert1 = json!({
            "db_url": test_db_url(),
            "query": format!("INSERT INTO {} (page_start, page_end) VALUES (1, 10)", table_name)
        });
        activity.execute(insert1).await.unwrap();

        let insert2 = json!({
            "db_url": test_db_url(),
            "query": format!("INSERT INTO {} (page_start, page_end) VALUES (5, NULL)", table_name)
        });
        activity.execute(insert2).await.unwrap();

        let insert3 = json!({
            "db_url": test_db_url(),
            "query": format!("INSERT INTO {} (page_start, page_end) VALUES (NULL, NULL)", table_name)
        });
        activity.execute(insert3).await.unwrap();

        // Query all rows
        let query = json!({
            "db_url": test_db_url(),
            "query": format!("SELECT id, page_start, page_end FROM {} ORDER BY id", table_name)
        });
        let result = activity.execute(query).await.unwrap();

        let output_value = result.to_json_value();
        let rows = output_value
            .get("result")
            .unwrap()
            .get("rows")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(rows.len(), 3);

        // Row 1: Both integers present
        assert_eq!(rows[0].get("page_start").unwrap(), 1);
        assert_eq!(rows[0].get("page_end").unwrap(), 10);

        // Row 2: page_start present, page_end null
        assert_eq!(rows[1].get("page_start").unwrap(), 5);
        assert!(rows[1].get("page_end").unwrap().is_null());

        // Row 3: Both null
        assert!(rows[2].get("page_start").unwrap().is_null());
        assert!(rows[2].get("page_end").unwrap().is_null());

        // Cleanup
        let cleanup = json!({
            "db_url": test_db_url(),
            "query": format!("DROP TABLE {}", table_name)
        });
        activity.execute(cleanup).await.unwrap();
    }

    #[tokio::test]
    async fn test_postgres_query_mixed_types() {
        let pool_cache = test_pool_cache();
        let activity = PostgresQueryActivity::new(pool_cache);

        // Create table with mixed column types (reproducing the exact bug scenario)
        let table_name = format!("test_mixed_types_{}", uuid::Uuid::now_v7().simple());
        let setup = json!({
            "db_url": test_db_url(),
            "query": format!(
                r#"CREATE TABLE {} (
                    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
                    source_id UUID,
                    page_start INTEGER,
                    page_end INTEGER,
                    score REAL,
                    title TEXT,
                    active BOOLEAN
                )"#,
                table_name
            )
        });
        activity.execute(setup).await.unwrap();

        // Insert test data (cast string params to uuid)
        let test_id = uuid::Uuid::now_v7();
        let test_source_id = uuid::Uuid::now_v7();
        let insert = json!({
            "db_url": test_db_url(),
            "query": format!(
                "INSERT INTO {} (id, source_id, page_start, page_end, score, title, active) VALUES ($1::uuid, $2::uuid, $3, $4, $5, $6, $7)",
                table_name
            ),
            "params": [test_id.to_string(), test_source_id.to_string(), 42, 100, 0.95, "Test Document", true]
        });
        activity.execute(insert).await.unwrap();

        // Query - this is the exact pattern that was failing
        let query = json!({
            "db_url": test_db_url(),
            "query": format!("SELECT id, source_id, page_start, page_end, score, title, active FROM {}", table_name)
        });
        let result = activity.execute(query).await.unwrap();

        let output_value = result.to_json_value();
        let rows = output_value
            .get("result")
            .unwrap()
            .get("rows")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(rows.len(), 1);

        let row = &rows[0];

        // UUID columns should be strings, not null
        let id_val = row.get("id").unwrap();
        assert!(id_val.is_string(), "id should be string, got: {:?}", id_val);
        assert_eq!(id_val.as_str().unwrap(), test_id.to_string());

        let source_id_val = row.get("source_id").unwrap();
        assert!(
            source_id_val.is_string(),
            "source_id should be string, got: {:?}",
            source_id_val
        );
        assert_eq!(source_id_val.as_str().unwrap(), test_source_id.to_string());

        // Integer columns should be numbers, not null
        let page_start = row.get("page_start").unwrap();
        assert!(
            page_start.is_number(),
            "page_start should be number, got: {:?}",
            page_start
        );
        assert_eq!(page_start, 42);

        let page_end = row.get("page_end").unwrap();
        assert!(
            page_end.is_number(),
            "page_end should be number, got: {:?}",
            page_end
        );
        assert_eq!(page_end, 100);

        // Float column
        let score = row.get("score").unwrap();
        assert!(
            score.is_number(),
            "score should be number, got: {:?}",
            score
        );

        // String column
        let title = row.get("title").unwrap();
        assert!(
            title.is_string(),
            "title should be string, got: {:?}",
            title
        );
        assert_eq!(title.as_str().unwrap(), "Test Document");

        // Boolean column
        let active = row.get("active").unwrap();
        assert!(
            active.is_boolean(),
            "active should be boolean, got: {:?}",
            active
        );
        assert_eq!(active, true);

        // Cleanup
        let cleanup = json!({
            "db_url": test_db_url(),
            "query": format!("DROP TABLE {}", table_name)
        });
        activity.execute(cleanup).await.unwrap();
    }

    #[tokio::test]
    async fn test_postgres_query_smallint_bigint() {
        let pool_cache = test_pool_cache();
        let activity = PostgresQueryActivity::new(pool_cache);

        // Create table with various integer sizes
        let table_name = format!("test_int_sizes_{}", uuid::Uuid::now_v7().simple());
        let setup = json!({
            "db_url": test_db_url(),
            "query": format!(
                "CREATE TABLE {} (id SERIAL PRIMARY KEY, small_val SMALLINT, big_val BIGINT)",
                table_name
            )
        });
        activity.execute(setup).await.unwrap();

        // Insert test data
        let insert = json!({
            "db_url": test_db_url(),
            "query": format!("INSERT INTO {} (small_val, big_val) VALUES ($1, $2)", table_name),
            "params": [32767, 9223372036854775807_i64]
        });
        activity.execute(insert).await.unwrap();

        // Query
        let query = json!({
            "db_url": test_db_url(),
            "query": format!("SELECT small_val, big_val FROM {}", table_name)
        });
        let result = activity.execute(query).await.unwrap();

        let output_value = result.to_json_value();
        let rows = output_value
            .get("result")
            .unwrap()
            .get("rows")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(rows.len(), 1);

        // SMALLINT should work
        let small_val = rows[0].get("small_val").unwrap();
        assert!(
            small_val.is_number(),
            "small_val should be number, got: {:?}",
            small_val
        );
        assert_eq!(small_val, 32767);

        // BIGINT should work
        let big_val = rows[0].get("big_val").unwrap();
        assert!(
            big_val.is_number(),
            "big_val should be number, got: {:?}",
            big_val
        );
        assert_eq!(big_val.as_i64().unwrap(), 9223372036854775807_i64);

        // Cleanup
        let cleanup = json!({
            "db_url": test_db_url(),
            "query": format!("DROP TABLE {}", table_name)
        });
        activity.execute(cleanup).await.unwrap();
    }

    // ========================================================================
    // Array/Object Parameter Tests (Bug fix: unsupported parameter type)
    // See docs/bugs/2026-01-05-postgres-array-params-unsupported.md
    // ========================================================================

    #[tokio::test]
    async fn test_postgres_query_array_parameter() {
        let pool_cache = test_pool_cache();
        let activity = PostgresQueryActivity::new(pool_cache);

        // Use jsonb_array_elements to extract array elements - this is the bug scenario
        let query = json!({
            "db_url": test_db_url(),
            "query": "SELECT value::text FROM jsonb_array_elements($1::jsonb) AS value ORDER BY value",
            "params": [[1, 2, 3, 4, 5]]
        });

        let result = activity.execute(query).await.unwrap();

        let output_value = result.to_json_value();
        let rows = output_value
            .get("result")
            .unwrap()
            .get("rows")
            .unwrap()
            .as_array()
            .unwrap();

        // Should have 5 rows, one for each array element
        assert_eq!(rows.len(), 5);
        assert_eq!(rows[0].get("value").unwrap(), "1");
        assert_eq!(rows[4].get("value").unwrap(), "5");
    }

    #[tokio::test]
    async fn test_postgres_query_object_parameter() {
        let pool_cache = test_pool_cache();
        let activity = PostgresQueryActivity::new(pool_cache);

        // Create table with JSONB column
        let table_name = format!("test_jsonb_{}", uuid::Uuid::now_v7().simple());
        let setup = json!({
            "db_url": test_db_url(),
            "query": format!("CREATE TABLE {} (id SERIAL PRIMARY KEY, data JSONB)", table_name)
        });
        activity.execute(setup).await.unwrap();

        // Insert an object parameter
        let test_object = serde_json::json!({
            "name": "Test User",
            "age": 30,
            "active": true,
            "tags": ["admin", "user"]
        });
        let insert = json!({
            "db_url": test_db_url(),
            "query": format!("INSERT INTO {} (data) VALUES ($1::jsonb) RETURNING id, data", table_name),
            "params": [test_object.clone()]
        });
        let result = activity.execute(insert).await.unwrap();

        let output_value = result.to_json_value();
        let rows = output_value
            .get("result")
            .unwrap()
            .get("rows")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(rows.len(), 1);

        // Query the inserted data
        let query = json!({
            "db_url": test_db_url(),
            "query": format!("SELECT data->>'name' as name, (data->>'age')::int as age FROM {}", table_name)
        });
        let result = activity.execute(query).await.unwrap();

        let output_value = result.to_json_value();
        let rows = output_value
            .get("result")
            .unwrap()
            .get("rows")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(rows[0].get("name").unwrap(), "Test User");
        assert_eq!(rows[0].get("age").unwrap(), 30);

        // Cleanup
        let cleanup = json!({
            "db_url": test_db_url(),
            "query": format!("DROP TABLE {}", table_name)
        });
        activity.execute(cleanup).await.unwrap();
    }

    #[tokio::test]
    async fn test_postgres_query_nested_json_parameter() {
        let pool_cache = test_pool_cache();
        let activity = PostgresQueryActivity::new(pool_cache);

        // Test with deeply nested structure
        let nested_data = serde_json::json!({
            "level1": {
                "level2": {
                    "level3": {
                        "values": [1, 2, 3],
                        "message": "deep nested"
                    }
                }
            },
            "array_of_objects": [
                {"id": 1, "name": "first"},
                {"id": 2, "name": "second"}
            ]
        });

        // Query nested values using PostgreSQL JSON operators
        let query = json!({
            "db_url": test_db_url(),
            "query": "SELECT $1::jsonb->'level1'->'level2'->'level3'->>'message' as message",
            "params": [nested_data]
        });

        let result = activity.execute(query).await.unwrap();

        let output_value = result.to_json_value();
        let rows = output_value
            .get("result")
            .unwrap()
            .get("rows")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(rows[0].get("message").unwrap(), "deep nested");
    }

    #[tokio::test]
    async fn test_postgres_query_json_special_characters() {
        let pool_cache = test_pool_cache();
        let activity = PostgresQueryActivity::new(pool_cache);

        // Test JSON with special characters (quotes, newlines, unicode)
        let special_data = serde_json::json!({
            "quoted": "He said \"hello\"",
            "newlines": "line1\nline2\nline3",
            "unicode": "日本語 emoji: 🎉",
            "backslash": "path\\to\\file",
            "tab": "col1\tcol2"
        });

        let query = json!({
            "db_url": test_db_url(),
            "query": "SELECT $1::jsonb->>'quoted' as quoted, $1::jsonb->>'unicode' as unicode",
            "params": [special_data]
        });

        let result = activity.execute(query).await.unwrap();

        let output_value = result.to_json_value();
        let rows = output_value
            .get("result")
            .unwrap()
            .get("rows")
            .unwrap()
            .as_array()
            .unwrap();

        assert_eq!(rows[0].get("quoted").unwrap(), "He said \"hello\"");
        assert_eq!(rows[0].get("unicode").unwrap(), "日本語 emoji: 🎉");
    }

    #[tokio::test]
    async fn test_postgres_query_array_of_objects_parameter() {
        let pool_cache = test_pool_cache();
        let activity = PostgresQueryActivity::new(pool_cache);

        // This is the exact pattern from the bug report - array of objects
        let passages = serde_json::json!([
            {"sequence": 1, "content": "First passage", "page": 1},
            {"sequence": 2, "content": "Second passage", "page": 2},
            {"sequence": 3, "content": "Third passage", "page": 3}
        ]);

        // Extract each object and get specific fields
        let query = json!({
            "db_url": test_db_url(),
            "query": "SELECT (elem->>'sequence')::int as seq, elem->>'content' as content FROM jsonb_array_elements($1::jsonb) AS elem ORDER BY (elem->>'sequence')::int",
            "params": [passages]
        });

        let result = activity.execute(query).await.unwrap();

        let output_value = result.to_json_value();
        let rows = output_value
            .get("result")
            .unwrap()
            .get("rows")
            .unwrap()
            .as_array()
            .unwrap();

        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].get("seq").unwrap(), 1);
        assert_eq!(rows[0].get("content").unwrap(), "First passage");
        assert_eq!(rows[2].get("seq").unwrap(), 3);
        assert_eq!(rows[2].get("content").unwrap(), "Third passage");
    }

    #[tokio::test]
    async fn test_postgres_transaction_json_parameters() {
        let pool_cache = test_pool_cache();
        let query_activity = PostgresQueryActivity::new(pool_cache.clone());
        let tx_activity = PostgresTransactionActivity::new(pool_cache);

        // Create table for transaction test
        let table_name = format!("test_tx_jsonb_{}", uuid::Uuid::now_v7().simple());
        let setup = json!({
            "db_url": test_db_url(),
            "query": format!("CREATE TABLE {} (id SERIAL PRIMARY KEY, data JSONB)", table_name)
        });
        query_activity.execute(setup).await.unwrap();

        // Execute transaction with JSON parameters
        let obj1 = serde_json::json!({"name": "Item 1", "value": 100});
        let obj2 = serde_json::json!({"name": "Item 2", "value": 200});

        let params = json!({
            "db_url": test_db_url(),
            "statements": [
                {
                    "query": format!("INSERT INTO {} (data) VALUES ($1::jsonb)", table_name),
                    "params": [obj1]
                },
                {
                    "query": format!("INSERT INTO {} (data) VALUES ($1::jsonb)", table_name),
                    "params": [obj2]
                },
                {
                    "query": format!("SELECT data->>'name' as name FROM {} ORDER BY id", table_name)
                }
            ]
        });

        let result = tx_activity.execute(params).await.unwrap();
        let output_value = result.to_json_value();
        let result_obj = output_value.get("result").unwrap();

        assert_eq!(result_obj.get("success").unwrap(), true);
        assert_eq!(result_obj.get("statements_executed").unwrap(), 3);

        // Check the SELECT result from the third statement
        let results = result_obj.get("results").unwrap().as_array().unwrap();
        let select_rows = results[2].get("rows").unwrap().as_array().unwrap();
        assert_eq!(select_rows.len(), 2);
        assert_eq!(select_rows[0].get("name").unwrap(), "Item 1");
        assert_eq!(select_rows[1].get("name").unwrap(), "Item 2");

        // Cleanup
        let cleanup = json!({
            "db_url": test_db_url(),
            "query": format!("DROP TABLE {}", table_name)
        });
        query_activity.execute(cleanup).await.unwrap();
    }
}
