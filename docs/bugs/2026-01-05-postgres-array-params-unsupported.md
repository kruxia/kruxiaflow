# Bug: postgres_query and postgres_transaction Don't Support Array/Object Parameters

**Date**: 2026-01-05
**Status**: Resolved
**Severity**: Medium
**Component**: Worker / Postgres Activities
**Resolution Date**: 2026-01-06

## Summary

The `postgres_query` and `postgres_transaction` activities only support scalar parameter types (String, Number, Boolean, Null). Array and Object parameters fail with "Unsupported parameter type" error, making it impossible to pass JSON data to PostgreSQL queries.

## Symptoms

When passing an array or object as a parameter:

```yaml
parameters:
  db_url: "{{INPUT.db_url}}"
  query: "SELECT * FROM jsonb_array_elements($1::jsonb)"
  params:
    - "{{some_activity.array_output}}"
```

The activity fails with:
```
WARN kruxiaflow_worker::poller: Activity execution failed
     activity_id=...
     error=Unsupported parameter type: [{"field":"value"}, ...]
```

## Root Cause

In `worker/src/activities/postgres.rs`, the `execute_statement` function only handles scalar types:

```rust
// Lines 229-251
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
```

The `_` catch-all rejects `Value::Array` and `Value::Object`.

## Impact

- Cannot use `jsonb_array_elements()` or similar PostgreSQL JSON functions with dynamic data
- Cannot pass complex data structures to stored procedures
- Workaround requires creating custom activities for any workflow needing JSON parameters

## Workaround

Create a custom activity in an external worker that handles the database operations directly, bypassing the builtin postgres_query activity.

## Proposed Fix

Serialize Array and Object values to JSON strings before binding:

```rust
Value::Array(arr) => {
    let json_str = serde_json::to_string(&Value::Array(arr.clone()))
        .map_err(|e| anyhow::anyhow!("Failed to serialize array: {}", e))?;
    query.bind(json_str)
}
Value::Object(obj) => {
    let json_str = serde_json::to_string(&Value::Object(obj.clone()))
        .map_err(|e| anyhow::anyhow!("Failed to serialize object: {}", e))?;
    query.bind(json_str)
}
```

This would allow SQL like:
```sql
SELECT * FROM jsonb_array_elements($1::jsonb)
```

Where `$1` receives the JSON string representation of the array, and PostgreSQL's `::jsonb` cast parses it.

## Alternative Fix

Add explicit JSON parameter type support with a wrapper:

```yaml
params:
  - type: json
    value: "{{some_activity.array_output}}"
```

This would provide more explicit control over parameter serialization.

## Files Affected

- `worker/src/activities/postgres.rs` - `execute_statement` function (line ~229)

## Test Cases Needed

1. Array parameter with `jsonb_array_elements()`
2. Object parameter with JSONB column insert
3. Nested array/object structures
4. Large JSON payloads
5. JSON containing special characters (quotes, newlines, unicode)

## Investigation Notes: The `| tojson` Double-Encoding Problem

### Observed Behavior

When using the `| tojson` filter to convert arrays to strings:

```yaml
params:
  - "{{extract_content.passages | tojson}}"
```

**Expected**: PostgreSQL receives string `[{"sequence":1,...}]`, parses as JSON array
**Actual**: PostgreSQL receives string `"[{\"sequence\":1,...}]"` (with outer quotes), parses as JSON string scalar

Error message: `ERROR: cannot extract elements from a scalar`

### Theoretical Flow (Should Work)

1. `extract_content.passages` → minijinja sequence (array)
2. `| tojson` → minijinja String containing `[{"sequence":1,...}]`
3. `minijinja_to_serde_json()` → `Value::String("[{...}]")`
4. Postgres activity binds string to query
5. PostgreSQL `$2::jsonb` parses string as JSON array

### Where Double-Encoding May Occur

**Suspect 1: `minijinja_to_serde_json()` conversion**
```rust
// core/src/workflow/template.rs
minijinja::value::ValueKind::String => Value::String(value.to_string()),
```
- `value.to_string()` uses Display trait
- For minijinja String, should return content without quotes
- But need to verify minijinja's behavior for `tojson` output specifically

**Suspect 2: Minijinja's HTML-safe tojson**
From minijinja docs:
> The resulting value is safe to use in HTML as well as it will not contain any special HTML characters.

- `tojson` may wrap result in a special "safe string" type
- This type's Display impl might have different behavior
- Could add escaping or quoting for HTML safety

**Suspect 3: Activity parameter serialization**
- Parameters are serialized to JSON when queued
- `Value::String("[{...}]")` serializes as `"[{...}]"` (quoted JSON string)
- Deserialization should restore `Value::String("[{...}]")`
- But if there's a double-serialization somewhere, could get `"\"[{...}]\""`

**Suspect 4: Template expression evaluation**
- `is_whole_template()` detects `"{{...}}"` patterns
- Evaluates expression and preserves type
- If `tojson` output is treated as needing further encoding, double-encoding occurs

### Debugging Steps Needed

1. Add logging in `minijinja_to_serde_json()` to print:
   - Input value kind
   - Input value debug representation
   - Output serde_json::Value

2. Add logging in `resolve_template_value()` to print:
   - Template expression being evaluated
   - Result after evaluation

3. Add logging in postgres activity to print:
   - Raw parameter values before binding
   - Exact string content for String parameters

4. Test with simple array:
   ```yaml
   params:
     - "{{[1, 2, 3] | tojson}}"
   ```
   This isolates the issue from activity output handling.

### Why Custom Activity Works

The custom `store.passages` activity bypasses all template/serialization issues:

1. Receives `passages` as native `Vec<PassageInput>` (deserialized by serde)
2. Receives `embeddings` as native `Vec<Vec<f64>>`
3. Directly binds values to SQLx query with proper types
4. No JSON string conversion needed for PostgreSQL

This confirms the issue is in the template → postgres_query pipeline, not in the data itself.

## Related Issues

- Also affects `postgres_transaction` activity which uses the same `execute_statement` function
- Related to template `| tojson` filter behavior (see `2026-01-04-secrets-not-loaded.md`)

## Resolution

### Fix Applied

Updated `execute_statement` function in `worker/src/activities/postgres.rs` (lines 313-319) to handle `Value::Array` and `Value::Object` by serializing them to JSON strings:

```rust
Value::Array(_) | Value::Object(_) => {
    // Serialize arrays and objects as JSON strings for PostgreSQL JSONB
    // Use in SQL with ::jsonb cast, e.g.: SELECT * FROM jsonb_array_elements($1::jsonb)
    let json_str = serde_json::to_string(param)
        .map_err(|e| anyhow::anyhow!("Failed to serialize JSON parameter: {}", e))?;
    query.bind(json_str)
}
```

### Usage

To use array or object parameters, cast the parameter to `jsonb` in your SQL:

```yaml
parameters:
  db_url: "{{INPUT.db_url}}"
  query: "SELECT * FROM jsonb_array_elements($1::jsonb)"
  params:
    - "{{some_activity.array_output}}"
```

### Files Changed

- `worker/src/activities/postgres.rs`: Added Array/Object handling in `execute_statement` function

### Tests Added

New unit tests in `worker/src/activities/postgres.rs`:

- `test_postgres_query_array_parameter`: Verifies array parameters work with `jsonb_array_elements()`
- `test_postgres_query_object_parameter`: Verifies object parameters work with JSONB column insert
- `test_postgres_query_nested_json_parameter`: Verifies deeply nested structures
- `test_postgres_query_json_special_characters`: Verifies special characters (quotes, newlines, unicode)
- `test_postgres_query_array_of_objects_parameter`: Reproduces the exact bug scenario from the report
- `test_postgres_transaction_json_parameters`: Verifies JSON parameters work in transactions
