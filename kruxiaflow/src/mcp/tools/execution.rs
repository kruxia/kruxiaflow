// The tool_box!-generated enum variants mirror the published MCP tool names,
// which all end in "Workflow"
#![allow(clippy::enum_variant_names)]

/// MCP Execution Tools
///
/// Three tools that create or modify workflow state:
/// - validate_workflow: parse + validate YAML/JSON in-process (no DB)
/// - deploy_workflow: validate + persist a workflow definition to the database
/// - submit_workflow: submit a workflow instance for execution, optionally with a
///   budget limit (definition must be deployed)
///
/// `cancel_workflow` is intentionally absent: the backend has no cancel endpoint
/// yet. Re-add the tool when workflow cancellation lands in the engine.
use rust_mcp_sdk::macros::{JsonSchema, mcp_tool};
use rust_mcp_sdk::schema::{CallToolResult, schema_utils::CallToolError};
use rust_mcp_sdk::tool_box;
use sqlx::PgPool;

use super::{ObjectJson, error_response, text_response};

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
                    "warnings": authoring_warnings(&definition, &self.workflow_yaml),
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
// Tool: deploy_workflow
// ============================================================================

#[mcp_tool(
    name = "deploy_workflow",
    description = "Deploy a workflow definition (YAML or JSON) to the database.\n\
        \n\
        Validates the definition first (same checks as validate_workflow), then \
        persists it as a new version. The version is auto-generated from the deployment \
        timestamp.\n\
        \n\
        Idempotent: if an identical definition (same name and content) is already \
        deployed, returns the existing version without creating a duplicate.\n\
        \n\
        When to use: After validating a workflow definition with validate_workflow \
        and before submitting instances with submit_workflow. This is the step that \
        makes the definition available for execution.",
    destructive_hint = false,
    read_only_hint = false,
    idempotent_hint = true
)]
#[derive(Debug, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct DeployWorkflow {
    /// The workflow definition as a YAML or JSON string
    pub workflow_yaml: String,
}

// ============================================================================
// Tool: submit_workflow
// ============================================================================

#[mcp_tool(
    name = "submit_workflow",
    description = "Submit a workflow for execution, optionally with a hard budget limit.\n\
        \n\
        The workflow definition must already be deployed (use validate_workflow to check \
        it first, then deploy_workflow to persist it). Provide the definition name, an \
        input object, and optionally a version and unique_key for idempotent submission.\n\
        \n\
        Set budget_limit_usd to cap total spend for this run: the engine enforces the \
        limit during execution (activities are aborted or downgraded to cheaper models \
        once the budget is exhausted). It overrides any budget in the definition's \
        settings. Use estimate_workflow_cost first to pick a sensible limit, and \
        get_workflow_cost afterwards to see actual spend against it.\n\
        \n\
        Returns immediately with a workflow_id — execution happens asynchronously. \
        Use get_workflow_status to monitor progress.\n\
        \n\
        When to use: After deploying a workflow definition with deploy_workflow. The \
        input object must match what the workflow's template expressions expect.",
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
    pub input: ObjectJson,

    /// Optional idempotency key. A second submission with the same key is rejected with a conflict error.
    pub unique_key: Option<String>,

    /// Optional hard budget limit in USD for this run, enforced by the engine during
    /// execution. Overrides the definition's settings.budget default.
    pub budget_limit_usd: Option<f64>,
}

// ============================================================================
// Enum + routing glue
// ============================================================================

tool_box!(
    ExecutionTools,
    [ValidateWorkflow, DeployWorkflow, SubmitWorkflow]
);

// ============================================================================
// Async runners for DB-backed tools
// ============================================================================

/// Deploy a workflow definition to the database.
///
/// Validates the YAML/JSON, then persists via WorkflowDefinitionRepository.
/// Idempotent: identical content returns the existing version.
pub async fn run_deploy_workflow(
    pool: &PgPool,
    params: &DeployWorkflow,
) -> Result<CallToolResult, CallToolError> {
    // Parse + validate the definition
    let definition = match kruxiaflow_core::WorkflowDefinition::from_yaml(&params.workflow_yaml) {
        Ok(def) => def,
        Err(err) => {
            let errors = extract_validation_errors(&err);
            let response = serde_json::json!({
                "error": "Workflow definition validation failed",
                "errors": errors,
            });
            return error_response(&response);
        }
    };

    let repo = kruxiaflow_core::workflow::WorkflowDefinitionRepository::new(pool.clone());

    let warnings = authoring_warnings(&definition, &params.workflow_yaml);

    match repo.store(definition).await {
        Ok(result) => {
            let response = serde_json::json!({
                "name": result.definition.name,
                "version": result.definition.version,
                "activity_count": result.definition.activities.len(),
                "created_at": result.definition.created_at.to_rfc3339(),
                "is_new": result.is_new,
                "unchanged": !result.is_new,
                "warnings": warnings,
            });
            text_response(&response)
        }
        Err(e) => {
            tracing::warn!("deploy_workflow error: {e}");
            let response = match &e {
                kruxiaflow_core::workflow::RepositoryError::ValidationError(ve) => {
                    let errors = extract_validation_errors(ve);
                    serde_json::json!({
                        "error": "Workflow definition validation failed",
                        "errors": errors,
                    })
                }
                kruxiaflow_core::workflow::RepositoryError::DuplicateVersion { name, version } => {
                    serde_json::json!({
                        "error": format!(
                            "Workflow definition '{}' version '{}' already exists (timestamp collision)",
                            name, version
                        ),
                        "name": name,
                        "version": version,
                    })
                }
                _ => serde_json::json!({
                    "error": format!("Failed to deploy workflow definition: {}", e),
                }),
            };
            error_response(&response)
        }
    }
}

/// Submit a workflow via WorkflowService.
pub async fn run_submit_workflow(
    pool: &PgPool,
    params: &SubmitWorkflow,
) -> Result<CallToolResult, CallToolError> {
    // Basic input validation before hitting the service. as_object also
    // coerces clients that pass the object as a JSON-encoded string.
    let Some(input) = params.input.as_object() else {
        let response = serde_json::json!({
            "error": "Input must be a JSON object, not a scalar or array",
            "definition_name": params.definition_name,
        });
        return error_response(&response);
    };

    let budget_limit_usd = match params.budget_limit_usd {
        Some(limit) => {
            let decimal = rust_decimal::Decimal::from_f64_retain(limit)
                .filter(|d| d.is_sign_positive() && !d.is_zero());
            if decimal.is_none() {
                return error_response(&serde_json::json!({
                    "error": "budget_limit_usd must be a positive number",
                    "budget_limit_usd": limit,
                    "definition_name": params.definition_name,
                }));
            }
            decimal
        }
        None => None,
    };

    let service = kruxiaflow_core::workflow::WorkflowService::new(pool.clone());

    let result = service
        .submit_workflow(
            &params.definition_name,
            params.version.as_deref(),
            input,
            params.unique_key.clone(),
            budget_limit_usd,
        )
        .await;

    match result {
        Ok(created) => {
            let response = serde_json::json!({
                "workflow_id": created.id.to_string(),
                "status": created.status,
                "definition_name": created.definition_name,
                "definition_version": created.definition_version,
                "budget_limit_usd": params.budget_limit_usd,
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

// ============================================================================
// Helpers
// ============================================================================

/// Warnings for authoring mistakes that pass validation but fail or surprise
/// at runtime. The definition parser ignores unknown fields, so an agent that
/// writes `type:`/`config:` instead of `activity_name:`/`parameters:` gets a
/// "valid" definition whose activities cannot execute — caught here instead.
fn authoring_warnings(
    definition: &kruxiaflow_core::WorkflowDefinition,
    raw_yaml: &str,
) -> Vec<String> {
    let mut warnings = Vec::new();

    // Unknown-field detection on the raw document (the typed parse has already
    // dropped them). Activities may be a map or a list of {key, ...} objects.
    let mut misnamed: Vec<(String, &'static str, &'static str)> = Vec::new();
    if let Ok(doc) = serde_yaml::from_str::<serde_json::Value>(raw_yaml)
        && let Some(activities) = doc.get("activities")
    {
        let entries: Vec<(String, &serde_json::Value)> = match activities {
            serde_json::Value::Object(map) => map.iter().map(|(k, v)| (k.clone(), v)).collect(),
            serde_json::Value::Array(arr) => arr
                .iter()
                .map(|v| {
                    let key = v
                        .get("key")
                        .and_then(|k| k.as_str())
                        .unwrap_or("?")
                        .to_string();
                    (key, v)
                })
                .collect(),
            _ => vec![],
        };
        for (key, entry) in entries {
            if entry.get("type").is_some() {
                misnamed.push((key.clone(), "type", "activity_name"));
            }
            if entry.get("config").is_some() {
                misnamed.push((key, "config", "parameters"));
            }
        }
    }
    for (key, wrong, right) in misnamed {
        warnings.push(format!(
            "Activity '{key}' uses unrecognised field '{wrong}', which is ignored by \
             the parser — the correct field is '{right}'. See \
             get_workflow_authoring_guide for the schema."
        ));
    }

    for activity in &definition.activities {
        if activity.activity_name.is_none() {
            warnings.push(format!(
                "Activity '{}' has no activity_name — workers dispatch on \
                 activity_name, so this activity will fail at runtime with \
                 'Activity implementation not found'. Set activity_name (e.g. \
                 llm_prompt, http_request) and put its arguments under parameters.",
                activity.key
            ));
        }
    }

    warnings
}

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
    // the structure might be readable. The schema requires a list, but accept
    // a map here too so the partial report still helps someone who wrote one.
    let Ok(doc) = serde_yaml::from_str::<serde_json::Value>(yaml) else {
        return (0, serde_json::json!({}));
    };
    let entries: Vec<(String, &serde_json::Value)> = match doc.get("activities") {
        Some(serde_json::Value::Object(map)) => map.iter().map(|(k, v)| (k.clone(), v)).collect(),
        Some(serde_json::Value::Array(arr)) => arr
            .iter()
            .map(|v| {
                let key = v
                    .get("key")
                    .and_then(|k| k.as_str())
                    .unwrap_or("?")
                    .to_string();
                (key, v)
            })
            .collect(),
        _ => return (0, serde_json::json!({})),
    };

    let count = entries.len();
    let deps: serde_json::Map<String, serde_json::Value> = entries
        .into_iter()
        .map(|(key, val)| {
            let dep_keys: Vec<String> = val
                .get("depends_on")
                .and_then(|d| d.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|item| {
                            // depends_on entries can be plain strings or objects with activity_key
                            item.as_str()
                                .or_else(|| item.get("activity_key").and_then(|v| v.as_str()))
                                .map(|s| s.to_string())
                        })
                        .collect()
                })
                .unwrap_or_default();
            (key, serde_json::json!(dep_keys))
        })
        .collect();
    (count, serde_json::Value::Object(deps))
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
