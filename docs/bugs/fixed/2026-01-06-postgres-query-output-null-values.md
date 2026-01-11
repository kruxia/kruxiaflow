# Bug: postgres_query Activity Output Returns Null Values for Database Fields

**Date**: 2026-01-06
**Status**: Resolved
**Severity**: High
**Component**: Worker / Template Engine / Activity Output Serialization
**Resolution Date**: 2026-01-06

## Summary

When a `postgres_query` activity result is passed to subsequent activities via template expressions, the field values are converted to `null` even though the database returned valid data. This makes it impossible to use query results for filtering or processing in downstream activities.

## Symptoms

When passing rows from a `postgres_query` result to a subsequent activity:

```yaml
- key: search_passages
  worker: builtin
  activity_name: postgres_query
  parameters:
    query: |
      SELECT p.id, p.source_id, p.page_start, s.title
      FROM passages p
      JOIN sources s ON p.source_id = s.id
      WHERE s.user_id = $1::uuid
    params:
      - "{{INPUT.user_id}}"
  outputs:
    - rows

- key: process_results
  worker: researcher
  activity_name: fetch.annotations
  parameters:
    passages: "{{search_passages.result.rows}}"
  depends_on:
    - search_passages
```

The custom worker activity receives `null` for fields like `id`, `source_id`, `page_start`:

```
fetch.annotations first passage id: Some(Null)
fetch.annotations first passage source_id: Some(Null)
fetch.annotations first passage page_start: Some(Null)
```

## Observed Behavior

1. **Query executes successfully** - The database query returns correct results
2. **Activity completes successfully** - The postgres_query activity reports completion with rows
3. **Template expression evaluates** - `{{search_passages.result.rows}}` evaluates and is passed
4. **Field values are null** - All non-string fields in the rows are converted to null

## Root Cause Hypothesis

The issue appears to be in the serialization/deserialization pipeline between activity output storage and template rendering. Possible locations:

1. **Activity result serialization** (`worker/src/activities/postgres.rs`)
   - `sqlx::Row` to `serde_json::Value` conversion may be losing type information

2. **Template value conversion** (`core/src/workflow/template.rs`)
   - `serde_json_to_minijinja()` or `minijinja_to_serde_json()` may be dropping values

3. **Activity output storage/retrieval** (`core/src/orchestrator/`)
   - JSON round-trip through the database may be affecting field values

4. **Worker parameter deserialization**
   - Custom worker may be receiving pre-processed values that lost type information

## Steps to Reproduce

1. Create a workflow with a `postgres_query` activity that selects UUID, integer, and text fields
2. Add a custom worker activity that receives `{{previous.result.rows}}`
3. Log the received values in the custom worker
4. Observe that non-string fields are null

### Minimal Reproduction Workflow

```yaml
name: test_null_values
namespace: test

activities:
  - key: get_data
    worker: builtin
    activity_name: postgres_query
    parameters:
      db_url: "{{INPUT.db_url}}"
      query: |
        SELECT
          id,           -- UUID
          page_start,   -- INTEGER
          content       -- TEXT
        FROM passages
        LIMIT 1
      params: []
    outputs:
      - rows

  - key: use_data
    worker: test
    activity_name: echo
    parameters:
      data: "{{get_data.result.rows}}"
    depends_on:
      - get_data
```

## Impact

- **Blocked**: Cannot pass structured query results between activities
- **Blocked**: Cannot implement passage-specific annotation filtering in research queries
- **Workaround**: None that preserves field values - must restructure workflows to avoid inter-activity data passing

## Investigation Notes

### What Works

- **String fields** - Text/varchar columns appear to pass through correctly
- **Direct LLM prompt templates** - Jinja templates in `llm_prompt` activities can iterate over rows and access fields
- **Single activity outputs** - Using results within the same activity works

### What Doesn't Work

- **UUID fields** - Become null when passed to another activity
- **Integer fields** - Become null when passed to another activity
- **Nested access** - `{{rows[0].id}}` returns null even though `rows[0]` exists

### Debugging Session Output

From custom worker logs:
```
INFO fetch.annotations first passage id: Some(Null)
INFO fetch.annotations first passage source_id: Some(Null)
INFO fetch.annotations first passage page_start: Some(Null)
```

The `Some(Null)` indicates the fields exist in the JSON structure but have null values, not that the fields are missing.

## Proposed Investigation

1. Add debug logging in `worker/src/activities/postgres.rs` to verify query results before serialization
2. Add debug logging in `core/src/workflow/template.rs` to trace value conversions
3. Add debug logging in orchestrator activity output storage/retrieval
4. Create integration test that verifies field values survive the activity output → template → activity input pipeline

## Related Issues

- `2026-01-05-postgres-array-params-unsupported.md` - Related postgres_query limitations (input side)
- This bug affects the **output** side of postgres_query

## Files to Investigate

- `worker/src/activities/postgres.rs` - Query result serialization
- `core/src/workflow/template.rs` - Template value conversion functions
- `core/src/orchestrator/orchestrator.rs` - Activity output handling
- `core/src/orchestrator/workflow_state.rs` - How outputs are stored

## Workarounds Attempted

1. **Custom worker activity** - Failed because the custom worker receives the same null values
2. **Template filters** - No filter can recover null values
3. **Different field selection** - Issue affects all non-string types regardless of column names

## Resolution

### Root Cause

The bug was in `worker/src/activities/postgres.rs` in the `row_to_json` function (lines 182-214). The function had two problems:

1. **Missing UUID type handling**: There was no `try_get::<uuid::Uuid>` call, so UUID columns fell through to the default `Value::Null` case.

2. **Missing nullable type handling**: The code only tried non-nullable type variants (e.g., `try_get::<i32>`). For PostgreSQL columns that are nullable (defined without `NOT NULL`), sqlx requires using `Option<T>` even when the current value is non-null. When the code tried `try_get::<i32>` on a nullable integer column, it failed and fell through to `Value::Null`.

### Fix Applied

Updated `row_to_json` to:

1. Add UUID type handling with `try_get::<uuid::Uuid>` and `try_get::<Option<uuid::Uuid>>`
2. For each type, try both non-nullable and nullable variants in sequence
3. Added support for additional integer sizes (SMALLINT via `i16`, BIGINT via `i64`)
4. Added support for REAL via `f32` in addition to DOUBLE PRECISION via `f64`

### Files Changed

- `worker/src/activities/postgres.rs`: Fixed `row_to_json` function

### Tests Added

**Unit Tests (no database required):**

These tests verify the `StatementResult` serialization contract that downstream activities depend on:

- `test_statement_result_serialization_with_rows` - Verifies rows serialize correctly with UUIDs, strings, integers
- `test_statement_result_rows_affected` - Verifies INSERT/UPDATE/DELETE output format
- `test_statement_result_uuid_as_string` - Regression test: UUIDs are strings, not null
- `test_statement_result_integers_as_numbers` - Regression test: Integers are numbers, not null
- `test_statement_result_nullable_values` - Verifies NULL is JSON null, not missing field
- `test_statement_result_mixed_types` - Reproduces exact bug scenario with all types
- `test_statement_result_roundtrip` - Verifies JSON roundtrip preserves all types
- `test_statement_result_empty_rows` - Empty rows array edge case
- `test_statement_result_jsonb_values` - JSONB columns preserve structure

**Integration Tests (database required):**

- `test_postgres_query_uuid_columns`: Verifies non-nullable UUID columns serialize correctly
- `test_postgres_query_nullable_uuid`: Verifies nullable UUID columns handle both values and NULLs
- `test_postgres_query_nullable_integer`: Verifies nullable integer columns work correctly
- `test_postgres_query_mixed_types`: Reproduces the exact bug scenario with UUID, integer, float, string, and boolean columns
- `test_postgres_query_smallint_bigint`: Verifies SMALLINT and BIGINT column support
