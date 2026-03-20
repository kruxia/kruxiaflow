/// MCP Execution Tools
///
/// Three tools that create or modify workflow state:
/// - validate_workflow: parse + validate YAML/JSON in-process (no DB)
/// - submit_workflow: deploy definition (if needed) and submit a workflow instance
/// - cancel_workflow: cancel a running workflow (stub — endpoint not yet implemented)
use rust_mcp_sdk::macros::{JsonSchema, mcp_tool};
use rust_mcp_sdk::schema::{CallToolResult, schema_utils::CallToolError};
use rust_mcp_sdk::tool_box;
use sqlx::PgPool;

use super::{AnyJson, error_response, parse_uuid, text_response};

// ============================================================================
// Tool: validate_workflow
// ============================================================================

#[mcp_tool(
    name = "validate_workflow",
    description = "Validate a workflow definition (YAML or JSON) without deploying it.\n\
        \n\
        Parses the definition, checks structure, validates activity keys, resolves \
        dependencies, and detects cycles. Returns a detailed report: which activities \
        were found, the full dependency map, and any validation errors.\n\
        \n\
        When to use: Before submitting a workflow. Catches errors like undefined \
        dependencies, duplicate keys, missing required fields, and circular \
        dependencies — all without touching the database.",
    read_only_hint = true,
    idempotent_hint = true
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct ValidateWorkflow {
    /// The workflow definition as a YAML or JSON string
    pub workflow_yaml: String,
}

impl ValidateWorkflow {
    pub fn call_tool(&self) -> Result<CallToolResult, CallToolError> {
        // Attempt to parse + validate. from_yaml handles both YAML and JSON,
        // calls validate() and normalize() internally.
        match kruxiaflow_core::WorkflowDefinition::from_yaml(&self.workflow_yaml) {
            Ok(definition) => {
                // Build dependency map: activity_key -> [keys it depends on]
                let deps: serde_json::Value = definition
                    .activities
                    .iter()
                    .map(|a| {
                        let dep_keys: Vec<&str> = a
                            .depends_on
                            .as_ref()
                            .map(|ds| ds.iter().map(|d| d.activity_key.as_str()).collect())
                            .unwrap_or_default();
                        (a.key.clone(), serde_json::json!(dep_keys))
                    })
                    .collect::<serde_json::Map<String, serde_json::Value>>()
                    .into();

                let response = serde_json::json!({
                    "valid": true,
                    "errors": [],
                    "warnings": [],
                    "activities": definition.activities.len(),
                    "dependencies": deps,
                });

                text_response(&response)
            }
            Err(err) => {
                // Extract error strings from the ValidationError
                let errors = extract_validation_errors(&err);

                // Try to extract a partial dependency map if the YAML at least parsed
                // into activities (best-effort — return empty if not possible)
                let (activity_count, deps) = extract_partial_info(&self.workflow_yaml);

                let response = serde_json::json!({
                    "valid": false,
                    "errors": errors,
                    "warnings": [],
                    "activities": activity_count,
                    "dependencies": deps,
                });

                text_response(&response)
            }
        }
    }
}

/// Wrapper for ValidateWorkflow — uniform run_* dispatch pattern.
pub fn run_validate_workflow(params: &ValidateWorkflow) -> Result<CallToolResult, CallToolError> {
    params.call_tool()
}

// ============================================================================
// Tool: submit_workflow
// ============================================================================

#[mcp_tool(
    name = "submit_workflow",
    description = "Submit a workflow for execution.\n\
        \n\
        The workflow definition must already be deployed (use validate_workflow to check \
        it first, then deploy via the API or include it in your workflow authoring flow). \
        Provide the definition name, an input object, and optionally a version and \
        unique_key for idempotent submission.\n\
        \n\
        Returns immediately with a workflow_id — execution happens asynchronously. \
        Use get_workflow_status to monitor progress.\n\
        \n\
        When to use: After validating and deploying a workflow definition. The input \
        object must match what the workflow's template expressions expect.",
    destructive_hint = true,
    read_only_hint = false,
    idempotent_hint = false
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct SubmitWorkflow {
    /// Name of the deployed workflow definition to run
    pub definition_name: String,

    /// Specific version to run in YYYYmmdd.HHMMSS.uuuuuu format (optional — uses latest if omitted)
    pub version: Option<String>,

    /// Input payload — must be a JSON object. Values are available as {{INPUT.key}} in the workflow.
    pub input: AnyJson,

    /// Optional idempotency key. A second submission with the same key is rejected with a conflict error.
    pub unique_key: Option<String>,
}

// ============================================================================
// Tool: cancel_workflow
// ============================================================================

#[mcp_tool(
    name = "cancel_workflow",
    description = "Cancel a running workflow by its ID.\n\
        \n\
        NOTE: Workflow cancellation is not yet fully implemented in the Kruxia Flow \
        backend. This tool will return the workflow's current status so you can \
        assess its state, but the actual cancellation action is not yet available. \
        Monitor with get_workflow_status in the meantime.\n\
        \n\
        When to use: When a running workflow needs to be stopped. Check back when \
        the cancellation endpoint is available.",
    destructive_hint = true,
    read_only_hint = false,
    idempotent_hint = true
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct CancelWorkflow {
    /// UUID of the workflow to cancel
    pub workflow_id: String,

    /// Optional reason for cancellation (for audit logging when cancellation is supported)
    pub reason: Option<String>,
}

// ============================================================================
// Enum + routing glue
// ============================================================================

tool_box!(
    ExecutionTools,
    [ValidateWorkflow, SubmitWorkflow, CancelWorkflow]
);

// ============================================================================
// Async runners for DB-backed tools
// ============================================================================

/// Submit a workflow via WorkflowService.
pub async fn run_submit_workflow(
    pool: &PgPool,
    params: &SubmitWorkflow,
) -> Result<CallToolResult, CallToolError> {
    // Basic input validation before hitting the service
    if !params.input.0.is_object() {
        let response = serde_json::json!({
            "error": "Input must be a JSON object, not a scalar or array",
            "definition_name": params.definition_name,
        });
        return error_response(&response);
    }

    let service = kruxiaflow_core::workflow::WorkflowService::new(pool.clone());

    let result = service
        .submit_workflow(
            &params.definition_name,
            params.version.as_deref(),
            params.input.0.clone(),
            params.unique_key.clone(),
        )
        .await;

    match result {
        Ok(created) => {
            let response = serde_json::json!({
                "workflow_id": created.id.to_string(),
                "status": created.status,
                "definition_name": created.definition_name,
                "definition_version": created.definition_version,
                "submitted_at": created.created_at.to_rfc3339(),
            });
            text_response(&response)
        }
        Err(e) => {
            tracing::warn!("submit_workflow error: {e}");
            let response = match &e {
                kruxiaflow_core::workflow::WorkflowServiceError::DefinitionNotFound {
                    name,
                    version,
                } => serde_json::json!({
                    "error": format!(
                        "Workflow definition '{}' version '{}' not found. Deploy it first.",
                        name, version
                    ),
                    "definition_name": name,
                    "version": version,
                }),
                kruxiaflow_core::workflow::WorkflowServiceError::DefinitionNotFoundLatest {
                    name,
                } => serde_json::json!({
                    "error": format!(
                        "Workflow definition '{}' not found (no versions deployed). Deploy it first.",
                        name
                    ),
                    "definition_name": name,
                }),
                kruxiaflow_core::workflow::WorkflowServiceError::DuplicateSubmission(key) => {
                    serde_json::json!({
                        "error": format!(
                            "A workflow with unique_key '{}' was already submitted. Use a different key or omit unique_key.",
                            key
                        ),
                        "unique_key": key,
                    })
                }
                kruxiaflow_core::workflow::WorkflowServiceError::InvalidInput(msg) => {
                    serde_json::json!({
                        "error": format!("Invalid input: {}", msg),
                        "definition_name": params.definition_name,
                    })
                }
                _ => serde_json::json!({
                    "error": format!("Submission failed: {}", e),
                    "definition_name": params.definition_name,
                }),
            };
            error_response(&response)
        }
    }
}

/// Cancel workflow — stub that returns current status.
///
/// The cancel endpoint is not yet implemented in the Kruxia Flow API.
/// This queries the workflow status and returns it alongside the limitation notice.
pub async fn run_cancel_workflow(
    pool: &PgPool,
    params: &CancelWorkflow,
) -> Result<CallToolResult, CallToolError> {
    let workflow_id = parse_uuid(&params.workflow_id)?;

    let query_svc = kruxiaflow_core::workflow::WorkflowQueryService::new(pool.clone());

    match query_svc.get_workflow(workflow_id).await {
        Ok(workflow) => {
            let response = serde_json::json!({
                "error": "Workflow cancellation is not yet supported in this version of Kruxia Flow. \
                          The cancel endpoint is pending implementation. \
                          Monitor with get_workflow_status and wait for completion or failure.",
                "workflow_id": workflow.id.to_string(),
                "definition_name": workflow.definition_name,
                "current_status": workflow.status,
                "reason_provided": params.reason,
            });
            error_response(&response)
        }
        Err(kruxiaflow_core::workflow::WorkflowQueryError::WorkflowNotFound(_)) => {
            let response = serde_json::json!({
                "error": format!("Workflow '{}' not found", params.workflow_id),
                "workflow_id": params.workflow_id,
            });
            error_response(&response)
        }
        Err(e) => {
            tracing::error!("cancel_workflow query error: {e:?}");
            Err(CallToolError::from_message(format!(
                "Database error looking up workflow '{}': {e}",
                params.workflow_id
            )))
        }
    }
}

// ============================================================================
// Helpers
// ============================================================================

/// Flatten a ValidationError into a Vec of human-readable strings.
fn extract_validation_errors(err: &kruxiaflow_core::ValidationError) -> Vec<String> {
    match err {
        kruxiaflow_core::ValidationError::SingleError(msg) => vec![msg.clone()],
        kruxiaflow_core::ValidationError::MultipleErrors(errs) => {
            let mut messages = Vec::new();
            for (field, field_errors) in errs.errors() {
                for msg in field_errors {
                    messages.push(format!("{}: {}", field, msg));
                }
            }
            messages
        }
    }
}

/// Best-effort extraction of activity count and dependency map from raw YAML.
/// Returns (0, {}) if the YAML can't be parsed at all.
///
/// TODO(#11): Uses `serde_yaml` directly rather than going through
/// `WorkflowDefinition::from_yaml`, because the definition may have failed
/// validation — we still want to extract whatever structural info we can from
/// the raw document.
fn extract_partial_info(yaml: &str) -> (usize, serde_json::Value) {
    // Try to parse as YAML and extract activities — even if validation failed
    // the structure might be readable
    if let Ok(doc) = serde_yaml::from_str::<serde_json::Value>(yaml)
        && let Some(activities) = doc.get("activities").and_then(|a| a.as_object())
    {
        let count = activities.len();
        let deps: serde_json::Map<String, serde_json::Value> = activities
            .iter()
            .map(|(key, val)| {
                let dep_keys: Vec<String> = val
                    .get("depends_on")
                    .and_then(|d| d.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|item| {
                                // depends_on entries can be plain strings or objects with activity_key
                                item.as_str()
                                    .or_else(|| {
                                        item.get("activity_key").and_then(|v| v.as_str())
                                    })
                                    .map(|s| s.to_string())
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                (key.clone(), serde_json::json!(dep_keys))
            })
            .collect();
        return (count, serde_json::Value::Object(deps));
    }
    (0, serde_json::json!({}))
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // extract_partial_info tests
    // =========================================================================

    #[test]
    fn test_extract_partial_info_valid_yaml() {
        let yaml = r#"
name: test-workflow
activities:
  fetch:
    worker: std
    activity_name: http_request
    parameters:
      method: GET
      url: "https://example.com"
  process:
    worker: std
    activity_name: echo
    depends_on: [fetch]
"#;
        let (count, deps) = extract_partial_info(yaml);
        assert_eq!(count, 2);
        assert_eq!(deps["fetch"], serde_json::json!([]));
        assert_eq!(deps["process"], serde_json::json!(["fetch"]));
    }

    #[test]
    fn test_extract_partial_info_conditional_depends_on() {
        let yaml = r#"
name: test
activities:
  check:
    worker: std
    activity_name: echo
  handle:
    worker: std
    activity_name: echo
    depends_on:
      - activity_key: check
        conditions: ["check.status == ok"]
"#;
        let (count, deps) = extract_partial_info(yaml);
        assert_eq!(count, 2);
        assert_eq!(deps["handle"], serde_json::json!(["check"]));
    }

    #[test]
    fn test_extract_partial_info_invalid_yaml() {
        let (count, deps) = extract_partial_info("{{{{not yaml at all");
        assert_eq!(count, 0);
        assert_eq!(deps, serde_json::json!({}));
    }

    #[test]
    fn test_extract_partial_info_empty_string() {
        let (count, deps) = extract_partial_info("");
        assert_eq!(count, 0);
        assert_eq!(deps, serde_json::json!({}));
    }

    #[test]
    fn test_extract_partial_info_yaml_without_activities() {
        let (count, deps) = extract_partial_info("name: test\nversion: 1");
        assert_eq!(count, 0);
        assert_eq!(deps, serde_json::json!({}));
    }

    // =========================================================================
    // validate_workflow tests
    // =========================================================================

    #[test]
    fn test_validate_workflow_valid_definition() {
        let tool = ValidateWorkflow {
            workflow_yaml: r#"
name: test-workflow
activities:
  - key: echo_step
    worker: std
    activity_name: echo
    parameters:
      message: hello
"#
            .to_string(),
        };

        let result = tool.call_tool().unwrap();
        let content = &result.content[0];
        {
            let text = content.as_text_content().unwrap();
            let parsed: serde_json::Value = serde_json::from_str(&text.text).unwrap();
            assert_eq!(parsed["valid"], true);
            assert_eq!(parsed["activities"], 1);
            assert!(parsed["errors"].as_array().unwrap().is_empty());
        }
    }

    #[test]
    fn test_validate_workflow_invalid_definition() {
        let tool = ValidateWorkflow {
            workflow_yaml: r#"
name: test-workflow
activities:
  - key: step_a
    worker: std
    activity_name: echo
    depends_on: [nonexistent]
"#
            .to_string(),
        };

        let result = tool.call_tool().unwrap();
        let content = &result.content[0];
        {
            let text = content.as_text_content().unwrap();
            let parsed: serde_json::Value = serde_json::from_str(&text.text).unwrap();
            assert_eq!(parsed["valid"], false);
            assert!(!parsed["errors"].as_array().unwrap().is_empty());
        }
    }
}
