# Bug: Model Pricing HashMap Tuple Keys Cannot Be Serialized to JSON

**Date Discovered:** 2026-01-04
**Date Resolved:** 2026-01-04
**Severity:** High (blocks all LLM activity execution)
**Status:** Resolved

## Summary

The `batch_get_pricing` function returns a `HashMap<(String, String), ModelPricing>` with tuple keys. When the orchestrator attempts to serialize this HashMap to JSON for enriching LLM activity parameters, it fails with "key must be a string" because JSON only supports string keys.

## Error Message

```
ERROR run_orchestrator: kruxiaflow_core::orchestrator::orchestrator: Failed to process event
  event_id=... workflow_id=... event_type=ActivityCompleted
  error=Serialization error: key must be a string
```

## Root Cause

### Location
- **File:** `core/src/cost/calculator.rs:141`
- **Function:** `batch_get_pricing`

```rust
pub async fn batch_get_pricing(
    &self,
    models: &[(String, String)], // Vec of (provider, model)
) -> Result<HashMap<(String, String), ModelPricing>>  // <-- Tuple key!
```

### Serialization Failure
- **File:** `core/src/orchestrator/orchestrator.rs:1536`

```rust
obj.insert(
    "model_pricing".to_string(),
    serde_json::to_value(&model_pricing)?,  // Fails here
);
```

## Reproduction

1. Create a workflow with a custom worker activity followed by an LLM activity (`llm_prompt` or `embedding`)
2. Execute the workflow
3. When the custom worker activity completes successfully, the orchestrator tries to schedule the LLM activity
4. The `enrich_llm_activity_params_w_budget` function is called
5. Serialization of `model_pricing` fails

### Example Workflow (from researcher project)

```yaml
activities:
  - key: extract_content
    worker: researcher
    activity_name: pdf.extract
    # ... custom worker completes successfully

  - key: generate_embeddings
    worker: builtin
    activity_name: embedding  # <-- Triggers the bug when scheduling this
    parameters:
      model: google/gemini-embedding-001
      input: "{{extract_content.passages | map(attribute='content') | list}}"
    depends_on:
      - extract_content
```

## Proposed Fix

Convert the HashMap to use string keys before JSON serialization:

```rust
// In enrich_llm_activity_params_w_budget, around line 1536

// Convert tuple keys to string keys for JSON serialization
let model_pricing_json: HashMap<String, &ModelPricing> = model_pricing
    .iter()
    .map(|((provider, model), pricing)| (format!("{}/{}", provider, model), pricing))
    .collect();

obj.insert(
    "model_pricing".to_string(),
    serde_json::to_value(&model_pricing_json)?,
);
```

### Alternative Fix

Change the return type of `batch_get_pricing` to use string keys throughout:

```rust
pub async fn batch_get_pricing(
    &self,
    models: &[(String, String)],
) -> Result<HashMap<String, ModelPricing>>  // Use "provider/model" as key
```

This would require updating all call sites but provides a cleaner API.

## Impact

- **Blocked:** All workflows containing LLM activities (`llm_prompt`, `embedding`) after any other activity
- **Not Affected:** Workflows where LLM activities are the first activity (no prior ActivityCompleted event triggers budget enrichment)

## Workaround

None. The bug must be fixed in the Kruxia Flow codebase.

## Resolution

**Fix Applied:** Changed `batch_get_pricing` return type from `HashMap<(String, String), ModelPricing>` to `HashMap<String, ModelPricing>` using "provider/model" string format for keys.

**Changes Made:**
1. **core/src/cost/calculator.rs:142** - Updated return type to `HashMap<String, ModelPricing>`
2. **core/src/cost/calculator.rs:173** - Changed key format to `format!("{}/{}", provider, model)`
3. **core/src/orchestrator/orchestrator.rs:1482** - Updated code to use string keys when accessing pricing map
4. **core/tests/llm_budget_integration_tests.rs:95-111** - Updated existing test to use string keys
5. **core/tests/llm_budget_integration_tests.rs:114-167** - Added new test `test_batch_get_pricing_json_serialization` to prevent regression

**Test Coverage:**
- `test_cost_calculator_batch_get_pricing` - Verifies batch pricing retrieval with string keys (integration test)
- `test_batch_get_pricing_json_serialization` - Regression test that verifies JSON serialization works and can roundtrip (integration test)

**Unit Tests (no database required):**
- `test_model_pricing_json_serialization` - Verifies ModelPricing struct serializes to JSON
- `test_model_pricing_json_roundtrip` - Verifies ModelPricing can roundtrip through JSON
- `test_model_pricing_hashmap_string_keys_serializable` - Verifies HashMap<String, ModelPricing> serializes (the fix)
- `test_model_pricing_hashmap_tuple_keys_not_serializable` - Documents that tuple keys fail (the bug)
- `test_model_pricing_hashmap_string_keys_roundtrip` - Verifies HashMap<String, ModelPricing> roundtrips
- `test_model_pricing_key_format_provider_slash_model` - Verifies key format is "provider/model"

## Related Code

- `core/src/orchestrator/orchestrator.rs` - `enrich_llm_activity_params_w_budget` function (lines 1382-1558)
- `core/src/cost/calculator.rs` - `batch_get_pricing` function (lines 138-185)
- `core/src/cost/calculator.rs` - `ModelPricing` struct (lines 11-16)
- `core/tests/llm_budget_integration_tests.rs` - Test coverage for the fix
