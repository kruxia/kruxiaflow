# Typed Query Parameters for PostgreSQL Activities

## Summary

Support explicit type annotations in PostgreSQL activity query parameters, allowing workflow authors to declare parameter types (decimal, uuid, timestamp, etc.) rather than relying on implicit inference or SQL-side casting.

## Motivation

PostgreSQL activity parameters arrive as `serde_json::Value`, which has a limited type system (string, number, bool, null, array, object). Some PostgreSQL types — notably `NUMERIC`, `UUID`, and `TIMESTAMP` — require values that JSON represents as plain strings.

The current approach binds all string parameters as `TEXT` and relies on SQL-side casting (e.g., `$1::numeric`) to convert. This works well and is the PostgreSQL-idiomatic approach, but typed parameters would provide:

1. **Explicitness**: The workflow definition declares intent, reducing reliance on SQL-side casts
2. **Error clarity**: Type mismatches are caught at bind time with clear error messages
3. **Extensibility**: New types can be added without changing the SQL

## Proposed Format

Plain values use default inference (current behavior). Wrapped objects with a `$type` key get explicit binding:

```yaml
activities:
  insert-price:
    activity_type: postgres_query
    params:
      query: "INSERT INTO prices (product, amount, id) VALUES ($1, $2, $3)"
      params:
        - "Widget"
        - { $decimal: "0.00053" }
        - { $uuid: "550e8400-e29b-41d4-a716-446655440000" }
```

Equivalent JSON:

```json
{
  "params": [
    "Widget",
    {"$decimal": "0.00053"},
    {"$uuid": "550e8400-e29b-41d4-a716-446655440000"}
  ]
}
```

## Supported Types

| Wrapper Key    | Rust Type              | PostgreSQL Type    |
|----------------|------------------------|--------------------|
| `$decimal`     | `rust_decimal::Decimal` | `NUMERIC/DECIMAL` |
| `$uuid`        | `uuid::Uuid`           | `UUID`             |
| `$timestamp`   | `chrono::DateTime`     | `TIMESTAMPTZ`      |
| `$date`        | `chrono::NaiveDate`    | `DATE`             |
| `$bytea`       | `Vec<u8>` (base64)    | `BYTEA`            |

## Implementation Sketch

In the `execute_statement` parameter binding loop, detect single-key objects with a `$`-prefixed key:

```rust
Value::Object(map) if map.len() == 1 => {
    if let Some((key, val)) = map.iter().next() {
        match key.as_str() {
            "$decimal" => {
                let d: Decimal = val.as_str()
                    .ok_or_else(|| anyhow!("$decimal value must be a string"))?
                    .parse()?;
                query.bind(d)
            }
            "$uuid" => {
                let u: Uuid = val.as_str()
                    .ok_or_else(|| anyhow!("$uuid value must be a string"))?
                    .parse()?;
                query.bind(u)
            }
            // ... other typed wrappers
            _ => {
                // No $-prefix match: serialize as JSONB (existing behavior)
                let json_str = serde_json::to_string(param)?;
                query.bind(json_str)
            }
        }
    }
}
```

## Backward Compatibility

Fully backward compatible. Existing parameters without `$`-prefix wrappers continue to work identically. The `$`-prefix convention avoids collision with normal JSON object keys.

## Alternative: SQL-Side Casting

For most use cases, SQL-side casting (`$1::numeric`) is sufficient and requires no changes to the parameter format. Typed parameters are an ergonomic enhancement, not a replacement.
