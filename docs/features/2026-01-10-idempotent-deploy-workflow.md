# Idempotent Workflow Deployment

**Date**: 2026-01-10
**Status**: Implemented
**Priority**: Medium
**Implemented**: 2026-01-14

## Problem Statement

Currently, every `POST /api/v1/workflow_definitions` creates a new version with an auto-generated timestamp, even if the workflow definition is identical to the currently deployed version. This leads to:

1. **Version bloat**: Repeated deployments (e.g., from CI/CD pipelines) create many duplicate versions
2. **Unnecessary database writes**: Identical definitions are stored multiple times
3. **Confusing version history**: Hard to identify which versions contain actual changes
4. **Wasted storage**: PostgreSQL stores redundant JSONB definitions

## Proposed Solution

Make workflow deployment idempotent: if the POSTed definition matches the currently deployed (latest) version, return the existing version instead of creating a new one.

### Behavior

| Scenario                                      | HTTP Status | Response                        |
|-----------------------------------------------|-------------|---------------------------------|
| New workflow (no existing versions)           | 201 Created | New version created             |
| Definition differs from latest version        | 201 Created | New version created             |
| Definition identical to latest version        | 200 OK      | Existing version returned       |

### Comparison Logic

Two workflow definitions are considered identical if their **normalized content** is the same. Normalization includes:

1. **Exclude metadata fields**: `version`, `created_at` are not part of comparison
2. **Canonical JSON serialization**: Convert to JSON with sorted keys
3. **Compare hash**: SHA-256 hash of normalized JSON

```rust
fn definitions_are_equal(a: &WorkflowDefinition, b: &WorkflowDefinition) -> bool {
    // Compare name
    if a.name != b.name {
        return false;
    }

    // Normalize and hash activities (excluding version/created_at)
    let hash_a = compute_definition_hash(a);
    let hash_b = compute_definition_hash(b);

    hash_a == hash_b
}

fn compute_definition_hash(def: &WorkflowDefinition) -> String {
    // Create a normalized representation for hashing
    let normalized = json!({
        "name": def.name,
        "description": def.description,
        "activities": def.activities,  // Already sorted by key
        "inputs": def.inputs,
    });

    // Serialize with sorted keys for deterministic output
    let canonical = serde_json::to_string(&normalized).unwrap();

    // SHA-256 hash
    sha256_hex(&canonical)
}
```

### API Response Changes

**When returning existing version (200 OK)**:

```json
{
  "name": "my-workflow",
  "version": "20260110.143052.123456",
  "created_at": "2026-01-10T14:30:52.123456Z",
  "unchanged": true
}
```

The `unchanged: true` field indicates the definition was already deployed and no new version was created.

**When creating new version (201 Created)**:

```json
{
  "name": "my-workflow",
  "version": "20260110.150000.000000",
  "created_at": "2026-01-10T15:00:00.000000Z"
}
```

No `unchanged` field (or `unchanged: false`) indicates a new version was created.

## Implementation Plan

### Phase 1: Add Definition Hash Function

**Location**: `core/src/workflow/definition.rs`

Add a method to compute a canonical hash of a workflow definition:

```rust
impl WorkflowDefinition {
    /// Compute a SHA-256 hash of the normalized definition.
    /// Used for idempotent deployment comparison.
    pub fn content_hash(&self) -> String {
        use sha2::{Sha256, Digest};

        // Create normalized representation (excludes version, created_at)
        let normalized = serde_json::json!({
            "name": &self.name,
            "description": &self.description,
            "activities": &self.activities,
            "inputs": &self.inputs,
        });

        // Serialize deterministically
        let canonical = serde_json::to_string(&normalized)
            .expect("WorkflowDefinition should always serialize");

        // Compute hash
        let mut hasher = Sha256::new();
        hasher.update(canonical.as_bytes());
        let result = hasher.finalize();

        hex::encode(result)
    }
}
```

### Phase 2: Store Content Hash in Database

**Migration**: Add `content_hash` column to `workflow_definitions` table:

```sql
ALTER TABLE workflow_definitions
ADD COLUMN content_hash TEXT;

-- Backfill existing rows (optional, can be done lazily)
-- UPDATE workflow_definitions SET content_hash = ...

-- Add index for fast lookup
CREATE INDEX idx_workflow_definitions_name_hash
ON workflow_definitions(name, content_hash);
```

### Phase 3: Update Repository

**Location**: `core/src/workflow/repository.rs`

Add method to find by content hash:

```rust
impl WorkflowDefinitionRepository {
    /// Find a workflow definition by name and content hash.
    /// Returns the existing definition if found, None otherwise.
    pub async fn find_by_content_hash(
        &self,
        name: &str,
        content_hash: &str,
    ) -> Result<Option<StoredWorkflowDefinition>, RepositoryError> {
        let row = sqlx::query_as!(
            StoredWorkflowDefinitionRow,
            r#"
            SELECT id, name, version, definition, content_hash, created_at
            FROM workflow_definitions
            WHERE name = $1 AND content_hash = $2
            ORDER BY created_at DESC
            LIMIT 1
            "#,
            name,
            content_hash
        )
        .fetch_optional(&self.pool)
        .await?;

        row.map(TryInto::try_into).transpose()
    }
}
```

Update `store` method to compute and store hash:

```rust
pub async fn store(
    &self,
    definition: WorkflowDefinition,
) -> Result<StoredWorkflowDefinition, RepositoryError> {
    let content_hash = definition.content_hash();

    // Check if identical definition already exists
    if let Some(existing) = self.find_by_content_hash(&definition.name, &content_hash).await? {
        return Ok(existing);  // Return existing, don't create new
    }

    // Create new version (existing logic)
    let version = generate_version();
    // ... rest of store logic, including content_hash in INSERT
}
```

### Phase 4: Update API Handler

**Location**: `api/src/handlers/workflow_definitions.rs`

Update response to indicate whether definition was unchanged:

```rust
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct DeployWorkflowDefinitionResponse {
    pub name: String,
    pub version: String,
    pub created_at: chrono::DateTime<chrono::Utc>,

    /// True if definition was identical to existing version (no new version created)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unchanged: Option<bool>,
}

pub async fn deploy_workflow_definition(
    repo: WorkflowDefinitionRepository,
    Extension(claims): Extension<ValidatedClaims>,
    body: String,
) -> ApiResult<(StatusCode, Json<DeployWorkflowDefinitionResponse>)> {
    // ... parsing logic ...

    let (stored, is_new) = repo.store_idempotent(definition).await.map_err(/* ... */)?;

    let status = if is_new { StatusCode::CREATED } else { StatusCode::OK };
    let unchanged = if is_new { None } else { Some(true) };

    Ok((
        status,
        Json(DeployWorkflowDefinitionResponse {
            name: stored.name,
            version: stored.version,
            created_at: stored.created_at,
            unchanged,
        }),
    ))
}
```

## Alternative Approaches Considered

### Option A: Compare Latest Version Only

Compare incoming definition only against the latest deployed version.

**Pros:**
- Simpler implementation
- No new database column needed
- Fast (single query)

**Cons:**
- If user deploys A, then B, then A again → creates duplicate of A
- Doesn't prevent all duplicates

### Option B: Content Hash with Full History Search (Recommended)

Store content hash and check against all versions with same name.

**Pros:**
- Prevents any duplicate content, regardless of order
- Efficient with index on (name, content_hash)
- Can return any matching version

**Cons:**
- Requires database migration
- Slightly more complex

### Option C: Full Definition Comparison

Load latest version and compare field-by-field in memory.

**Pros:**
- No database changes
- Exact comparison

**Cons:**
- Expensive for large definitions
- Must handle JSON normalization carefully
- Slower than hash comparison

## Recommendation

**Option B (Content Hash)** is recommended because:
1. Efficient: Hash comparison is O(1)
2. Complete: Prevents all duplicates, not just sequential ones
3. Auditable: Hash stored in database for verification
4. Scalable: Index-backed lookup

## Testing Plan

1. **New workflow**: First deployment creates version, returns 201
2. **Changed workflow**: Modified definition creates new version, returns 201
3. **Unchanged workflow**: Identical definition returns existing version, returns 200
4. **Whitespace changes**: Verify normalization handles formatting differences
5. **Field ordering**: Verify definitions with reordered fields are considered equal
6. **Concurrent deploys**: Two identical deploys at same time both succeed (one creates, one returns existing)

## Migration Considerations

- Existing workflow definitions won't have `content_hash` initially
- Backfill can be done lazily (compute hash on first access) or via migration script
- New deployments always compute and store hash
- Old versions without hash are never matched (treated as unique)

## Dependencies

- `sha2` crate for SHA-256 hashing
- `hex` crate for hex encoding (likely already present)

## Estimated Effort

- Phase 1 (Hash function): 1 hour
- Phase 2 (Database migration): 1 hour
- Phase 3 (Repository update): 2 hours
- Phase 4 (API handler): 1 hour
- Testing: 2 hours

**Total: ~7 hours**

## Future Enhancements

1. **Diff endpoint**: `GET /api/v1/workflow_definitions/{name}/diff?from=v1&to=v2`
2. **Changelog**: Track what changed between versions
3. **Rollback**: `POST /api/v1/workflow_definitions/{name}/rollback?to=v1`
