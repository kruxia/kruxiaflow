# Bug: http_request Activity Doesn't Decompress Gzip Responses

**Date**: 2026-01-07
**Status**: Open
**Severity**: Medium
**Component**: Worker / HTTP Request Activity

## Summary

The `http_request` builtin activity sends `Accept-Encoding: gzip` by default (via the underlying HTTP client), but does not decompress gzip-compressed responses before storing them in the database. This causes PostgreSQL to reject the workflow event because gzip binary data contains null bytes (`\u0000`) which cannot be stored in JSON text fields.

## Symptoms

When making an HTTP request to a server that returns gzip-compressed content:

```yaml
- key: lookup_doi_org
  worker: builtin
  activity_name: http_request
  parameters:
    method: GET
    url: "https://doi.org/10.1093/mind/fzab057"
    headers:
      Accept: "application/vnd.citationstyles.csl+json"
```

The activity completes but fails when kruxiaflow tries to store the result:

```
ERROR kruxiaflow_api::error: Internal error: InternalError(Event source error: Database error:
  error returned from database: unsupported Unicode escape sequence

postgres-1  | ERROR:  unsupported Unicode escape sequence
postgres-1  | DETAIL:  \u0000 cannot be converted to text.
postgres-1  | CONTEXT:  JSON data, line 1: ...,"outputs":{"response":{"body":"\u001f�\b\u0000...
```

The byte sequence `\u001f�\b\u0000` is the gzip magic number (0x1f 0x8b 0x08 0x00), confirming the response body is raw gzip data.

## Root Cause

The HTTP client (likely `reqwest`) sends `Accept-Encoding: gzip, deflate` by default when the `gzip` feature is enabled. Many servers (including doi.org) honor this and return compressed responses. However, the `http_request` activity stores the raw response body without checking `Content-Encoding` and decompressing if necessary.

## Impact

- HTTP requests to any server that returns gzip content will fail to store results
- doi.org, CrossRef, and many other APIs compress responses by default
- Workflows using `http_request` to fetch external data are unreliable

## Workaround

Add `Accept-Encoding: identity` header to HTTP requests to disable compression:

```yaml
- key: lookup_doi_org
  worker: builtin
  activity_name: http_request
  parameters:
    method: GET
    url: "https://doi.org/10.1093/mind/fzab057"
    headers:
      Accept: "application/vnd.citationstyles.csl+json"
      Accept-Encoding: "identity"  # Disable gzip
```

This tells the server not to compress the response.

## Proposed Fix

Option 1: Auto-decompress gzip responses in the `http_request` activity:

```rust
// After receiving response
let body = if response.headers()
    .get("content-encoding")
    .map(|v| v.to_str().unwrap_or(""))
    .unwrap_or("")
    .contains("gzip")
{
    use flate2::read::GzDecoder;
    let bytes = response.bytes().await?;
    let mut decoder = GzDecoder::new(&bytes[..]);
    let mut decompressed = String::new();
    decoder.read_to_string(&mut decompressed)?;
    decompressed
} else {
    response.text().await?
};
```

Option 2: Configure reqwest to auto-decompress (if not already):

```rust
let client = reqwest::Client::builder()
    .gzip(true)  // Enable automatic gzip decompression
    .build()?;
```

With `gzip(true)`, reqwest automatically decompresses responses and removes the `Content-Encoding` header, making this transparent to callers.

Option 3: Don't send `Accept-Encoding` header by default:

```rust
let client = reqwest::Client::builder()
    .no_gzip()  // Disable gzip in Accept-Encoding
    .build()?;
```

## Recommended Fix

Option 2 (auto-decompression) is the best solution because:
- It's the most user-friendly (transparent decompression)
- It preserves bandwidth savings from compression
- It matches typical HTTP client behavior (browsers, curl, etc.)
- No workflow changes required

## Files Affected

- Location of `http_request` activity implementation (likely `worker/src/activities/http.rs` or similar)

## Test Cases Needed

1. Request to server returning gzip-compressed JSON
2. Request to server returning uncompressed JSON
3. Request to server returning gzip-compressed HTML
4. Request with explicit `Accept-Encoding: identity`
5. Request with explicit `Accept-Encoding: gzip`
6. Binary response (should work regardless)

## Related Issues

- PostgreSQL JSON/JSONB columns cannot store null bytes
- Similar issues may occur with deflate or brotli compression
