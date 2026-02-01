# Feature: `coalesce` Template Filter

**Date**: 2026-01-10
**Status**: Implemented
**Component**: Core / Template Engine

## Summary

Added a custom `coalesce` filter to the minijinja template engine that handles both undefined and null/None values. Unlike the built-in `default` filter which only handles undefined values (per Jinja2 semantics), `coalesce` provides SQL-like null handling that's more intuitive when working with database results.

## Motivation

The built-in `default` filter in minijinja follows Jinja2 semantics where it only provides a fallback for **undefined** values, not for **null/None** values. This is often surprising when working with database results where NULL fields need fallback handling.

**Problem scenario:**
```yaml
- key: check_source
  worker: std
  activity_name: postgres_query
  parameters:
    query: "SELECT doi FROM sources WHERE id = $1"
    # Returns: {rows: [{doi: null}]} when DOI is not set

- key: fetch_openalex
  depends_on:
    - activity_key: check_source
      # This fails! default doesn't handle null, so length fails on null
      condition: "{{check_source.result.rows[0].doi | default('') | length > 0}}"
```

Error: `invalid operation: cannot calculate length of value of type none`

## Solution

Added a `coalesce` filter that handles both undefined AND null values:

```yaml
# Works correctly with both undefined and null values:
condition: "{{check_source.result.rows[0].doi | coalesce('') | length > 0}}"
```

## Usage

Replace `| default(value)` with `| coalesce(value)` when you need to handle null values:

```yaml
# String fallback
"{{data.name | coalesce('Unknown')}}"

# Numeric fallback
"{{data.count | coalesce(0)}}"

# Boolean fallback
"{{data.active | coalesce(false)}}"

# Array fallback
"{{data.items | coalesce([])}}"
```

## Behavior Comparison

| Value       | `\| default('x')` | `\| coalesce('x')` |
|-------------|-------------------|---------------------|
| `undefined` | `'x'`             | `'x'`               |
| `null`      | `null` (unchanged)| `'x'`               |
| `""`        | `""`              | `""`                |
| `0`         | `0`               | `0`                 |
| `false`     | `false`           | `false`             |
| `"hello"`   | `"hello"`         | `"hello"`           |

Note: Unlike some "falsy" coalesce implementations, this `coalesce` only replaces **undefined** and **null** - it does NOT replace empty strings, zero, or false.

## Implementation

File: `core/src/workflow/template.rs`

```rust
fn create_template_env() -> Environment<'static> {
    let mut env = Environment::new();
    env.set_undefined_behavior(minijinja::UndefinedBehavior::Strict);

    env.add_filter("coalesce", |value: MiniValue, default: MiniValue| {
        if value.is_undefined() || value.is_none() {
            default
        } else {
            value
        }
    });

    env
}
```

## Test Cases

Tests in `core/src/workflow/template.rs`:

1. `test_coalesce_filter_on_null_value` - Coalesce replaces null with fallback
2. `test_coalesce_filter_with_length` - Exact use case: `{{value | coalesce('') | length}}`
3. `test_coalesce_filter_on_undefined_value` - Also handles undefined (like default)
4. `test_coalesce_filter_on_non_null_value` - Passes through non-null values unchanged
5. `test_coalesce_filter_preserves_type` - Preserves number, boolean, array types
6. `test_coalesce_vs_default_on_null` - Demonstrates difference from default filter
7. `test_coalesce_with_zero_and_empty_string` - Zero and empty string are NOT replaced
8. `test_coalesce_in_condition_expression` - Full condition pattern

## Design Notes

- Named `coalesce` to align with SQL's `COALESCE()` function which users working with databases will recognize
- Does NOT replace "falsy" values like `0`, `""`, or `false` - only `undefined` and `null`
- The existing `default` filter is preserved for Jinja2 compatibility
- Types are preserved through the filter (number, boolean, array, etc.)
