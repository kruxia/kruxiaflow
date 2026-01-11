# Feature Request: regex_replace Jinja Filter

## Summary

Add a `regex_replace` filter to Kruxia Flow's Jinja template engine for advanced string manipulation in workflow definitions.

## Motivation

When working with file references from workflow storage, workflows need to transform paths between different formats. For example, transforming a file reference to an API URL:

```
postgres://workflow_id/activity_key/filename
```
to:
```
/api/v1/workflows/workflow_id/activities/activity_key/files/filename
```

Currently, this requires knowing the exact activity_key and filename in advance and using multiple `replace` calls. A `regex_replace` filter would enable generic path transformations.

## Proposed Syntax

```jinja
{{ value | regex_replace(pattern, replacement) }}
```

### Example Usage

```yaml
url: "{{INPUT.base_url}}{{file_ref | replace('postgres://', '/api/v1/workflows/') | regex_replace('/([^/]+)/([^/]+)$', '/activities/\\1/files/\\2')}}"
```

## Implementation Notes

MiniJinja (which Kruxia Flow uses) supports custom filters. The implementation would:

1. Add a custom filter function using the `regex` crate
2. Support capture group references (`\1`, `\2`, etc.) in the replacement string
3. Handle regex compilation errors gracefully

### Example Implementation

```rust
use minijinja::{Environment, Error, ErrorKind};
use regex::Regex;

fn regex_replace(value: &str, pattern: &str, replacement: &str) -> Result<String, Error> {
    let re = Regex::new(pattern)
        .map_err(|e| Error::new(ErrorKind::InvalidOperation, format!("invalid regex: {}", e)))?;
    Ok(re.replace_all(value, replacement).to_string())
}

// Register the filter
env.add_filter("regex_replace", regex_replace);
```

## Workaround

### Preferred: Use WORKFLOW.id with known values

When the activity_key and filename are known at workflow design time, use `WORKFLOW.id` directly:

```yaml
url: "{{INPUT.kruxiaflow_url}}/api/v1/workflows/{{WORKFLOW.id}}/activities/embed_query/files/embeddings.jsonl"
```

This is cleaner and avoids string parsing entirely.

### Alternative: Multiple replace calls

If you need to parse dynamic file references, use multiple `replace` calls:

```yaml
# Knowing activity_key=embed_query and filename=embeddings.jsonl
url: "{{base_url}}{{file_ref | replace('postgres://', '/api/v1/workflows/') | replace('/embed_query/embeddings.jsonl', '/activities/embed_query/files/embeddings.jsonl')}}"
```

## Related

- `split` filter would also be useful for path manipulation
- Consider adding `urlencode` filter for URL-safe encoding
