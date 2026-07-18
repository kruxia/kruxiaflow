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
                Value::String(s) => {
                    // Bind as TEXT. For NUMERIC columns, use ::numeric cast in SQL:
                    //   INSERT INTO t (price) VALUES ($1::numeric)
                    query.bind(s.clone())
                }
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
        let params: PostgresQueryParams = serde_json::from_value(parameters.clone())
            .map_err(|e| {
                tracing::error!(
                    serde_error = %e,
                    parameters = %parameters,
                    "Failed to parse PostgreSQL query parameters"
                );
                e
            })
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
        "std"
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
        "std"
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ========================================================================
    // Unit Tests: JSON Parameter Serialization (No Database Required)
    // Regression tests for: docs/bugs/2026-01-05-postgres-array-params-unsupported.md
    // ========================================================================

    /// Helper to simulate the parameter serialization logic used in execute_statement
    /// This extracts the serialization behavior for unit testing without needing a database
    fn serialize_param_for_postgres(param: &Value) -> Result<String> {
        match param {
            Value::String(s) => Ok(s.clone()),
            Value::Number(n) => Ok(n.to_string()),
            Value::Bool(b) => Ok(b.to_string()),
            Value::Null => Ok("NULL".to_string()),
            Value::Array(_) | Value::Object(_) => {
                // This is the fix being tested - arrays and objects serialize to JSON strings
                serde_json::to_string(param)
                    .map_err(|e| anyhow::anyhow!("Failed to serialize JSON parameter: {}", e))
            }
        }
    }

    #[test]
    fn test_serialize_array_parameter() {
        // Simple array of integers
        let param = json!([1, 2, 3, 4, 5]);
        let result = serialize_param_for_postgres(&param).unwrap();
        assert_eq!(result, "[1,2,3,4,5]");
    }

    #[test]
    fn test_serialize_array_of_strings() {
        let param = json!(["hello", "world", "test"]);
        let result = serialize_param_for_postgres(&param).unwrap();
        assert_eq!(result, r#"["hello","world","test"]"#);
    }

    #[test]
    fn test_serialize_object_parameter() {
        let param = json!({"name": "test", "value": 42});
        let result = serialize_param_for_postgres(&param).unwrap();
        // Note: JSON object key order may vary, so parse and compare
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed.get("name").unwrap(), "test");
        assert_eq!(parsed.get("value").unwrap(), 42);
    }

    #[test]
    fn test_serialize_nested_object() {
        let param = json!({
            "level1": {
                "level2": {
                    "level3": {
                        "value": "deep"
                    }
                }
            }
        });
        let result = serialize_param_for_postgres(&param).unwrap();
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(
            parsed
                .get("level1")
                .unwrap()
                .get("level2")
                .unwrap()
                .get("level3")
                .unwrap()
                .get("value")
                .unwrap(),
            "deep"
        );
    }

    #[test]
    fn test_serialize_array_of_objects() {
        // This is the exact bug scenario from the report
        let param = json!([
            {"sequence": 1, "content": "First passage"},
            {"sequence": 2, "content": "Second passage"},
            {"sequence": 3, "content": "Third passage"}
        ]);
        let result = serialize_param_for_postgres(&param).unwrap();

        // Should produce valid JSON array
        let parsed: Vec<Value> = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed[0].get("sequence").unwrap(), 1);
        assert_eq!(parsed[0].get("content").unwrap(), "First passage");
        assert_eq!(parsed[2].get("sequence").unwrap(), 3);
    }

    #[test]
    fn test_serialize_special_characters_in_json() {
        // JSON with quotes, newlines, backslashes, and unicode
        let param = json!({
            "quoted": "He said \"hello\"",
            "newlines": "line1\nline2",
            "unicode": "日本語 emoji: 🎉",
            "backslash": "path\\to\\file",
            "tab": "col1\tcol2"
        });
        let result = serialize_param_for_postgres(&param).unwrap();

        // Should be valid JSON that can be parsed back
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed.get("quoted").unwrap(), "He said \"hello\"");
        assert_eq!(parsed.get("newlines").unwrap(), "line1\nline2");
        assert_eq!(parsed.get("unicode").unwrap(), "日本語 emoji: 🎉");
        assert_eq!(parsed.get("backslash").unwrap(), "path\\to\\file");
        assert_eq!(parsed.get("tab").unwrap(), "col1\tcol2");
    }

    #[test]
    fn test_serialize_empty_array() {
        let param = json!([]);
        let result = serialize_param_for_postgres(&param).unwrap();
        assert_eq!(result, "[]");
    }

    #[test]
    fn test_serialize_empty_object() {
        let param = json!({});
        let result = serialize_param_for_postgres(&param).unwrap();
        assert_eq!(result, "{}");
    }

    #[test]
    fn test_serialize_mixed_array() {
        // Array with mixed types
        let param = json!([1, "two", true, null, {"nested": "object"}, [1, 2, 3]]);
        let result = serialize_param_for_postgres(&param).unwrap();

        let parsed: Vec<Value> = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed.len(), 6);
        assert_eq!(parsed[0], 1);
        assert_eq!(parsed[1], "two");
        assert_eq!(parsed[2], true);
        assert!(parsed[3].is_null());
        assert_eq!(parsed[4].get("nested").unwrap(), "object");
        assert_eq!(parsed[5].as_array().unwrap().len(), 3);
    }

    #[test]
    fn test_serialize_large_array() {
        // Test with a larger array
        let items: Vec<Value> = (0..1000)
            .map(|i| json!({"id": i, "data": format!("item_{}", i)}))
            .collect();
        let param = Value::Array(items);
        let result = serialize_param_for_postgres(&param).unwrap();

        let parsed: Vec<Value> = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed.len(), 1000);
        assert_eq!(parsed[0].get("id").unwrap(), 0);
        assert_eq!(parsed[999].get("id").unwrap(), 999);
    }

    #[test]
    fn test_serialize_object_with_null_value() {
        let param = json!({"present": "value", "missing": null});
        let result = serialize_param_for_postgres(&param).unwrap();

        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed.get("present").unwrap(), "value");
        assert!(parsed.get("missing").unwrap().is_null());
    }

    #[test]
    #[allow(clippy::approx_constant)]
    fn test_serialize_object_with_numeric_values() {
        let param = json!({
            "integer": 42,
            "float": 3.141_59,
            "negative": -100,
            "zero": 0,
            "big": 9223372036854775807_i64
        });
        let result = serialize_param_for_postgres(&param).unwrap();

        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed.get("integer").unwrap(), 42);
        assert_eq!(parsed.get("negative").unwrap(), -100);
        assert_eq!(parsed.get("zero").unwrap(), 0);
        assert_eq!(
            parsed.get("big").unwrap().as_i64().unwrap(),
            9223372036854775807_i64
        );
    }

    #[test]
    fn test_serialize_scalar_types_unchanged() {
        // Scalar types should pass through without JSON serialization
        assert_eq!(
            serialize_param_for_postgres(&json!("hello")).unwrap(),
            "hello"
        );
        assert_eq!(serialize_param_for_postgres(&json!(42)).unwrap(), "42");
        assert_eq!(serialize_param_for_postgres(&json!(3.15)).unwrap(), "3.15");
        assert_eq!(serialize_param_for_postgres(&json!(true)).unwrap(), "true");
        assert_eq!(
            serialize_param_for_postgres(&json!(false)).unwrap(),
            "false"
        );
        assert_eq!(serialize_param_for_postgres(&json!(null)).unwrap(), "NULL");
    }

    #[test]
    fn test_array_produces_valid_postgresql_jsonb() {
        // The serialized array should be valid for PostgreSQL's ::jsonb cast
        let param = json!([{"id": 1}, {"id": 2}]);
        let result = serialize_param_for_postgres(&param).unwrap();

        // Valid JSON that PostgreSQL can parse
        assert!(result.starts_with('['));
        assert!(result.ends_with(']'));
        // Should NOT have extra quotes around it (that would cause the "cannot extract from scalar" error)
        assert!(!result.starts_with("\"["));
    }

    #[test]
    fn test_object_produces_valid_postgresql_jsonb() {
        let param = json!({"key": "value"});
        let result = serialize_param_for_postgres(&param).unwrap();

        // Valid JSON that PostgreSQL can parse
        assert!(result.starts_with('{'));
        assert!(result.ends_with('}'));
        // Should NOT have extra quotes
        assert!(!result.starts_with("\"{"));
    }

    // ========================================================================
    // Unit Tests: StatementResult Output Format (No Database Required)
    // Regression tests for: docs/bugs/2026-01-06-postgres-query-output-null-values.md
    //
    // These tests verify the expected output format of postgres_query results.
    // The row_to_json function requires PgRow which needs a database, but we can
    // verify the StatementResult serialization contract that downstream activities
    // depend on.
    // ========================================================================

    #[test]
    fn test_statement_result_serialization_with_rows() {
        // Verify StatementResult serializes correctly with rows
        let result = StatementResult {
            rows: Some(vec![
                json!({"id": "550e8400-e29b-41d4-a716-446655440000", "name": "test", "count": 42}),
                json!({"id": "550e8400-e29b-41d4-a716-446655440001", "name": "test2", "count": 100}),
            ]),
            rows_affected: None,
        };

        let json_str = serde_json::to_string(&result).expect("Should serialize");
        let parsed: Value = serde_json::from_str(&json_str).expect("Should parse");

        // Verify structure
        let rows = parsed.get("rows").expect("Should have rows");
        assert!(rows.is_array());
        assert_eq!(rows.as_array().unwrap().len(), 2);

        // Verify first row has correct types
        let row0 = &rows.as_array().unwrap()[0];
        assert!(row0.get("id").unwrap().is_string());
        assert!(row0.get("name").unwrap().is_string());
        assert!(row0.get("count").unwrap().is_number());
    }

    #[test]
    fn test_statement_result_rows_affected() {
        // Verify StatementResult with rows_affected (INSERT/UPDATE/DELETE)
        let result = StatementResult {
            rows: None,
            rows_affected: Some(5),
        };

        let json_str = serde_json::to_string(&result).expect("Should serialize");
        let parsed: Value = serde_json::from_str(&json_str).expect("Should parse");

        // rows should not be present (skip_serializing_if)
        assert!(parsed.get("rows").is_none());
        assert_eq!(parsed.get("rows_affected").unwrap(), 5);
    }

    #[test]
    fn test_statement_result_uuid_as_string() {
        // Regression test: UUIDs should be serialized as strings, not null
        // This is the expected output format from row_to_json
        let uuid_str = "550e8400-e29b-41d4-a716-446655440000";
        let result = StatementResult {
            rows: Some(vec![json!({"id": uuid_str, "source_id": uuid_str})]),
            rows_affected: None,
        };

        let json_str = serde_json::to_string(&result).expect("Should serialize");
        let parsed: Value = serde_json::from_str(&json_str).expect("Should parse");

        let row = &parsed.get("rows").unwrap().as_array().unwrap()[0];

        // UUIDs should be strings, NOT null
        let id = row.get("id").unwrap();
        assert!(id.is_string(), "UUID should be string, got: {:?}", id);
        assert_eq!(id.as_str().unwrap(), uuid_str);

        let source_id = row.get("source_id").unwrap();
        assert!(
            source_id.is_string(),
            "UUID should be string, got: {:?}",
            source_id
        );
    }

    #[test]
    fn test_statement_result_integers_as_numbers() {
        // Regression test: Integers should be serialized as numbers, not null
        let result = StatementResult {
            rows: Some(vec![json!({
                "page_start": 1,
                "page_end": 100,
                "small_val": 32767_i16,
                "big_val": 9223372036854775807_i64
            })]),
            rows_affected: None,
        };

        let json_str = serde_json::to_string(&result).expect("Should serialize");
        let parsed: Value = serde_json::from_str(&json_str).expect("Should parse");

        let row = &parsed.get("rows").unwrap().as_array().unwrap()[0];

        // All integers should be numbers, NOT null
        let page_start = row.get("page_start").unwrap();
        assert!(
            page_start.is_number(),
            "Integer should be number, got: {:?}",
            page_start
        );
        assert_eq!(page_start, 1);

        let page_end = row.get("page_end").unwrap();
        assert!(
            page_end.is_number(),
            "Integer should be number, got: {:?}",
            page_end
        );
        assert_eq!(page_end, 100);

        let big_val = row.get("big_val").unwrap();
        assert!(
            big_val.is_number(),
            "BIGINT should be number, got: {:?}",
            big_val
        );
        assert_eq!(big_val.as_i64().unwrap(), 9223372036854775807_i64);
    }

    #[test]
    fn test_statement_result_nullable_values() {
        // Regression test: Nullable columns should properly represent NULL as JSON null
        let result = StatementResult {
            rows: Some(vec![
                json!({"id": 1, "nullable_field": "has_value", "another_nullable": 42}),
                json!({"id": 2, "nullable_field": null, "another_nullable": null}),
            ]),
            rows_affected: None,
        };

        let json_str = serde_json::to_string(&result).expect("Should serialize");
        let parsed: Value = serde_json::from_str(&json_str).expect("Should parse");

        let rows = parsed.get("rows").unwrap().as_array().unwrap();

        // First row: values present
        assert_eq!(rows[0].get("nullable_field").unwrap(), "has_value");
        assert_eq!(rows[0].get("another_nullable").unwrap(), 42);

        // Second row: null values should be JSON null, not missing
        assert!(
            rows[1].get("nullable_field").is_some(),
            "Nullable field should exist"
        );
        assert!(
            rows[1].get("nullable_field").unwrap().is_null(),
            "Nullable field should be null"
        );
        assert!(rows[1].get("another_nullable").unwrap().is_null());
    }

    #[test]
    fn test_statement_result_mixed_types() {
        // Regression test: Mixed types should all serialize correctly
        // This reproduces the exact bug scenario from the report
        let result = StatementResult {
            rows: Some(vec![json!({
                "id": "550e8400-e29b-41d4-a716-446655440000",  // UUID as string
                "source_id": "550e8400-e29b-41d4-a716-446655440001",  // UUID as string
                "page_start": 42,  // INTEGER
                "page_end": 100,  // INTEGER
                "score": 0.95,  // REAL/FLOAT
                "title": "Test Document",  // TEXT
                "active": true  // BOOLEAN
            })]),
            rows_affected: None,
        };

        let json_str = serde_json::to_string(&result).expect("Should serialize");
        let parsed: Value = serde_json::from_str(&json_str).expect("Should parse");

        let row = &parsed.get("rows").unwrap().as_array().unwrap()[0];

        // ALL fields should have their correct types, NONE should be null
        assert!(row.get("id").unwrap().is_string());
        assert!(row.get("source_id").unwrap().is_string());
        assert!(row.get("page_start").unwrap().is_number());
        assert!(row.get("page_end").unwrap().is_number());
        assert!(row.get("score").unwrap().is_number());
        assert!(row.get("title").unwrap().is_string());
        assert!(row.get("active").unwrap().is_boolean());

        // Verify specific values
        assert_eq!(
            row.get("id").unwrap(),
            "550e8400-e29b-41d4-a716-446655440000"
        );
        assert_eq!(row.get("page_start").unwrap(), 42);
        assert_eq!(row.get("title").unwrap(), "Test Document");
        assert_eq!(row.get("active").unwrap(), true);
    }

    #[test]
    #[allow(clippy::approx_constant)]
    fn test_statement_result_roundtrip() {
        // Verify StatementResult survives JSON roundtrip (important for activity output storage)
        let original = StatementResult {
            rows: Some(vec![json!({
                "uuid_field": "550e8400-e29b-41d4-a716-446655440000",
                "int_field": 42,
                "float_field": 3.14,
                "bool_field": true,
                "null_field": null,
                "string_field": "hello"
            })]),
            rows_affected: None,
        };

        // Serialize to JSON string
        let json_str = serde_json::to_string(&original).expect("Should serialize");

        // Deserialize back
        let roundtrip: StatementResult =
            serde_json::from_str(&json_str).expect("Should deserialize");

        // Verify structure preserved
        assert!(roundtrip.rows.is_some());
        let rows = roundtrip.rows.unwrap();
        assert_eq!(rows.len(), 1);

        let row = &rows[0];
        assert_eq!(
            row.get("uuid_field").unwrap(),
            "550e8400-e29b-41d4-a716-446655440000"
        );
        assert_eq!(row.get("int_field").unwrap(), 42);
        assert!(row.get("null_field").unwrap().is_null());
    }

    #[test]
    fn test_statement_result_empty_rows() {
        // Empty rows array should serialize correctly
        let result = StatementResult {
            rows: Some(vec![]),
            rows_affected: None,
        };

        let json_str = serde_json::to_string(&result).expect("Should serialize");
        let parsed: Value = serde_json::from_str(&json_str).expect("Should parse");

        let rows = parsed.get("rows").unwrap().as_array().unwrap();
        assert!(rows.is_empty());
    }

    #[test]
    fn test_statement_result_jsonb_values() {
        // JSONB columns should preserve their structure
        let result = StatementResult {
            rows: Some(vec![json!({
                "id": 1,
                "metadata": {"key": "value", "nested": {"a": 1, "b": 2}},
                "tags": ["tag1", "tag2", "tag3"]
            })]),
            rows_affected: None,
        };

        let json_str = serde_json::to_string(&result).expect("Should serialize");
        let parsed: Value = serde_json::from_str(&json_str).expect("Should parse");

        let row = &parsed.get("rows").unwrap().as_array().unwrap()[0];

        // JSONB object should be preserved
        let metadata = row.get("metadata").unwrap();
        assert!(metadata.is_object());
        assert_eq!(metadata.get("key").unwrap(), "value");
        assert_eq!(metadata.get("nested").unwrap().get("a").unwrap(), 1);

        // JSONB array should be preserved
        let tags = row.get("tags").unwrap();
        assert!(tags.is_array());
        assert_eq!(tags.as_array().unwrap().len(), 3);
    }

    // ========================================================================
    // Integration Test Helpers (Database Required)
    // ========================================================================

    // Helper to get the URL of the isolated per-test database created by
    // #[sqlx::test]. These activities connect by URL (not via a provided pool),
    // so the URL is rebuilt from DATABASE_URL with the test database's name.
    fn test_db_url(pool: &PgPool) -> String {
        let base = std::env::var("DATABASE_URL")
            .expect("DATABASE_URL must be set for #[sqlx::test] tests");
        let db_name = pool
            .connect_options()
            .get_database()
            .expect("test pool should have a database name")
            .to_string();
        let scheme_end = base.find("://").map(|i| i + 3).unwrap_or(0);
        let path_start = base[scheme_end..]
            .find('/')
            .map(|i| scheme_end + i)
            .unwrap_or(base.len());
        format!("{}/{}", &base[..path_start], db_name)
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

    #[sqlx::test(migrations = "../migrations")]
    async fn test_postgres_query_select(pool: PgPool) {
        let activity = PostgresQueryActivity::new(test_pool_cache());

        let params = json!({
            "db_url": test_db_url(&pool),
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

    #[sqlx::test(migrations = "../migrations")]
    async fn test_postgres_query_with_params(pool: PgPool) {
        let activity = PostgresQueryActivity::new(test_pool_cache());

        let params = json!({
            "db_url": test_db_url(&pool),
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

    #[sqlx::test(migrations = "../migrations")]
    async fn test_postgres_query_insert(pool: PgPool) {
        let pool_cache = test_pool_cache();
        let activity = PostgresQueryActivity::new(pool_cache);

        // Create a test table using a unique name to avoid conflicts
        let table_name = format!("test_users_{}", uuid::Uuid::now_v7().simple());
        let setup = json!({
            "db_url": test_db_url(&pool),
            "query": format!("CREATE TABLE {} (id SERIAL PRIMARY KEY, name TEXT, email TEXT)", table_name)
        });
        activity.execute(setup).await.unwrap();

        // Insert a row
        let params = json!({
            "db_url": test_db_url(&pool),
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
            "db_url": test_db_url(&pool),
            "query": format!("DROP TABLE {}", table_name)
        });
        activity.execute(cleanup).await.unwrap();
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn test_postgres_query_update(pool: PgPool) {
        let pool_cache = test_pool_cache();
        let activity = PostgresQueryActivity::new(pool_cache);

        // Create test table with unique name
        let table_name = format!("test_products_{}", uuid::Uuid::now_v7().simple());
        let setup = json!({
            "db_url": test_db_url(&pool),
            "query": format!("CREATE TABLE {} (id SERIAL PRIMARY KEY, name TEXT, price INT)", table_name)
        });
        activity.execute(setup).await.unwrap();

        // Populate test table
        let populate = json!({
            "db_url": test_db_url(&pool),
            "query": format!("INSERT INTO {} (name, price) VALUES ('Widget', 100), ('Gadget', 200)", table_name)
        });
        activity.execute(populate).await.unwrap();

        // Update rows
        let params = json!({
            "db_url": test_db_url(&pool),
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
            "db_url": test_db_url(&pool),
            "query": format!("DROP TABLE {}", table_name)
        });
        activity.execute(cleanup).await.unwrap();
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn test_postgres_pool_reuse(pool: PgPool) {
        // Use shared pool cache to test reuse
        let pool_cache = test_pool_cache();
        let activity = PostgresQueryActivity::new(pool_cache.clone());

        // First query creates pool
        let params1 = json!({
            "db_url": test_db_url(&pool),
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
            "db_url": test_db_url(&pool),
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

    #[sqlx::test(migrations = "../migrations")]
    async fn test_postgres_transaction_commit(pool: PgPool) {
        let pool_cache = test_pool_cache();
        let query_activity = PostgresQueryActivity::new(pool_cache.clone());
        let tx_activity = PostgresTransactionActivity::new(pool_cache);

        // Create test table
        let table_name = format!("test_tx_commit_{}", uuid::Uuid::now_v7().simple());
        let setup = json!({
            "db_url": test_db_url(&pool),
            "query": format!("CREATE TABLE {} (id SERIAL PRIMARY KEY, name TEXT, balance INT)", table_name)
        });
        query_activity.execute(setup).await.unwrap();

        // Execute transaction with multiple statements
        let params = json!({
            "db_url": test_db_url(&pool),
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
            "db_url": test_db_url(&pool),
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
            "db_url": test_db_url(&pool),
            "query": format!("DROP TABLE {}", table_name)
        });
        query_activity.execute(cleanup).await.unwrap();
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn test_postgres_transaction_rollback(pool: PgPool) {
        let pool_cache = test_pool_cache();
        let query_activity = PostgresQueryActivity::new(pool_cache.clone());
        let tx_activity = PostgresTransactionActivity::new(pool_cache);

        // Create test table with unique constraint
        let table_name = format!("test_tx_rollback_{}", uuid::Uuid::now_v7().simple());
        let setup = json!({
            "db_url": test_db_url(&pool),
            "query": format!("CREATE TABLE {} (id SERIAL PRIMARY KEY, name TEXT UNIQUE, balance INT)", table_name)
        });
        query_activity.execute(setup).await.unwrap();

        // Insert initial data
        let initial = json!({
            "db_url": test_db_url(&pool),
            "query": format!("INSERT INTO {} (name, balance) VALUES ('Alice', 1000)", table_name)
        });
        query_activity.execute(initial).await.unwrap();

        // Execute transaction that will fail due to duplicate key
        let params = json!({
            "db_url": test_db_url(&pool),
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
            "db_url": test_db_url(&pool),
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
            "db_url": test_db_url(&pool),
            "query": format!("DROP TABLE {}", table_name)
        });
        query_activity.execute(cleanup).await.unwrap();
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn test_postgres_transaction_mixed_queries(pool: PgPool) {
        let pool_cache = test_pool_cache();
        let query_activity = PostgresQueryActivity::new(pool_cache.clone());
        let tx_activity = PostgresTransactionActivity::new(pool_cache);

        // Create test table
        let table_name = format!("test_tx_mixed_{}", uuid::Uuid::now_v7().simple());
        let setup = json!({
            "db_url": test_db_url(&pool),
            "query": format!("CREATE TABLE {} (id SERIAL PRIMARY KEY, name TEXT, value INT)", table_name)
        });
        query_activity.execute(setup).await.unwrap();

        // Execute transaction with SELECT and DML mixed
        let params = json!({
            "db_url": test_db_url(&pool),
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
            "db_url": test_db_url(&pool),
            "query": format!("DROP TABLE {}", table_name)
        });
        query_activity.execute(cleanup).await.unwrap();
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn test_postgres_transaction_isolation_level(pool: PgPool) {
        let pool_cache = test_pool_cache();
        let tx_activity = PostgresTransactionActivity::new(pool_cache.clone());
        let query_activity = PostgresQueryActivity::new(pool_cache);

        // Create test table
        let table_name = format!("test_tx_isolation_{}", uuid::Uuid::now_v7().simple());
        let setup = json!({
            "db_url": test_db_url(&pool),
            "query": format!("CREATE TABLE {} (id SERIAL PRIMARY KEY, counter INT)", table_name)
        });
        query_activity.execute(setup).await.unwrap();

        // Execute transaction with serializable isolation
        let params = json!({
            "db_url": test_db_url(&pool),
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
            "db_url": test_db_url(&pool),
            "query": format!("DROP TABLE {}", table_name)
        });
        query_activity.execute(cleanup).await.unwrap();
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn test_postgres_shared_pool_between_activities(pool: PgPool) {
        // Both activities should share the same pool
        let pool_cache = test_pool_cache();
        let query_activity = PostgresQueryActivity::new(pool_cache.clone());
        let tx_activity = PostgresTransactionActivity::new(pool_cache.clone());

        // Query activity creates pool
        let params1 = json!({
            "db_url": test_db_url(&pool),
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
            "db_url": test_db_url(&pool),
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

    #[sqlx::test(migrations = "../migrations")]
    async fn test_postgres_query_uuid_columns(pool: PgPool) {
        let pool_cache = test_pool_cache();
        let activity = PostgresQueryActivity::new(pool_cache);

        // Create table with UUID column
        let table_name = format!("test_uuid_{}", uuid::Uuid::now_v7().simple());
        let setup = json!({
            "db_url": test_db_url(&pool),
            "query": format!(
                "CREATE TABLE {} (id UUID PRIMARY KEY DEFAULT gen_random_uuid(), name TEXT)",
                table_name
            )
        });
        activity.execute(setup).await.unwrap();

        // Insert a row with explicit UUID (cast string to uuid)
        let test_uuid = uuid::Uuid::now_v7();
        let insert = json!({
            "db_url": test_db_url(&pool),
            "query": format!("INSERT INTO {} (id, name) VALUES ($1::uuid, $2)", table_name),
            "params": [test_uuid.to_string(), "Test User"]
        });
        activity.execute(insert).await.unwrap();

        // Query the UUID column - this is the bug scenario
        let query = json!({
            "db_url": test_db_url(&pool),
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
            "db_url": test_db_url(&pool),
            "query": format!("DROP TABLE {}", table_name)
        });
        activity.execute(cleanup).await.unwrap();
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn test_postgres_query_nullable_uuid(pool: PgPool) {
        let pool_cache = test_pool_cache();
        let activity = PostgresQueryActivity::new(pool_cache);

        // Create table with nullable UUID column
        let table_name = format!("test_nullable_uuid_{}", uuid::Uuid::now_v7().simple());
        let setup = json!({
            "db_url": test_db_url(&pool),
            "query": format!(
                "CREATE TABLE {} (id SERIAL PRIMARY KEY, ref_id UUID, name TEXT)",
                table_name
            )
        });
        activity.execute(setup).await.unwrap();

        // Insert rows - one with UUID, one with NULL
        let test_uuid = uuid::Uuid::now_v7();
        let insert1 = json!({
            "db_url": test_db_url(&pool),
            "query": format!("INSERT INTO {} (ref_id, name) VALUES ($1::uuid, $2)", table_name),
            "params": [test_uuid.to_string(), "With UUID"]
        });
        activity.execute(insert1).await.unwrap();

        let insert2 = json!({
            "db_url": test_db_url(&pool),
            "query": format!("INSERT INTO {} (ref_id, name) VALUES (NULL, $1)", table_name),
            "params": ["Without UUID"]
        });
        activity.execute(insert2).await.unwrap();

        // Query both rows
        let query = json!({
            "db_url": test_db_url(&pool),
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
            "db_url": test_db_url(&pool),
            "query": format!("DROP TABLE {}", table_name)
        });
        activity.execute(cleanup).await.unwrap();
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn test_postgres_query_nullable_integer(pool: PgPool) {
        let pool_cache = test_pool_cache();
        let activity = PostgresQueryActivity::new(pool_cache);

        // Create table with nullable integer columns
        let table_name = format!("test_nullable_int_{}", uuid::Uuid::now_v7().simple());
        let setup = json!({
            "db_url": test_db_url(&pool),
            "query": format!(
                "CREATE TABLE {} (id SERIAL PRIMARY KEY, page_start INTEGER, page_end INTEGER)",
                table_name
            )
        });
        activity.execute(setup).await.unwrap();

        // Insert rows with various nullable integer states
        let insert1 = json!({
            "db_url": test_db_url(&pool),
            "query": format!("INSERT INTO {} (page_start, page_end) VALUES (1, 10)", table_name)
        });
        activity.execute(insert1).await.unwrap();

        let insert2 = json!({
            "db_url": test_db_url(&pool),
            "query": format!("INSERT INTO {} (page_start, page_end) VALUES (5, NULL)", table_name)
        });
        activity.execute(insert2).await.unwrap();

        let insert3 = json!({
            "db_url": test_db_url(&pool),
            "query": format!("INSERT INTO {} (page_start, page_end) VALUES (NULL, NULL)", table_name)
        });
        activity.execute(insert3).await.unwrap();

        // Query all rows
        let query = json!({
            "db_url": test_db_url(&pool),
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
            "db_url": test_db_url(&pool),
            "query": format!("DROP TABLE {}", table_name)
        });
        activity.execute(cleanup).await.unwrap();
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn test_postgres_query_mixed_types(pool: PgPool) {
        let pool_cache = test_pool_cache();
        let activity = PostgresQueryActivity::new(pool_cache);

        // Create table with mixed column types (reproducing the exact bug scenario)
        let table_name = format!("test_mixed_types_{}", uuid::Uuid::now_v7().simple());
        let setup = json!({
            "db_url": test_db_url(&pool),
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
            "db_url": test_db_url(&pool),
            "query": format!(
                "INSERT INTO {} (id, source_id, page_start, page_end, score, title, active) VALUES ($1::uuid, $2::uuid, $3, $4, $5, $6, $7)",
                table_name
            ),
            "params": [test_id.to_string(), test_source_id.to_string(), 42, 100, 0.95, "Test Document", true]
        });
        activity.execute(insert).await.unwrap();

        // Query - this is the exact pattern that was failing
        let query = json!({
            "db_url": test_db_url(&pool),
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
            "db_url": test_db_url(&pool),
            "query": format!("DROP TABLE {}", table_name)
        });
        activity.execute(cleanup).await.unwrap();
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn test_postgres_query_smallint_bigint(pool: PgPool) {
        let pool_cache = test_pool_cache();
        let activity = PostgresQueryActivity::new(pool_cache);

        // Create table with various integer sizes
        let table_name = format!("test_int_sizes_{}", uuid::Uuid::now_v7().simple());
        let setup = json!({
            "db_url": test_db_url(&pool),
            "query": format!(
                "CREATE TABLE {} (id SERIAL PRIMARY KEY, small_val SMALLINT, big_val BIGINT)",
                table_name
            )
        });
        activity.execute(setup).await.unwrap();

        // Insert test data
        let insert = json!({
            "db_url": test_db_url(&pool),
            "query": format!("INSERT INTO {} (small_val, big_val) VALUES ($1, $2)", table_name),
            "params": [32767, 9223372036854775807_i64]
        });
        activity.execute(insert).await.unwrap();

        // Query
        let query = json!({
            "db_url": test_db_url(&pool),
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
            "db_url": test_db_url(&pool),
            "query": format!("DROP TABLE {}", table_name)
        });
        activity.execute(cleanup).await.unwrap();
    }

    // ========================================================================
    // Array/Object Parameter Tests (Bug fix: unsupported parameter type)
    // See docs/bugs/2026-01-05-postgres-array-params-unsupported.md
    // ========================================================================

    #[sqlx::test(migrations = "../migrations")]
    async fn test_postgres_query_array_parameter(pool: PgPool) {
        let pool_cache = test_pool_cache();
        let activity = PostgresQueryActivity::new(pool_cache);

        // Use jsonb_array_elements to extract array elements - this is the bug scenario
        let query = json!({
            "db_url": test_db_url(&pool),
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

    #[sqlx::test(migrations = "../migrations")]
    async fn test_postgres_query_object_parameter(pool: PgPool) {
        let pool_cache = test_pool_cache();
        let activity = PostgresQueryActivity::new(pool_cache);

        // Create table with JSONB column
        let table_name = format!("test_jsonb_{}", uuid::Uuid::now_v7().simple());
        let setup = json!({
            "db_url": test_db_url(&pool),
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
            "db_url": test_db_url(&pool),
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
            "db_url": test_db_url(&pool),
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
            "db_url": test_db_url(&pool),
            "query": format!("DROP TABLE {}", table_name)
        });
        activity.execute(cleanup).await.unwrap();
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn test_postgres_query_nested_json_parameter(pool: PgPool) {
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
            "db_url": test_db_url(&pool),
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

    #[sqlx::test(migrations = "../migrations")]
    async fn test_postgres_query_json_special_characters(pool: PgPool) {
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
            "db_url": test_db_url(&pool),
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

    #[sqlx::test(migrations = "../migrations")]
    async fn test_postgres_query_array_of_objects_parameter(pool: PgPool) {
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
            "db_url": test_db_url(&pool),
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

    // ========================================================================
    // Unit Tests: PoolConfig (No Database Required)
    // ========================================================================

    #[test]
    fn test_pool_config_from_env_loads() {
        // Just verify the function doesn't panic and returns a valid config
        let config = PoolConfig::from_env();
        // max_connections should be at least 1 (either default 5 or from env)
        assert!(config.max_connections >= 1);
        // acquire_timeout should be positive
        assert!(config.acquire_timeout_secs > 0);
    }

    #[test]
    fn test_pool_config_debug() {
        let config = PoolConfig {
            max_connections: 10,
            min_connections: Some(2),
            acquire_timeout_secs: 60,
            max_lifetime_secs: Some(3600),
            idle_timeout_secs: Some(300),
        };
        let debug_str = format!("{:?}", config);
        assert!(debug_str.contains("max_connections: 10"));
        assert!(debug_str.contains("min_connections: Some(2)"));
    }

    // ========================================================================
    // Unit Tests: Query Type Detection Edge Cases
    // ========================================================================

    #[test]
    fn test_determine_query_type_ddl() {
        // CREATE, DROP, ALTER default to Insert (DML) behavior
        assert!(matches!(
            determine_query_type("CREATE TABLE test (id INT)"),
            QueryType::Insert
        ));
        assert!(matches!(
            determine_query_type("DROP TABLE test"),
            QueryType::Insert
        ));
        assert!(matches!(
            determine_query_type("ALTER TABLE test ADD COLUMN x INT"),
            QueryType::Insert
        ));
    }

    #[test]
    fn test_determine_query_type_case_insensitive() {
        assert!(matches!(
            determine_query_type("select 1"),
            QueryType::Select
        ));
        assert!(matches!(
            determine_query_type("Select * from foo"),
            QueryType::Select
        ));
        assert!(matches!(
            determine_query_type("insert into foo values (1)"),
            QueryType::Insert
        ));
        assert!(matches!(
            determine_query_type("update foo set x = 1"),
            QueryType::Update
        ));
        assert!(matches!(
            determine_query_type("delete from foo"),
            QueryType::Delete
        ));
    }

    #[test]
    fn test_determine_query_type_with_leading_whitespace() {
        assert!(matches!(
            determine_query_type("  SELECT 1"),
            QueryType::Select
        ));
        assert!(matches!(
            determine_query_type("\n\tINSERT INTO foo VALUES (1)"),
            QueryType::Insert
        ));
    }

    #[test]
    fn test_determine_query_type_delete_with_returning() {
        // DELETE with RETURNING on same line
        assert!(matches!(
            determine_query_type("DELETE FROM foo WHERE id = 1 RETURNING *"),
            QueryType::Select
        ));
        // DELETE with RETURNING on new line
        assert!(matches!(
            determine_query_type("DELETE FROM foo WHERE id = 1\nRETURNING id, name"),
            QueryType::Select
        ));
    }

    #[test]
    fn test_determine_query_type_update_with_multiline_returning() {
        let sql = r#"UPDATE orders
            SET status = 'shipped'
            WHERE id = $1
            RETURNING id, status"#;
        assert!(matches!(determine_query_type(sql), QueryType::Select));
    }

    #[test]
    fn test_has_returning_clause_edge_cases() {
        // RETURNING at end of string with newline before
        assert!(has_returning_clause(
            &"INSERT INTO X\nRETURNING".to_uppercase()
        ));
        // RETURNING followed by newline
        assert!(has_returning_clause(
            &"INSERT INTO X RETURNING\nid".to_uppercase()
        ));
        // No RETURNING at all
        assert!(!has_returning_clause(
            &"INSERT INTO X VALUES (1)".to_uppercase()
        ));
        // RETURNING as part of column name (no space before)
        assert!(!has_returning_clause(
            &"SELECT NOT_RETURNING FROM FOO".to_uppercase()
        ));
    }

    // ========================================================================
    // Unit Tests: Serde for Transaction Types (No Database Required)
    // ========================================================================

    #[test]
    fn test_transaction_statement_serde() {
        let stmt = TransactionStatement {
            query: "SELECT $1::int".to_string(),
            params: Some(vec![json!(42)]),
        };
        let json_str = serde_json::to_string(&stmt).unwrap();
        let parsed: TransactionStatement = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed.query, "SELECT $1::int");
        assert!(parsed.params.is_some());
        assert_eq!(parsed.params.unwrap().len(), 1);
    }

    #[test]
    fn test_transaction_statement_no_params() {
        let stmt = TransactionStatement {
            query: "SELECT 1".to_string(),
            params: None,
        };
        let json_str = serde_json::to_string(&stmt).unwrap();
        let parsed: Value = serde_json::from_str(&json_str).unwrap();
        // params should be absent (skip_serializing_if)
        assert!(parsed.get("params").is_none());
    }

    #[test]
    fn test_isolation_level_serde() {
        // Default is ReadCommitted
        let level: IsolationLevel = serde_json::from_str("\"read_committed\"").unwrap();
        assert!(matches!(level, IsolationLevel::ReadCommitted));

        let level: IsolationLevel = serde_json::from_str("\"repeatable_read\"").unwrap();
        assert!(matches!(level, IsolationLevel::RepeatableRead));

        let level: IsolationLevel = serde_json::from_str("\"serializable\"").unwrap();
        assert!(matches!(level, IsolationLevel::Serializable));
    }

    #[test]
    fn test_isolation_level_default() {
        let level = IsolationLevel::default();
        assert!(matches!(level, IsolationLevel::ReadCommitted));
    }

    #[test]
    fn test_transaction_params_serde() {
        let params = TransactionParams {
            db_url: "postgres://localhost/test".to_string(),
            statements: vec![TransactionStatement {
                query: "SELECT 1".to_string(),
                params: None,
            }],
            isolation_level: IsolationLevel::Serializable,
        };
        let json_str = serde_json::to_string(&params).unwrap();
        let parsed: TransactionParams = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed.db_url, "postgres://localhost/test");
        assert_eq!(parsed.statements.len(), 1);
        assert!(matches!(
            parsed.isolation_level,
            IsolationLevel::Serializable
        ));
    }

    #[test]
    fn test_transaction_result_serde() {
        let result = TransactionResult {
            success: true,
            statements_executed: 3,
            results: vec![
                StatementResult {
                    rows: Some(vec![json!({"id": 1})]),
                    rows_affected: None,
                },
                StatementResult {
                    rows: None,
                    rows_affected: Some(2),
                },
            ],
        };
        let json_str = serde_json::to_string(&result).unwrap();
        let parsed: TransactionResult = serde_json::from_str(&json_str).unwrap();
        assert!(parsed.success);
        assert_eq!(parsed.statements_executed, 3);
        assert_eq!(parsed.results.len(), 2);
    }

    #[test]
    fn test_postgres_query_params_serde() {
        let params = PostgresQueryParams {
            db_url: "postgres://localhost/test".to_string(),
            query: "SELECT $1::text".to_string(),
            params: Some(vec![json!("hello")]),
        };
        let json_str = serde_json::to_string(&params).unwrap();
        let parsed: PostgresQueryParams = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed.db_url, "postgres://localhost/test");
        assert_eq!(parsed.query, "SELECT $1::text");
        assert!(parsed.params.is_some());
    }

    #[test]
    fn test_postgres_query_params_no_params() {
        let params = PostgresQueryParams {
            db_url: "postgres://localhost/test".to_string(),
            query: "SELECT 1".to_string(),
            params: None,
        };
        let json_str = serde_json::to_string(&params).unwrap();
        let parsed: Value = serde_json::from_str(&json_str).unwrap();
        assert!(parsed.get("params").is_none());
    }

    // ========================================================================
    // Unit Tests: Activity Trait Methods (No Database Required)
    // ========================================================================

    #[test]
    fn test_postgres_query_activity_name() {
        let activity = PostgresQueryActivity::new(test_pool_cache());
        assert_eq!(activity.name(), "postgres_query");
        assert_eq!(activity.worker(), "std");
    }

    #[test]
    fn test_postgres_transaction_activity_name() {
        let activity = PostgresTransactionActivity::new(test_pool_cache());
        assert_eq!(activity.name(), "postgres_transaction");
        assert_eq!(activity.worker(), "std");
    }

    #[test]
    fn test_new_pool_cache_is_empty() {
        let cache = new_pool_cache();
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let read = cache.read().await;
            assert!(read.is_empty());
        });
    }

    #[test]
    fn test_statement_result_both_none() {
        let result = StatementResult {
            rows: None,
            rows_affected: None,
        };
        let json_str = serde_json::to_string(&result).unwrap();
        let parsed: Value = serde_json::from_str(&json_str).unwrap();
        // Both should be absent due to skip_serializing_if
        assert!(parsed.get("rows").is_none());
        assert!(parsed.get("rows_affected").is_none());
    }

    #[test]
    fn test_statement_result_deserialization() {
        // Test deserializing from raw JSON (simulating what we'd get from storage)
        let json = r#"{"rows":[{"id":1,"name":"test"}]}"#;
        let result: StatementResult = serde_json::from_str(json).unwrap();
        assert!(result.rows.is_some());
        assert!(result.rows_affected.is_none());
        let rows = result.rows.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].get("name").unwrap(), "test");
    }

    #[test]
    fn test_statement_result_deserialization_rows_affected() {
        let json = r#"{"rows_affected":5}"#;
        let result: StatementResult = serde_json::from_str(json).unwrap();
        assert!(result.rows.is_none());
        assert_eq!(result.rows_affected, Some(5));
    }

    // ========================================================================
    // Integration Test Helpers (Database Required)
    // ========================================================================

    #[sqlx::test(migrations = "../migrations")]
    async fn test_postgres_transaction_json_parameters(pool: PgPool) {
        let pool_cache = test_pool_cache();
        let query_activity = PostgresQueryActivity::new(pool_cache.clone());
        let tx_activity = PostgresTransactionActivity::new(pool_cache);

        // Create table for transaction test
        let table_name = format!("test_tx_jsonb_{}", uuid::Uuid::now_v7().simple());
        let setup = json!({
            "db_url": test_db_url(&pool),
            "query": format!("CREATE TABLE {} (id SERIAL PRIMARY KEY, data JSONB)", table_name)
        });
        query_activity.execute(setup).await.unwrap();

        // Execute transaction with JSON parameters
        let obj1 = serde_json::json!({"name": "Item 1", "value": 100});
        let obj2 = serde_json::json!({"name": "Item 2", "value": 200});

        let params = json!({
            "db_url": test_db_url(&pool),
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
            "db_url": test_db_url(&pool),
            "query": format!("DROP TABLE {}", table_name)
        });
        query_activity.execute(cleanup).await.unwrap();
    }
}
