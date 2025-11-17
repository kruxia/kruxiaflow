use crate::registry::ActivityImpl;
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::postgres::{PgPool, PgPoolOptions, PgRow};
use sqlx::{Column, Row};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

// ============================================================================
// PostgreSQL Executor
// ============================================================================

/// Connection pool cache for reusing connections across activities
type PoolCache = Arc<RwLock<HashMap<String, PgPool>>>;

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

/// PostgreSQL activity executor
struct PostgresExecutor {
    pool_cache: PoolCache,
    pool_config: PoolConfig,
}

impl PostgresExecutor {
    fn new() -> Self {
        Self {
            pool_cache: Arc::new(RwLock::new(HashMap::new())),
            pool_config: PoolConfig::from_env(),
        }
    }
}

impl PostgresExecutor {
    /// Get or create a connection pool for the given database URL
    ///
    /// Uses system-level pool configuration from environment variables.
    /// Pools are cached by db_url to avoid connection overhead.
    async fn get_pool(&self, db_url: &str) -> Result<PgPool> {
        // Check if pool exists in cache
        {
            let cache = self.pool_cache.read().await;
            if let Some(pool) = cache.get(db_url) {
                return Ok(pool.clone());
            }
        }

        // Create new pool with system configuration
        let mut options = PgPoolOptions::new()
            .max_connections(self.pool_config.max_connections)
            .acquire_timeout(Duration::from_secs(self.pool_config.acquire_timeout_secs));

        if let Some(min_connections) = self.pool_config.min_connections {
            options = options.min_connections(min_connections);
        }

        if let Some(max_lifetime_secs) = self.pool_config.max_lifetime_secs {
            options = options.max_lifetime(Duration::from_secs(max_lifetime_secs));
        }

        if let Some(idle_timeout_secs) = self.pool_config.idle_timeout_secs {
            options = options.idle_timeout(Duration::from_secs(idle_timeout_secs));
        }

        let pool = options
            .connect(db_url)
            .await
            .context("Failed to connect to PostgreSQL database")?;

        // Store in cache
        {
            let mut cache = self.pool_cache.write().await;
            cache.insert(db_url.to_string(), pool.clone());
        }

        Ok(pool)
    }

    /// Execute a PostgreSQL query
    async fn execute(&self, params: PostgresQueryParams) -> Result<PostgresQueryResult> {
        let pool = self.get_pool(&params.db_url).await?;

        // Build parameterized query
        let mut query = sqlx::query(&params.query);

        // Add parameters
        if let Some(params_list) = &params.params {
            for param in params_list {
                query = match param {
                    Value::String(s) => query.bind(s),
                    Value::Number(n) => {
                        if let Some(i) = n.as_i64() {
                            query.bind(i)
                        } else if let Some(f) = n.as_f64() {
                            query.bind(f)
                        } else {
                            return Err(anyhow::anyhow!("Invalid number parameter"));
                        }
                    }
                    Value::Bool(b) => query.bind(b),
                    Value::Null => query.bind(Option::<String>::None),
                    _ => {
                        return Err(anyhow::anyhow!(
                            "Unsupported parameter type: {}",
                            param.to_string()
                        ))
                    }
                };
            }
        }

        // Determine query type from SQL (simple heuristic)
        let query_type = determine_query_type(&params.query);

        match query_type {
            QueryType::Select => {
                // Execute SELECT query and fetch results
                let rows = query.fetch_all(&pool).await.context("Failed to execute SELECT query")?;

                // Convert rows to JSON
                let results: Vec<Value> = rows
                    .iter()
                    .map(|row| row_to_json(row))
                    .collect::<Result<Vec<_>>>()?;

                Ok(PostgresQueryResult {
                    rows: Some(results),
                    rows_affected: None,
                })
            }
            QueryType::Insert | QueryType::Update | QueryType::Delete => {
                // Execute DML query
                let result = query
                    .execute(&pool)
                    .await
                    .context("Failed to execute query")?;

                Ok(PostgresQueryResult {
                    rows: None,
                    rows_affected: Some(result.rows_affected()),
                })
            }
        }
    }
}

impl Default for PostgresExecutor {
    fn default() -> Self {
        Self::new()
    }
}

/// Determine query type from SQL statement
fn determine_query_type(sql: &str) -> QueryType {
    let sql_upper = sql.trim_start().to_uppercase();

    if sql_upper.starts_with("SELECT") || sql_upper.starts_with("WITH") {
        QueryType::Select
    } else if sql_upper.starts_with("INSERT") {
        QueryType::Insert
    } else if sql_upper.starts_with("UPDATE") {
        QueryType::Update
    } else if sql_upper.starts_with("DELETE") {
        QueryType::Delete
    } else {
        // Default to DML for unknown queries
        QueryType::Insert
    }
}

/// Query type enum
enum QueryType {
    Select,
    Insert,
    Update,
    Delete,
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

// ============================================================================
// Request/Response Types
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

/// PostgreSQL query result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostgresQueryResult {
    /// Result rows (for SELECT queries)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rows: Option<Vec<Value>>,

    /// Number of rows affected (for INSERT/UPDATE/DELETE)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rows_affected: Option<u64>,
}

// ============================================================================
// PostgreSQL Activity (ActivityImpl wrapper for built-in worker)
// ============================================================================

/// PostgreSQL query activity (built-in worker)
///
/// Executes SQL queries with parameterized binding
pub struct PostgresQueryActivity {
    executor: PostgresExecutor,
}

impl PostgresQueryActivity {
    pub fn new() -> Self {
        Self {
            executor: PostgresExecutor::new(),
        }
    }
}

impl Default for PostgresQueryActivity {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ActivityImpl for PostgresQueryActivity {
    async fn execute(&self, parameters: Value) -> Result<Value> {
        tracing::debug!("Executing postgres_query activity with parameters: {:?}", parameters);

        // Parse parameters from JSON
        let params: PostgresQueryParams = serde_json::from_value(parameters)
            .context("Failed to parse PostgreSQL query parameters")?;

        // Execute query
        let result = self.executor.execute(params).await?;

        // Serialize result to JSON for output
        let output = serde_json::to_value(&result)
            .context("Failed to serialize PostgreSQL query result")?;

        tracing::debug!("PostgreSQL query completed");

        Ok(output)
    }

    fn name(&self) -> &str {
        "postgres_query"
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

    #[tokio::test]
    async fn test_postgres_query_select() {
        let activity = PostgresQueryActivity::new();

        let params = json!({
            "db_url": test_db_url(),
            "query": "SELECT 1 as num, 'test' as text, true as flag"
        });

        let result = activity.execute(params).await.unwrap();

        assert!(result.get("rows").is_some());
        let rows = result.get("rows").unwrap().as_array().unwrap();
        assert_eq!(rows.len(), 1);

        let row = &rows[0];
        assert_eq!(row.get("num").unwrap(), 1);
        assert_eq!(row.get("text").unwrap(), "test");
        assert_eq!(row.get("flag").unwrap(), true);
    }

    #[tokio::test]
    async fn test_postgres_query_with_params() {
        let activity = PostgresQueryActivity::new();

        let params = json!({
            "db_url": test_db_url(),
            "query": "SELECT $1::text as name, $2::int as age",
            "params": ["Alice", 30]
        });

        let result = activity.execute(params).await.unwrap();

        assert!(result.get("rows").is_some());
        let rows = result.get("rows").unwrap().as_array().unwrap();
        assert_eq!(rows.len(), 1);

        let row = &rows[0];
        assert_eq!(row.get("name").unwrap(), "Alice");
        assert_eq!(row.get("age").unwrap(), 30);
    }

    #[tokio::test]
    async fn test_postgres_query_insert() {
        let activity = PostgresQueryActivity::new();

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

        assert!(result.get("rows_affected").is_some());
        assert_eq!(result.get("rows_affected").unwrap(), 1);

        // Cleanup
        let cleanup = json!({
            "db_url": test_db_url(),
            "query": format!("DROP TABLE {}", table_name)
        });
        activity.execute(cleanup).await.unwrap();
    }

    #[tokio::test]
    async fn test_postgres_query_update() {
        let activity = PostgresQueryActivity::new();

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

        assert!(result.get("rows_affected").is_some());
        assert_eq!(result.get("rows_affected").unwrap(), 1);

        // Cleanup
        let cleanup = json!({
            "db_url": test_db_url(),
            "query": format!("DROP TABLE {}", table_name)
        });
        activity.execute(cleanup).await.unwrap();
    }

    #[tokio::test]
    async fn test_postgres_pool_reuse() {
        let activity = PostgresQueryActivity::new();

        // First query creates pool
        let params1 = json!({
            "db_url": test_db_url(),
            "query": "SELECT 'first' as result"
        });
        let result1 = activity.execute(params1).await.unwrap();
        assert!(result1.get("rows").is_some());

        // Second query should reuse cached pool
        let params2 = json!({
            "db_url": test_db_url(),
            "query": "SELECT 'second' as result"
        });
        let result2 = activity.execute(params2).await.unwrap();
        assert!(result2.get("rows").is_some());

        let rows = result2.get("rows").unwrap().as_array().unwrap();
        assert_eq!(rows[0].get("result").unwrap(), "second");
    }
}
