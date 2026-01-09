# Bug: minijinja `default` Filter Doesn't Handle null/None Values

**Date**: 2026-01-08
**Status**: Open
**Severity**: Medium
**Component**: Core / Template Engine / minijinja

## Summary

The `default` filter in minijinja only applies to undefined values, not to explicit null/None values. This causes errors when template expressions encounter database NULL values that need fallback handling.

## Symptoms

```yaml
- key: check_source
  worker: builtin
  activity_name: postgres_query
  parameters:
    query: "SELECT doi FROM sources WHERE id = $1"
    # Returns: {rows: [{doi: null}]} when DOI is not set

- key: fetch_openalex
  depends_on:
    - activity_key: check_source
      condition: "{{check_source.result.rows[0].doi | default('') | length > 0}}"
```

Expected behavior:
- When `doi` is NULL, `| default('')` returns empty string
- `| length` returns 0
- Condition evaluates to false

Actual behavior:
- `doi` is null (explicit None value)
- `| default('')` passes through null (only applies to undefined)
- `| length` fails on null:
  ```
  invalid operation: cannot calculate length of value of type none
  ```

## Root Cause

minijinja's `default` filter follows Jinja2 semantics where it only provides a fallback for **undefined** values, not for **null/None** values. This is technically correct behavior per Jinja2 spec, but often surprising when working with database results.

In Jinja2/minijinja:
- `undefined | default('x')` → `'x'`
- `null | default('x')` → `null` (not undefined, so default doesn't apply)

## Impact

- Template expressions accessing database fields that might be NULL fail unexpectedly
- Users must handle NULL at the data source level or use complex template workarounds
- Common pattern `{{value | default('') | length}}` doesn't work for NULL values

## Suggested Fix

Option 1: Add a custom filter like `coalesce` that handles both undefined and null:
```rust
env.add_filter("coalesce", |value: Value, default: Value| {
    if value.is_undefined() || value.is_none() {
        default
    } else {
        value
    }
});
```

Option 2: Add `default(value, true)` support where second arg means "also replace falsy/null":
```
{{doi | default('', true)}}  # Replaces both undefined and null
```

Option 3: Document the limitation and recommend SQL-level handling

## Workaround

Handle NULL values at the data source level using SQL COALESCE:

```yaml
- key: check_source
  parameters:
    query: |
      SELECT COALESCE(doi, '') as doi
      FROM sources WHERE id = $1
```

Or use complex template expressions:
```yaml
condition: "{{check_source.result.rows[0].doi if check_source.result.rows[0].doi is not none else '' | length > 0}}"
```

## Test Cases Needed

1. Template with `| default()` on null value
2. Template with `| default()` on undefined value (should work)
3. Template with nested object access where intermediate is null
4. postgres_query result with NULL column value

## Notes

This is technically "working as designed" per Jinja2 semantics, but is a common source of confusion. The fix should either:
- Add a variant filter that handles null
- Document the limitation prominently
- Consider whether kruxiaflow's use case warrants different default behavior
