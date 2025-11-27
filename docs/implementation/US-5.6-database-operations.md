# US-5.6: Database Operations Implementation Plan

**Version**: 1.2
**Date**: 2025-11-26
**Status**: ✅ Complete (`postgres_query` ✅, `postgres_transaction` ✅)
**Epic**: 5 - Built-In Activity Library

---

## Overview

This story implements built-in database connector activities that enable workflows to query and update databases directly without external workers.

**User Story**:
> As a data engineer, I want built-in database connectors, so that workflows can query and update databases directly.

---

## Acceptance Criteria

| Criterion                                    | Status |
|----------------------------------------------|--------|
| `postgres_query` activity                    | ✅     |
| `postgres_transaction` activity              | ✅     |
| PostgreSQL native: Direct queries            | ✅     |
| SQL transactions: Multi-statement atomicity  | ✅     |
| Connection pooling built-in                  | ✅     |
| Parameter binding for SQL injection prevention | ✅   |
| Shared pool cache between activities         | ✅     |
| Isolation level configuration                | ✅     |
| RETURNING clause support                     | ✅     |

**Deferred to Post-MVP**: Redis and SQLite as activity I/O backends (see `docs/post-mvp.md` Story 1.14)

---

## Activity 1: `postgres_query` ✅ COMPLETE

### Description

Executes a single SQL query against a PostgreSQL database with parameterized binding.

### Implementation

**File**: `worker/src/activities/postgres.rs`

### YAML Usage

```yaml
activities:
  fetch_users:
    activity: postgres_query
    parameters:
      db_url: "{{SECRET.db_url}}"
      query: "SELECT id, name, email FROM users WHERE status = $1"
      params:
        - "active"
```

### Parameters

| Parameter | Type       | Required | Description                                       |
|-----------|------------|----------|---------------------------------------------------|
| `db_url`  | string     | Yes      | PostgreSQL connection URL                         |
| `query`   | string     | Yes      | SQL query to execute                              |
| `params`  | array      | No       | Positional parameters for parameterized queries   |

### Output

For SELECT queries:
```json
{
  "result": {
    "rows": [
      {"id": 1, "name": "Alice", "email": "alice@example.com"},
      {"id": 2, "name": "Bob", "email": "bob@example.com"}
    ]
  }
}
```

For INSERT/UPDATE/DELETE queries:
```json
{
  "result": {
    "rows_affected": 3
  }
}
```

### Features Implemented

1. **Connection Pool Caching**
   - Pools cached by `db_url` to avoid connection overhead
   - Configurable via environment variables:
     - `STREAMFLOW_POSTGRES_POOL_MAX_CONNECTIONS` (default: 5)
     - `STREAMFLOW_POSTGRES_POOL_MIN_CONNECTIONS`
     - `STREAMFLOW_POSTGRES_POOL_ACQUIRE_TIMEOUT_SECS` (default: 30)
     - `STREAMFLOW_POSTGRES_POOL_MAX_LIFETIME_SECS`
     - `STREAMFLOW_POSTGRES_POOL_IDLE_TIMEOUT_SECS`

2. **Query Type Detection**
   - Automatic detection of SELECT, INSERT, UPDATE, DELETE
   - SELECT returns rows as JSON array
   - DML returns `rows_affected` count

3. **Parameter Binding**
   - Supports: String, Number (i64, f64), Boolean, Null
   - Uses `$1`, `$2`, etc. for positional parameters
   - Prevents SQL injection

4. **Row-to-JSON Conversion**
   - Automatic type mapping: TEXT→String, INT→Number, BOOL→Boolean
   - JSONB columns preserved as JSON
   - NULL values handled correctly

### Test Coverage

- `test_postgres_query_select` - Basic SELECT query
- `test_postgres_query_with_params` - Parameterized queries
- `test_postgres_query_insert` - INSERT with row count
- `test_postgres_query_update` - UPDATE with row count
- `test_postgres_pool_reuse` - Connection pool caching

---

## Activity 2: `postgres_transaction` ✅ COMPLETE

### Description

Executes multiple SQL statements within a single atomic transaction. All statements succeed or all are rolled back.

### YAML Usage

```yaml
activities:
  transfer_funds:
    activity: postgres_transaction
    parameters:
      db_url: "{{SECRET.db_url}}"
      statements:
        - query: "UPDATE accounts SET balance = balance - $1 WHERE id = $2"
          params: [100.00, "{{INPUT.from_account}}"]
        - query: "UPDATE accounts SET balance = balance + $1 WHERE id = $2"
          params: [100.00, "{{INPUT.to_account}}"]
        - query: "INSERT INTO transactions (from_id, to_id, amount, created_at) VALUES ($1, $2, $3, NOW())"
          params: ["{{INPUT.from_account}}", "{{INPUT.to_account}}", 100.00]
      isolation_level: "read_committed"  # optional
```

### Parameters

| Parameter         | Type       | Required | Default          | Description                                    |
|-------------------|------------|----------|------------------|------------------------------------------------|
| `db_url`          | string     | Yes      | -                | PostgreSQL connection URL                      |
| `statements`      | array      | Yes      | -                | Array of statement objects                     |
| `isolation_level` | string     | No       | `read_committed` | Transaction isolation level                    |

**Statement Object**:
| Field    | Type   | Required | Description                         |
|----------|--------|----------|-------------------------------------|
| `query`  | string | Yes      | SQL statement to execute            |
| `params` | array  | No       | Parameters for this statement       |

**Isolation Levels**:
- `read_committed` (default) - Standard PostgreSQL default
- `repeatable_read` - Snapshot isolation
- `serializable` - Full serializability

### Output

```json
{
  "result": {
    "success": true,
    "statements_executed": 3,
    "results": [
      {"rows_affected": 1},
      {"rows_affected": 1},
      {"rows_affected": 1}
    ]
  }
}
```

On rollback:
```json
{
  "error": {
    "code": "TRANSACTION_FAILED",
    "message": "Statement 2 failed: duplicate key violation",
    "statement_index": 1,
    "rolled_back": true
  }
}
```

### Implementation Tasks

1. **Refactor to shared `Arc<PoolCache>`** (~1.5 hours)
   - Extract `PoolCache` type to be passed at construction time
   - Update `PostgresQueryActivity::new(pool_cache)` to accept shared cache
   - Create `get_or_create_pool(cache, db_url, config)` shared function
   - Create `bind_params(query, params)` helper for parameter binding
   - Create `execute_statement(executor, query, params)` generic over `PgExecutor`
   - Update `register_builtin_activities` to create and share the cache

2. **Add PostgresTransactionActivity** (~2 hours)
   - New struct accepting shared `Arc<PoolCache>` at construction
   - Implement `ActivityImpl` trait
   - Transaction logic: BEGIN → execute statements → COMMIT/ROLLBACK
   - Isolation level support via `SET TRANSACTION ISOLATION LEVEL`
   - Register in activity module exports

3. **Tests** (~2 hours)
   - `test_postgres_transaction_commit` - All statements succeed
   - `test_postgres_transaction_rollback` - Failure causes rollback
   - `test_postgres_transaction_isolation` - Isolation level respected
   - `test_postgres_transaction_mixed_queries` - SELECT + DML in same transaction

### Estimated Duration

**Total**: ~5.5 hours

### Code Sharing Summary

| Component                  | Current Location           | Change |
|----------------------------|----------------------------|--------|
| `PoolCache` type           | `PostgresExecutor`         | ✅ Accept via constructor, create in `register_builtin_activities` |
| `PoolConfig` struct        | `PostgresExecutor`         | ✅ Already reusable |
| `get_pool()` method        | `PostgresExecutor`         | ✅ Extract to `get_or_create_pool(cache, url, config)` |
| `determine_query_type()`   | Module function            | ✅ Already shared |
| `row_to_json()`            | Module function            | ✅ Already shared |
| Parameter binding          | `PostgresExecutor::execute`| ✅ Extract to `bind_param()` helper |

---

## Implementation Architecture

### File Structure

```
worker/src/activities/
├── mod.rs                 # Module exports (add PostgresTransactionActivity)
├── postgres.rs            # Shared utilities + both activities
│   ├── PoolCache (type)        # Arc<RwLock<HashMap<String, PgPool>>>
│   ├── PoolConfig              # Pool configuration (existing)
│   ├── get_or_create_pool()    # Shared pool management (accepts cache)
│   ├── bind_param()            # Shared parameter binding
│   ├── execute_statement()     # Shared statement execution
│   ├── determine_query_type()  # Existing
│   ├── row_to_json()           # Existing
│   ├── PostgresQueryActivity   # Accepts Arc<PoolCache> at construction
│   └── PostgresTransactionActivity  # NEW - shares same cache
└── ...

worker/src/builtin.rs      # Creates shared PoolCache, passes to both activities
```

### Shared Utilities

```rust
// ============================================================================
// Shared Pool Management
// ============================================================================

/// Connection pool cache type - shared across all PostgreSQL activities
pub type PoolCache = Arc<RwLock<HashMap<String, PgPool>>>;

/// Create a new empty pool cache
pub fn new_pool_cache() -> PoolCache {
    Arc::new(RwLock::new(HashMap::new()))
}

/// Get or create a connection pool for the given database URL
async fn get_or_create_pool(
    cache: &PoolCache,
    db_url: &str,
    config: &PoolConfig,
) -> Result<PgPool> {
    // Check cache
    {
        let cache_read = cache.read().await;
        if let Some(pool) = cache_read.get(db_url) {
            return Ok(pool.clone());
        }
    }

    // Create new pool with config
    let pool = PgPoolOptions::new()
        .max_connections(config.max_connections)
        .acquire_timeout(Duration::from_secs(config.acquire_timeout_secs))
        // ... other options
        .connect(db_url)
        .await?;

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rows: Option<Vec<Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rows_affected: Option<u64>,
}

/// Execute a single statement, generic over executor (pool or transaction)
async fn execute_statement<'e, E>(
    executor: E,
    query_str: &str,
    params: Option<&[Value]>,
) -> Result<StatementResult>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let mut query = sqlx::query(query_str);

    // Bind parameters
    if let Some(params) = params {
        for param in params {
            query = bind_param(query, param)?;
        }
    }

    let query_type = determine_query_type(query_str);

    match query_type {
        QueryType::Select => {
            let rows = query.fetch_all(executor).await?;
            let results: Vec<Value> = rows.iter().map(row_to_json).collect::<Result<_>>()?;
            Ok(StatementResult { rows: Some(results), rows_affected: None })
        }
        _ => {
            let result = query.execute(executor).await?;
            Ok(StatementResult { rows: None, rows_affected: Some(result.rows_affected()) })
        }
    }
}

/// Bind a single parameter value to a query
fn bind_param<'q>(
    query: Query<'q, Postgres, PgArguments>,
    param: &Value,
) -> Result<Query<'q, Postgres, PgArguments>> {
    Ok(match param {
        Value::String(s) => query.bind(s.clone()),
        Value::Number(n) if n.is_i64() => query.bind(n.as_i64().unwrap()),
        Value::Number(n) => query.bind(n.as_f64().unwrap()),
        Value::Bool(b) => query.bind(*b),
        Value::Null => query.bind(Option::<String>::None),
        _ => return Err(anyhow::anyhow!("Unsupported parameter type")),
    })
}
```

### Activity Implementations

```rust
// ============================================================================
// PostgresQueryActivity (refactored to accept shared cache)
// ============================================================================

pub struct PostgresQueryActivity {
    pool_cache: PoolCache,
    pool_config: PoolConfig,
}

impl PostgresQueryActivity {
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
        let params: PostgresQueryParams = serde_json::from_value(parameters)?;
        let pool = get_or_create_pool(&self.pool_cache, &params.db_url, &self.pool_config).await?;
        // ... rest of implementation unchanged
    }
    // ...
}

// ============================================================================
// PostgresTransactionActivity (NEW - shares same cache)
// ============================================================================

pub struct PostgresTransactionActivity {
    pool_cache: PoolCache,
    pool_config: PoolConfig,
}

impl PostgresTransactionActivity {
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
        let params: TransactionParams = serde_json::from_value(parameters)?;

        let pool = get_or_create_pool(&self.pool_cache, &params.db_url, &self.pool_config).await?;
        let mut tx = pool.begin().await?;

        // Set isolation level
        let isolation_sql = match params.isolation_level {
            IsolationLevel::ReadCommitted => "SET TRANSACTION ISOLATION LEVEL READ COMMITTED",
            IsolationLevel::RepeatableRead => "SET TRANSACTION ISOLATION LEVEL REPEATABLE READ",
            IsolationLevel::Serializable => "SET TRANSACTION ISOLATION LEVEL SERIALIZABLE",
        };
        sqlx::query(isolation_sql).execute(&mut *tx).await?;

        let mut results = Vec::new();

        for (idx, stmt) in params.statements.iter().enumerate() {
            match execute_statement(&mut *tx, &stmt.query, stmt.params.as_deref()).await {
                Ok(result) => results.push(result),
                Err(e) => {
                    tx.rollback().await.ok(); // Best effort rollback
                    return Err(anyhow::anyhow!(
                        "Statement {} failed: {}. Transaction rolled back.",
                        idx, e
                    ));
                }
            }
        }

        tx.commit().await?;

        let output = serde_json::to_value(TransactionResult {
            success: true,
            statements_executed: results.len(),
            results,
        })?;

        Ok(ActivityResult::value("result", output))
    }

    fn name(&self) -> &str { "postgres_transaction" }
    fn worker(&self) -> &str { "builtin" }
}
```

### Registration in builtin.rs

```rust
use crate::activities::{
    EchoActivity, EmbeddingActivity, HttpRequestActivity, LLMPromptActivity,
    PostgresQueryActivity, PostgresTransactionActivity,
    new_pool_cache,  // NEW: factory function for shared cache
};

pub fn register_builtin_activities(cache_service: Arc<dyn CacheService>) -> ActivityRegistry {
    let mut registry = ActivityRegistry::new(cache_service);

    // Create shared PostgreSQL connection pool cache
    let pg_pool_cache = new_pool_cache();

    // Register activities
    registry.register(Arc::new(EchoActivity));
    registry.register(Arc::new(HttpRequestActivity::new()));

    // PostgreSQL activities share the same pool cache
    registry.register(Arc::new(PostgresQueryActivity::new(pg_pool_cache.clone())));
    registry.register(Arc::new(PostgresTransactionActivity::new(pg_pool_cache)));

    // LLM activities
    registry.register(Arc::new(LLMPromptActivity::new()));
    registry.register(Arc::new(EmbeddingActivity::new()));

    registry
}
```

### Transaction Types

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionParams {
    pub db_url: String,
    pub statements: Vec<TransactionStatement>,
    #[serde(default)]
    pub isolation_level: IsolationLevel,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionStatement {
    pub query: String,
    #[serde(default)]
    pub params: Option<Vec<Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum IsolationLevel {
    #[default]
    ReadCommitted,
    RepeatableRead,
    Serializable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionResult {
    pub success: bool,
    pub statements_executed: usize,
    pub results: Vec<StatementResult>,
}
```

---

## Example Workflows

### Example: Bank Transfer (Atomic)

```yaml
name: bank_transfer
description: Transfer funds between accounts atomically

activities:
  transfer:
    activity: postgres_transaction
    parameters:
      db_url: "{{SECRET.bank_db_url}}"
      statements:
        - query: |
            UPDATE accounts
            SET balance = balance - $1,
                updated_at = NOW()
            WHERE id = $2 AND balance >= $1
          params:
            - "{{INPUT.amount}}"
            - "{{INPUT.from_account}}"
        - query: |
            UPDATE accounts
            SET balance = balance + $1,
                updated_at = NOW()
            WHERE id = $2
          params:
            - "{{INPUT.amount}}"
            - "{{INPUT.to_account}}"
        - query: |
            INSERT INTO transaction_log (from_id, to_id, amount, status, created_at)
            VALUES ($1, $2, $3, 'completed', NOW())
          params:
            - "{{INPUT.from_account}}"
            - "{{INPUT.to_account}}"
            - "{{INPUT.amount}}"
      isolation_level: serializable

  notify_sender:
    activity: http_request
    parameters:
      method: POST
      url: "{{INPUT.notification_url}}"
      body:
        message: "Transfer of ${{INPUT.amount}} completed"
        transaction_id: "{{WORKFLOW.id}}"
    depends_on:
      - transfer
```

### Example: Inventory Update with Validation

```yaml
name: process_order
description: Update inventory and create order atomically

activities:
  process:
    activity: postgres_transaction
    parameters:
      db_url: "{{SECRET.db_url}}"
      statements:
        # Check and decrement inventory
        - query: |
            UPDATE inventory
            SET quantity = quantity - $1
            WHERE product_id = $2 AND quantity >= $1
            RETURNING quantity
          params:
            - "{{INPUT.quantity}}"
            - "{{INPUT.product_id}}"
        # Create order record
        - query: |
            INSERT INTO orders (product_id, quantity, customer_id, status, created_at)
            VALUES ($1, $2, $3, 'pending', NOW())
            RETURNING id
          params:
            - "{{INPUT.product_id}}"
            - "{{INPUT.quantity}}"
            - "{{INPUT.customer_id}}"
```

---

## Testing Strategy

### Unit Tests

Located in `worker/src/activities/postgres.rs`:

```rust
#[cfg(test)]
mod transaction_tests {
    #[tokio::test]
    async fn test_postgres_transaction_commit() {
        // Create test table, execute transaction, verify all changes applied
    }

    #[tokio::test]
    async fn test_postgres_transaction_rollback() {
        // Execute transaction with intentional failure, verify rollback
    }

    #[tokio::test]
    async fn test_postgres_transaction_isolation() {
        // Test serializable isolation prevents phantom reads
    }
}
```

### Integration Tests

Add to `api/tests/` for end-to-end workflow testing:

```rust
#[tokio::test]
async fn test_transaction_workflow_e2e() {
    // Submit workflow with postgres_transaction activity
    // Verify atomic execution
}
```

---

## Success Criteria

- [ ] `postgres_transaction` activity executes multiple statements atomically
- [ ] Automatic rollback on any statement failure
- [ ] Configurable isolation levels (read_committed, repeatable_read, serializable)
- [ ] Detailed error reporting with statement index
- [ ] Connection pool shared with `postgres_query`
- [ ] All tests pass
- [ ] Example workflow documented

---

## Post-MVP: Deferred Database Operations

The following database operations are deferred to post-MVP (see `docs/post-mvp.md` Story 1.14):

1. **Redis Operations**
   - `redis_get`, `redis_set`, `redis_del`
   - `redis_hash_get`, `redis_hash_set`
   - `redis_list_push`, `redis_list_pop`
   - Pub/sub support

2. **SQLite Operations**
   - `sqlite_query` - For edge deployment
   - `sqlite_transaction` - Atomic transactions
   - Embedded database for offline workflows

These are deferred because:
- PostgreSQL covers 90%+ of workflow database needs
- Redis/SQLite add complexity without immediate user demand
- Edge deployment (SQLite) is a post-MVP focus area
