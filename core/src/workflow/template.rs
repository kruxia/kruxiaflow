use super::{ActivityOutput, OutputType};
use minijinja::{Environment, Value as MiniValue};
use serde_json::Value;
use std::collections::HashMap;
use thiserror::Error;
use tracing;

#[derive(Debug, Error)]
pub enum TemplateError {
    #[error("Template reference not found: {0}")]
    ReferenceNotFound(String),

    #[error("Invalid template syntax: {0}")]
    InvalidSyntax(String),

    #[error("Type error: {0}")]
    TypeError(String),

    #[error("Template evaluation error: {0}")]
    EvaluationError(String),
}

impl From<minijinja::Error> for TemplateError {
    fn from(err: minijinja::Error) -> Self {
        TemplateError::EvaluationError(err.to_string())
    }
}

/// Activity state information for template context
#[derive(Debug, Clone)]
pub struct ActivityContextInfo {
    /// Structured outputs with type information
    pub outputs: Vec<ActivityOutput>,

    /// Iteration-scoped outputs (grouped by name as arrays)
    /// Only present for iteration_scoped activities
    pub iteration_outputs: Option<HashMap<String, Vec<Value>>>,

    /// Current iteration number (0-based)
    pub iteration: u32,

    /// Accumulated cost in USD across all attempts/iterations
    pub accumulated_cost_usd: rust_decimal::Decimal,

    /// Activity status: "completed", "failed", "skipped", "pending", "running", "not_scheduled"
    pub status: String,
}

/// Template context for resolving expressions
#[derive(Debug, Clone)]
pub struct TemplateContext {
    /// Workflow inputs provided at runtime
    pub inputs: HashMap<String, Value>,

    /// Activity state information (key = activity_key)
    pub activity_states: HashMap<String, ActivityContextInfo>,

    /// Secrets (e.g., API keys)
    pub secrets: HashMap<String, String>,

    /// Workflow-level variables
    pub workflow: HashMap<String, Value>,

    /// Current activity context (for {{ACTIVITY.*}} variables)
    /// Key is the activity_key being resolved
    pub current_activity_key: Option<String>,

    /// Budget settings for current activity (used for remaining_budget_usd calculation)
    pub current_activity_budget_limit: Option<rust_decimal::Decimal>,

    /// Signal data for current activity (when wait_for_signal is configured)
    /// Available as {{SIGNAL}} or {{SIGNAL.field}} in templates
    pub signal: Option<Value>,
}

impl TemplateContext {
    pub fn new() -> Self {
        Self {
            inputs: HashMap::new(),
            activity_states: HashMap::new(),
            secrets: HashMap::new(),
            workflow: HashMap::new(),
            current_activity_key: None,
            current_activity_budget_limit: None,
            signal: None,
        }
    }

    pub fn with_inputs(mut self, inputs: HashMap<String, Value>) -> Self {
        self.inputs = inputs;
        self
    }

    pub fn with_secrets(mut self, secrets: HashMap<String, String>) -> Self {
        self.secrets = secrets;
        self
    }

    /// Add activity state information for template resolution
    pub fn add_activity_state(
        &mut self,
        activity_key: String,
        outputs: Vec<ActivityOutput>,
        iteration_outputs: Option<HashMap<String, Vec<Value>>>,
        iteration: u32,
        accumulated_cost_usd: rust_decimal::Decimal,
        status: String,
    ) {
        self.activity_states.insert(
            activity_key,
            ActivityContextInfo {
                outputs,
                iteration_outputs,
                iteration,
                accumulated_cost_usd,
                status,
            },
        );
    }

    /// Legacy method for backward compatibility
    pub fn add_activity_output(&mut self, activity_key: String, outputs: Vec<ActivityOutput>) {
        self.add_activity_state(
            activity_key,
            outputs,
            None,
            0,
            rust_decimal::Decimal::ZERO,
            "completed".to_string(), // Default to completed for legacy callers
        );
    }

    /// Set current activity context for {{ACTIVITY.*}} variables
    pub fn with_current_activity(
        mut self,
        activity_key: String,
        budget_limit: Option<rust_decimal::Decimal>,
    ) -> Self {
        self.current_activity_key = Some(activity_key);
        self.current_activity_budget_limit = budget_limit;
        self
    }

    /// Convert TemplateContext to minijinja Value for template evaluation
    fn to_minijinja_value(&self) -> MiniValue {
        let mut context_map = HashMap::new();

        // Add INPUT
        context_map.insert(
            "INPUT".to_string(),
            serde_json_to_minijinja(&Value::Object(
                self.inputs
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
            )),
        );

        // Add SECRET
        let secrets_obj: HashMap<String, Value> = self
            .secrets
            .iter()
            .map(|(k, v)| (k.clone(), Value::String(v.clone())))
            .collect();
        context_map.insert(
            "SECRET".to_string(),
            serde_json_to_minijinja(&Value::Object(secrets_obj.into_iter().collect())),
        );

        // Add WORKFLOW
        context_map.insert(
            "WORKFLOW".to_string(),
            serde_json_to_minijinja(&Value::Object(
                self.workflow
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
            )),
        );

        // Add activity outputs as top-level keys.
        // For iteration-scoped activities: serialize outputs grouped by name as arrays
        // For non-iteration-scoped activities: serialize outputs as single values
        // Also build FILE and FOLDER context maps
        let mut file_map: HashMap<String, HashMap<String, String>> = HashMap::new();
        let mut folder_map: HashMap<String, HashMap<String, String>> = HashMap::new();

        for (activity_key, activity_info) in &self.activity_states {
            // Check if this is an iteration-scoped activity
            if let Some(iteration_outputs) = &activity_info.iteration_outputs {
                // Iteration-scoped: outputs are already grouped by name as arrays
                let mut value_map: serde_json::Map<String, Value> = iteration_outputs
                    .iter()
                    .map(|(name, values)| (name.clone(), Value::Array(values.clone())))
                    .collect();

                // Add status to activity context
                value_map.insert(
                    "status".to_string(),
                    Value::String(activity_info.status.clone()),
                );

                // Always add iteration-scoped activities to context, even if empty
                // This ensures templates can reference them (e.g., {{activity.output | last}})
                context_map.insert(
                    activity_key.clone(),
                    serde_json_to_minijinja(&Value::Object(value_map)),
                );
            } else {
                // Non-iteration-scoped: serialize outputs as single values (current behavior)
                let mut value_outputs = HashMap::new();
                let mut file_outputs = HashMap::new();
                let mut folder_outputs = HashMap::new();

                for output in &activity_info.outputs {
                    match output.output_type {
                        OutputType::Value => {
                            value_outputs.insert(output.name.clone(), output.value.clone());
                        }
                        OutputType::File => {
                            if let Some(file_ref) = output.value.as_str() {
                                file_outputs.insert(output.name.clone(), file_ref.to_string());
                            }
                        }
                        OutputType::Folder => {
                            if let Some(folder_ref) = output.value.as_str() {
                                folder_outputs.insert(output.name.clone(), folder_ref.to_string());
                            }
                        }
                    }
                }

                // Add activity to context with status (always) and value outputs
                // This ensures {{activity.status}} is always accessible, even if no outputs yet
                let mut activity_obj: serde_json::Map<String, Value> =
                    value_outputs.into_iter().collect();
                activity_obj.insert(
                    "status".to_string(),
                    Value::String(activity_info.status.clone()),
                );
                context_map.insert(
                    activity_key.clone(),
                    serde_json_to_minijinja(&Value::Object(activity_obj)),
                );

                // Add to FILE map
                if !file_outputs.is_empty() {
                    file_map.insert(activity_key.clone(), file_outputs);
                }

                // Add to FOLDER map
                if !folder_outputs.is_empty() {
                    folder_map.insert(activity_key.clone(), folder_outputs);
                }
            }
        }

        // Add FILE context
        if !file_map.is_empty() {
            let file_value = Value::Object(
                file_map
                    .into_iter()
                    .map(|(k, v)| {
                        (
                            k,
                            Value::Object(
                                v.into_iter()
                                    .map(|(name, ref_str)| (name, Value::String(ref_str)))
                                    .collect(),
                            ),
                        )
                    })
                    .collect(),
            );
            context_map.insert("FILE".to_string(), serde_json_to_minijinja(&file_value));
        }

        // Add FOLDER context
        if !folder_map.is_empty() {
            let folder_value = Value::Object(
                folder_map
                    .into_iter()
                    .map(|(k, v)| {
                        (
                            k,
                            Value::Object(
                                v.into_iter()
                                    .map(|(name, ref_str)| (name, Value::String(ref_str)))
                                    .collect(),
                            ),
                        )
                    })
                    .collect(),
            );
            context_map.insert("FOLDER".to_string(), serde_json_to_minijinja(&folder_value));
        }

        // Add ACTIVITY context (for current activity being resolved)
        if let Some(current_key) = &self.current_activity_key
            && let Some(activity_info) = self.activity_states.get(current_key)
        {
            let remaining_budget = if let Some(limit) = self.current_activity_budget_limit {
                (limit - activity_info.accumulated_cost_usd)
                    .max(rust_decimal::Decimal::ZERO)
                    .to_string()
            } else {
                "0.00".to_string()
            };

            let activity_context = serde_json::json!({
                "iteration": activity_info.iteration,
                "accumulated_cost_usd": activity_info.accumulated_cost_usd.to_string(),
                "remaining_budget_usd": remaining_budget,
                "status": activity_info.status,
            });

            context_map.insert(
                "ACTIVITY".to_string(),
                serde_json_to_minijinja(&activity_context),
            );
        }

        // Add SIGNAL context (for activities that received an external signal)
        if let Some(signal_data) = &self.signal {
            context_map.insert("SIGNAL".to_string(), serde_json_to_minijinja(signal_data));
        }

        MiniValue::from_object(context_map)
    }
}

impl Default for TemplateContext {
    fn default() -> Self {
        Self::new()
    }
}

/// Create a minijinja Environment with custom filters
fn create_template_env() -> Environment<'static> {
    let mut env = Environment::new();

    // Configure strict undefined behavior - errors on undefined variables
    env.set_undefined_behavior(minijinja::UndefinedBehavior::Strict);

    // Add custom coalesce filter that handles both undefined and null values
    // This addresses: docs/bugs/2026-01-08-minijinja-default-filter-null.md
    // The built-in `default` filter only handles undefined, not null/None values
    env.add_filter("coalesce", |value: MiniValue, default: MiniValue| {
        if value.is_undefined() || value.is_none() {
            default
        } else {
            value
        }
    });

    env
}

/// Check if a string contains template expressions {{...}}
fn contains_templates(s: &str) -> bool {
    s.contains("{{") && s.contains("}}")
}

/// Check if entire string is a single template expression
fn is_whole_template(s: &str) -> bool {
    let trimmed = s.trim();
    trimmed.starts_with("{{")
        && trimmed.ends_with("}}")
        && !trimmed[2..trimmed.len() - 2].contains("{{") // No nested {{
}

/// Extract expression from template string "{{expr}}" -> "expr"
fn extract_expression(s: &str) -> &str {
    let trimmed = s.trim();
    if trimmed.starts_with("{{") && trimmed.ends_with("}}") {
        trimmed[2..trimmed.len() - 2].trim()
    } else {
        s
    }
}

/// Resolve template expressions in a JSON value using minijinja
///
/// - For whole-value templates like `"{{INPUT.max_retries}}"`, evaluates expression and preserves type
/// - For embedded templates like `"Status: {{INPUT.status}}"`, performs string interpolation
/// - Recursively processes objects and arrays
pub fn resolve_template_value(
    value: &Value,
    context: &TemplateContext,
) -> Result<Value, TemplateError> {
    let env = create_template_env();
    let mini_context = context.to_minijinja_value();

    resolve_value_recursive(value, &env, &mini_context)
}

fn resolve_value_recursive(
    value: &Value,
    env: &Environment,
    context: &MiniValue,
) -> Result<Value, TemplateError> {
    match value {
        Value::String(s) => {
            if is_whole_template(s) {
                // Entire value is a template - evaluate as expression, preserve type
                let expr_str = extract_expression(s);
                let expr = env.compile_expression(expr_str)?;
                let mini_result = expr.eval(context)?;
                let result = minijinja_to_serde_json(&mini_result);

                // Debug logging for embeddings_file template resolution
                if expr_str.contains("embeddings_file") {
                    tracing::info!(
                        expr = %expr_str,
                        mini_result_kind = ?mini_result.kind(),
                        mini_result_is_undefined = mini_result.is_undefined(),
                        mini_result_str = %mini_result,
                        result = ?result,
                        "Template resolved embeddings_file expression"
                    );
                }

                Ok(result)
            } else if contains_templates(s) {
                // String contains embedded templates - render as template string
                let tmpl = env.template_from_str(s)?;
                let rendered = tmpl.render(context)?;
                Ok(Value::String(rendered))
            } else {
                // No templates, return as-is
                Ok(value.clone())
            }
        }
        Value::Array(arr) => {
            // Recursively resolve array elements
            let resolved: Result<Vec<Value>, _> = arr
                .iter()
                .map(|v| resolve_value_recursive(v, env, context))
                .collect();
            Ok(Value::Array(resolved?))
        }
        Value::Object(obj) => {
            // Recursively resolve object values
            let resolved: Result<serde_json::Map<String, Value>, TemplateError> = obj
                .iter()
                .map(|(k, v)| {
                    let resolved_v = resolve_value_recursive(v, env, context)?;

                    // Debug logging for embeddings_file key
                    if k == "embeddings_file" {
                        tracing::info!(
                            key = %k,
                            input_value = ?v,
                            resolved_value = ?resolved_v,
                            "Object key embeddings_file resolved"
                        );
                    }

                    Ok((k.clone(), resolved_v))
                })
                .collect();
            Ok(Value::Object(resolved?))
        }
        _ => {
            // Numbers, bools, null pass through unchanged
            Ok(value.clone())
        }
    }
}

/// Convert serde_json::Value to minijinja::Value
fn serde_json_to_minijinja(value: &Value) -> MiniValue {
    match value {
        Value::Null => MiniValue::from(()),
        Value::Bool(b) => MiniValue::from(*b),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                MiniValue::from(i)
            } else if let Some(u) = n.as_u64() {
                MiniValue::from(u)
            } else if let Some(f) = n.as_f64() {
                MiniValue::from(f)
            } else {
                MiniValue::from(())
            }
        }
        Value::String(s) => MiniValue::from(s.as_str()),
        Value::Array(arr) => {
            let mini_arr: Vec<MiniValue> = arr.iter().map(serde_json_to_minijinja).collect();
            MiniValue::from(mini_arr)
        }
        Value::Object(obj) => {
            let mini_obj: HashMap<String, MiniValue> = obj
                .iter()
                .map(|(k, v)| (k.clone(), serde_json_to_minijinja(v)))
                .collect();
            MiniValue::from_object(mini_obj)
        }
    }
}

/// Convert minijinja::Value to serde_json::Value
fn minijinja_to_serde_json(value: &MiniValue) -> Value {
    match value.kind() {
        minijinja::value::ValueKind::Undefined | minijinja::value::ValueKind::None => Value::Null,
        minijinja::value::ValueKind::Bool => Value::Bool(value.is_true()),
        minijinja::value::ValueKind::Number => {
            // Try to get as i64 first for integers
            if let Some(i) = value.as_i64() {
                Value::Number(serde_json::Number::from(i))
            } else {
                // For floats, convert via string representation to preserve precision
                let value_str = value.to_string();
                if let Ok(f) = value_str.parse::<f64>() {
                    if let Some(n) = serde_json::Number::from_f64(f) {
                        Value::Number(n)
                    } else {
                        Value::Null
                    }
                } else {
                    Value::Null
                }
            }
        }
        minijinja::value::ValueKind::String => Value::String(value.to_string()),
        minijinja::value::ValueKind::Seq => {
            let arr: Vec<Value> = value
                .try_iter()
                .unwrap()
                .map(|v| minijinja_to_serde_json(&v))
                .collect();
            Value::Array(arr)
        }
        minijinja::value::ValueKind::Map => {
            let obj: serde_json::Map<String, Value> = value
                .try_iter()
                .unwrap()
                .filter_map(|key| {
                    let key_str = key.to_string();
                    value
                        .get_item(&key)
                        .ok()
                        .map(|val| (key_str, minijinja_to_serde_json(&val)))
                })
                .collect();
            Value::Object(obj)
        }
        _ => Value::Null,
    }
}

/// Legacy compatibility: Resolve template expressions in a string (always returns string)
///
/// For new code, prefer resolve_template_value() which preserves types
pub fn resolve_template(
    template: &str,
    context: &TemplateContext,
) -> Result<String, TemplateError> {
    let env = create_template_env();
    let mini_context = context.to_minijinja_value();

    if contains_templates(template) {
        let tmpl = env.template_from_str(template)?;
        Ok(tmpl.render(&mini_context)?)
    } else {
        Ok(template.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_input_template() {
        let mut context = TemplateContext::new();
        context.inputs.insert(
            "webhook_url".to_string(),
            Value::String("https://example.com/webhook".to_string()),
        );

        let template = "{{INPUT.webhook_url}}";
        let result = resolve_template(template, &context).unwrap();
        assert_eq!(result, "https://example.com/webhook");
    }

    #[test]
    fn test_resolve_activity_output_template() {
        use crate::workflow::{ActivityOutput, OutputType};

        let mut context = TemplateContext::new();
        let outputs = vec![
            ActivityOutput {
                name: "temperature".to_string(),
                output_type: OutputType::Value,
                value: Value::Number(72.into()),
            },
            ActivityOutput {
                name: "conditions".to_string(),
                output_type: OutputType::Value,
                value: Value::String("sunny".to_string()),
            },
        ];
        context.add_activity_output("fetch_weather".to_string(), outputs);

        let template =
            "Temperature is {{fetch_weather.temperature}} and {{fetch_weather.conditions}}";
        let result = resolve_template(template, &context).unwrap();
        assert_eq!(result, "Temperature is 72 and sunny");
    }

    #[test]
    fn test_resolve_secret_template() {
        let mut context = TemplateContext::new();
        context
            .secrets
            .insert("api_key".to_string(), "secret_key_123".to_string());

        let template = "Bearer {{SECRET.api_key}}";
        let result = resolve_template(template, &context).unwrap();
        assert_eq!(result, "Bearer secret_key_123");
    }

    // ============================================================================
    // Regression Tests: Secrets in Template Context
    // Prevents: docs/bugs/2026-01-04-secrets-not-loaded.md
    // ============================================================================

    #[test]
    fn test_secrets_with_builder_pattern() {
        // Verify the with_secrets() builder method works correctly
        let mut secrets = HashMap::new();
        secrets.insert(
            "db_url".to_string(),
            "postgres://localhost/test".to_string(),
        );
        secrets.insert("api_key".to_string(), "sk-12345".to_string());

        let context = TemplateContext::new().with_secrets(secrets);

        // Verify secrets are accessible in templates
        let template = "{{SECRET.db_url}}";
        let result = resolve_template(template, &context).unwrap();
        assert_eq!(result, "postgres://localhost/test");

        let template = "{{SECRET.api_key}}";
        let result = resolve_template(template, &context).unwrap();
        assert_eq!(result, "sk-12345");
    }

    #[test]
    fn test_secrets_multiple_in_one_template() {
        // Multiple secrets referenced in a single template string
        let mut secrets = HashMap::new();
        secrets.insert("host".to_string(), "db.example.com".to_string());
        secrets.insert("port".to_string(), "5432".to_string());
        secrets.insert("user".to_string(), "admin".to_string());
        secrets.insert("pass".to_string(), "secret123".to_string());

        let context = TemplateContext::new().with_secrets(secrets);

        let template =
            "postgres://{{SECRET.user}}:{{SECRET.pass}}@{{SECRET.host}}:{{SECRET.port}}/mydb";
        let result = resolve_template(template, &context).unwrap();
        assert_eq!(
            result,
            "postgres://admin:secret123@db.example.com:5432/mydb"
        );
    }

    #[test]
    fn test_secrets_as_whole_value() {
        // When entire value is a secret reference, type should be preserved (string)
        let mut secrets = HashMap::new();
        secrets.insert(
            "db_url".to_string(),
            "postgres://localhost/test".to_string(),
        );

        let context = TemplateContext::new().with_secrets(secrets);

        let value = Value::String("{{SECRET.db_url}}".to_string());
        let result = resolve_template_value(&value, &context).unwrap();
        assert_eq!(
            result,
            Value::String("postgres://localhost/test".to_string())
        );
    }

    #[test]
    fn test_secrets_with_special_characters() {
        // Secret values with special characters should be preserved exactly
        let mut secrets = HashMap::new();
        secrets.insert(
            "complex_pass".to_string(),
            "p@ss=word!with#special$chars&more%stuff".to_string(),
        );

        let context = TemplateContext::new().with_secrets(secrets);

        let value = Value::String("{{SECRET.complex_pass}}".to_string());
        let result = resolve_template_value(&value, &context).unwrap();
        assert_eq!(
            result,
            Value::String("p@ss=word!with#special$chars&more%stuff".to_string())
        );
    }

    #[test]
    fn test_secrets_in_object() {
        // Secrets should resolve correctly when used in object values
        let mut secrets = HashMap::new();
        secrets.insert(
            "db_url".to_string(),
            "postgres://localhost/test".to_string(),
        );
        secrets.insert("api_key".to_string(), "sk-secret-key".to_string());

        let context = TemplateContext::new().with_secrets(secrets);

        let object = serde_json::json!({
            "database_url": "{{SECRET.db_url}}",
            "authorization": "Bearer {{SECRET.api_key}}"
        });

        let result = resolve_template_value(&object, &context).unwrap();
        assert_eq!(
            result,
            serde_json::json!({
                "database_url": "postgres://localhost/test",
                "authorization": "Bearer sk-secret-key"
            })
        );
    }

    #[test]
    fn test_secrets_missing_returns_null() {
        // Accessing a non-existent key on SECRET object returns null (not an error)
        // because SECRET itself exists as an empty object
        let context = TemplateContext::new(); // No secrets

        let value = Value::String("{{SECRET.missing_key}}".to_string());
        let result = resolve_template_value(&value, &context).unwrap();

        // Missing keys on existing objects return null (undefined -> null)
        assert_eq!(result, Value::Null);
    }

    #[test]
    fn test_secrets_undefined_top_level_fails() {
        // Accessing a completely undefined top-level variable fails in strict mode
        let context = TemplateContext::new();

        let value = Value::String("{{UNDEFINED_CONTEXT.key}}".to_string());
        let result = resolve_template_value(&value, &context);

        assert!(
            result.is_err(),
            "Should fail when top-level context doesn't exist"
        );
    }

    #[test]
    fn test_secrets_empty_string_value() {
        // Empty string secrets should be preserved, not treated as missing
        let mut secrets = HashMap::new();
        secrets.insert("empty_secret".to_string(), "".to_string());

        let context = TemplateContext::new().with_secrets(secrets);

        let value = Value::String("{{SECRET.empty_secret}}".to_string());
        let result = resolve_template_value(&value, &context).unwrap();
        assert_eq!(result, Value::String("".to_string()));
    }

    #[test]
    fn test_secrets_combined_with_inputs() {
        // Secrets and inputs should work together in the same template
        let mut secrets = HashMap::new();
        secrets.insert("api_key".to_string(), "sk-secret".to_string());

        let mut inputs = HashMap::new();
        inputs.insert(
            "endpoint".to_string(),
            Value::String("https://api.example.com".to_string()),
        );

        let context = TemplateContext::new()
            .with_secrets(secrets)
            .with_inputs(inputs);

        let template = "curl -H 'Authorization: {{SECRET.api_key}}' {{INPUT.endpoint}}";
        let result = resolve_template(template, &context).unwrap();
        assert_eq!(
            result,
            "curl -H 'Authorization: sk-secret' https://api.example.com"
        );
    }

    #[test]
    fn test_secrets_with_filter() {
        // Secrets should work with minijinja filters
        let mut secrets = HashMap::new();
        secrets.insert("name".to_string(), "secret_value".to_string());

        let context = TemplateContext::new().with_secrets(secrets);

        let value = Value::String("{{SECRET.name | upper}}".to_string());
        let result = resolve_template_value(&value, &context).unwrap();
        assert_eq!(result, Value::String("SECRET_VALUE".to_string()));
    }

    #[test]
    fn test_secrets_default_filter_for_missing() {
        // The default filter can provide fallback for missing secrets
        let context = TemplateContext::new(); // No secrets

        let value = Value::String("{{SECRET.missing | default('fallback_value')}}".to_string());
        let result = resolve_template_value(&value, &context).unwrap();
        assert_eq!(result, Value::String("fallback_value".to_string()));
    }

    #[test]
    fn test_secret_context_is_object() {
        // Verify SECRET is exposed as an object in the template context
        let mut secrets = HashMap::new();
        secrets.insert("key1".to_string(), "value1".to_string());
        secrets.insert("key2".to_string(), "value2".to_string());

        let context = TemplateContext::new().with_secrets(secrets);

        // Access multiple secrets to verify object structure
        let template = "{{SECRET.key1}}-{{SECRET.key2}}";
        let result = resolve_template(template, &context).unwrap();
        assert_eq!(result, "value1-value2");
    }

    #[test]
    fn test_resolve_template_value_preserves_types() {
        use crate::workflow::{ActivityOutput, OutputType};

        let mut context = TemplateContext::new();
        let outputs = vec![
            ActivityOutput {
                name: "temperature".to_string(),
                output_type: OutputType::Value,
                value: Value::Number(72.into()),
            },
            ActivityOutput {
                name: "valid".to_string(),
                output_type: OutputType::Value,
                value: Value::Bool(true),
            },
        ];
        context.add_activity_output("check".to_string(), outputs);

        // When entire value is a template, preserve type
        let value = Value::String("{{check.temperature}}".to_string());
        let result = resolve_template_value(&value, &context).unwrap();
        assert_eq!(result, Value::Number(72.into()));

        let value = Value::String("{{check.valid}}".to_string());
        let result = resolve_template_value(&value, &context).unwrap();
        assert_eq!(result, Value::Bool(true));
    }

    #[test]
    fn test_resolve_template_value_in_object() {
        use crate::workflow::{ActivityOutput, OutputType};

        let mut context = TemplateContext::new();
        context.inputs.insert(
            "email".to_string(),
            Value::String("test@example.com".to_string()),
        );
        let outputs = vec![ActivityOutput {
            name: "valid".to_string(),
            output_type: OutputType::Value,
            value: Value::Bool(true),
        }];
        context.add_activity_output("check_email".to_string(), outputs);

        let object = serde_json::json!({
            "email": "{{INPUT.email}}",
            "status": "{{check_email.valid}}"
        });

        let result = resolve_template_value(&object, &context).unwrap();
        assert_eq!(
            result,
            serde_json::json!({
                "email": "test@example.com",
                "status": true
            })
        );
    }

    #[test]
    fn test_minijinja_filters() {
        let mut context = TemplateContext::new();
        context
            .inputs
            .insert("name".to_string(), Value::String("alice".to_string()));

        // Test upper filter
        let value = Value::String("{{INPUT.name | upper}}".to_string());
        let result = resolve_template_value(&value, &context).unwrap();
        assert_eq!(result, Value::String("ALICE".to_string()));
    }

    #[test]
    fn test_minijinja_default_filter() {
        let context = TemplateContext::new();

        // Test default filter for missing value
        let value = Value::String("{{INPUT.missing | default('fallback')}}".to_string());
        let result = resolve_template_value(&value, &context).unwrap();
        assert_eq!(result, Value::String("fallback".to_string()));
    }

    #[test]
    fn test_nested_path_access() {
        use crate::workflow::{ActivityOutput, OutputType};

        let mut context = TemplateContext::new();
        let outputs = vec![ActivityOutput {
            name: "response".to_string(),
            output_type: OutputType::Value,
            value: serde_json::json!({
                "json": {
                    "properties": {
                        "periods": [
                            {"temperature": 72, "conditions": "Partly Cloudy"}
                        ]
                    }
                }
            }),
        }];
        context.add_activity_output("fetch_weather".to_string(), outputs);

        let value = Value::String(
            "{{fetch_weather.response.json.properties.periods[0].temperature}}".to_string(),
        );
        let result = resolve_template_value(&value, &context).unwrap();
        assert_eq!(result, Value::Number(72.into()));
    }

    #[test]
    fn test_array_resolution() {
        let mut context = TemplateContext::new();
        context
            .inputs
            .insert("count".to_string(), Value::Number(5.into()));

        let array = serde_json::json!([
            "{{INPUT.count}}",
            "static value",
            "Count is {{INPUT.count}}"
        ]);

        let result = resolve_template_value(&array, &context).unwrap();
        assert_eq!(result, serde_json::json!([5, "static value", "Count is 5"]));
    }

    #[test]
    fn test_missing_reference_error() {
        let context = TemplateContext::new();
        let template = "{{INPUT.missing}}";
        let result = resolve_template(template, &context);
        assert!(result.is_err());
    }

    #[test]
    fn test_resolve_iteration_scoped_as_array() {
        let mut context = TemplateContext::new();

        // Create iteration outputs grouped by name as arrays
        let mut iteration_outputs = HashMap::new();
        iteration_outputs.insert(
            "results".to_string(),
            vec![
                serde_json::json!("result1"),
                serde_json::json!("result2"),
                serde_json::json!("result3"),
            ],
        );
        iteration_outputs.insert(
            "score".to_string(),
            vec![
                serde_json::json!(10),
                serde_json::json!(20),
                serde_json::json!(30),
            ],
        );

        context.add_activity_state(
            "search".to_string(),
            vec![],
            Some(iteration_outputs),
            2,
            rust_decimal::Decimal::new(750, 2), // $7.50
            "completed".to_string(),
        );

        // Access all iterations as array
        let value = Value::String("{{search.results}}".to_string());
        let result = resolve_template_value(&value, &context).unwrap();
        assert_eq!(
            result,
            Value::Array(vec![
                serde_json::json!("result1"),
                serde_json::json!("result2"),
                serde_json::json!("result3"),
            ])
        );

        // Access scores array
        let value = Value::String("{{search.score}}".to_string());
        let result = resolve_template_value(&value, &context).unwrap();
        assert_eq!(
            result,
            Value::Array(vec![
                serde_json::json!(10),
                serde_json::json!(20),
                serde_json::json!(30),
            ])
        );
    }

    #[test]
    fn test_resolve_latest_with_filter() {
        let mut context = TemplateContext::new();

        // Create iteration outputs
        let mut iteration_outputs = HashMap::new();
        iteration_outputs.insert(
            "results".to_string(),
            vec![
                serde_json::json!("result1"),
                serde_json::json!("result2"),
                serde_json::json!("result3"),
            ],
        );

        context.add_activity_state(
            "search".to_string(),
            vec![],
            Some(iteration_outputs),
            2,
            rust_decimal::Decimal::ZERO,
            "completed".to_string(),
        );

        // Access latest value using | last filter
        let value = Value::String("{{search.results | last}}".to_string());
        let result = resolve_template_value(&value, &context).unwrap();
        assert_eq!(result, serde_json::json!("result3"));

        // Access first value using | first filter
        let value = Value::String("{{search.results | first}}".to_string());
        let result = resolve_template_value(&value, &context).unwrap();
        assert_eq!(result, serde_json::json!("result1"));
    }

    #[test]
    fn test_resolve_array_length_filter() {
        let mut context = TemplateContext::new();

        let mut iteration_outputs = HashMap::new();
        iteration_outputs.insert(
            "results".to_string(),
            vec![
                serde_json::json!("result1"),
                serde_json::json!("result2"),
                serde_json::json!("result3"),
            ],
        );

        context.add_activity_state(
            "search".to_string(),
            vec![],
            Some(iteration_outputs),
            2,
            rust_decimal::Decimal::ZERO,
            "completed".to_string(),
        );

        // Get array length using | length filter
        let value = Value::String("{{search.results | length}}".to_string());
        let result = resolve_template_value(&value, &context).unwrap();
        assert_eq!(result, Value::Number(3.into()));
    }

    #[test]
    fn test_resolve_iteration_counter() {
        let mut context = TemplateContext::new();

        let mut iteration_outputs = HashMap::new();
        iteration_outputs.insert("results".to_string(), vec![serde_json::json!("result1")]);

        context.add_activity_state(
            "search".to_string(),
            vec![],
            Some(iteration_outputs),
            2, // iteration = 2
            rust_decimal::Decimal::ZERO,
            "completed".to_string(),
        );

        // Set current activity context
        context = context.with_current_activity("search".to_string(), None);

        // Access iteration counter via {{ACTIVITY.iteration}}
        let value = Value::String("{{ACTIVITY.iteration}}".to_string());
        let result = resolve_template_value(&value, &context).unwrap();
        assert_eq!(result, Value::Number(2.into()));
    }

    #[test]
    fn test_resolve_accumulated_cost() {
        let mut context = TemplateContext::new();

        let mut iteration_outputs = HashMap::new();
        iteration_outputs.insert("results".to_string(), vec![serde_json::json!("result1")]);

        context.add_activity_state(
            "search".to_string(),
            vec![],
            Some(iteration_outputs),
            2,
            rust_decimal::Decimal::new(1050, 2), // $10.50
            "completed".to_string(),
        );

        // Set current activity context
        context = context.with_current_activity("search".to_string(), None);

        // Access accumulated cost via {{ACTIVITY.accumulated_cost_usd}}
        let value = Value::String("{{ACTIVITY.accumulated_cost_usd}}".to_string());
        let result = resolve_template_value(&value, &context).unwrap();
        assert_eq!(result, Value::String("10.50".to_string()));
    }

    #[test]
    fn test_resolve_remaining_budget() {
        let mut context = TemplateContext::new();

        let mut iteration_outputs = HashMap::new();
        iteration_outputs.insert("results".to_string(), vec![serde_json::json!("result1")]);

        context.add_activity_state(
            "search".to_string(),
            vec![],
            Some(iteration_outputs),
            2,
            rust_decimal::Decimal::new(750, 2), // $7.50 accumulated
            "completed".to_string(),
        );

        // Set current activity context with budget limit of $10.00
        context = context.with_current_activity(
            "search".to_string(),
            Some(rust_decimal::Decimal::new(1000, 2)), // $10.00 limit
        );

        // Access remaining budget via {{ACTIVITY.remaining_budget_usd}}
        let value = Value::String("{{ACTIVITY.remaining_budget_usd}}".to_string());
        let result = resolve_template_value(&value, &context).unwrap();
        assert_eq!(result, Value::String("2.50".to_string())); // $10.00 - $7.50 = $2.50
    }

    #[test]
    fn test_non_iteration_scoped_outputs() {
        use crate::workflow::{ActivityOutput, OutputType};

        let mut context = TemplateContext::new();

        // Non-iteration-scoped activity (iteration_outputs = None)
        context.add_activity_state(
            "fetch".to_string(),
            vec![
                ActivityOutput {
                    name: "temperature".to_string(),
                    output_type: OutputType::Value,
                    value: Value::Number(72.into()),
                },
                ActivityOutput {
                    name: "conditions".to_string(),
                    output_type: OutputType::Value,
                    value: Value::String("sunny".to_string()),
                },
            ],
            None, // Not iteration-scoped
            0,
            rust_decimal::Decimal::ZERO,
            "completed".to_string(),
        );

        // Should return single values (not arrays)
        let value = Value::String("{{fetch.temperature}}".to_string());
        let result = resolve_template_value(&value, &context).unwrap();
        assert_eq!(result, Value::Number(72.into()));

        let value = Value::String("{{fetch.conditions}}".to_string());
        let result = resolve_template_value(&value, &context).unwrap();
        assert_eq!(result, Value::String("sunny".to_string()));
    }

    #[test]
    fn test_iteration_array_in_condition() {
        let mut context = TemplateContext::new();

        let mut iteration_outputs = HashMap::new();
        iteration_outputs.insert(
            "sufficient".to_string(),
            vec![
                serde_json::json!(false),
                serde_json::json!(false),
                serde_json::json!(true),
            ],
        );

        context.add_activity_state(
            "evaluate".to_string(),
            vec![],
            Some(iteration_outputs),
            2,
            rust_decimal::Decimal::ZERO,
            "completed".to_string(),
        );

        // Check latest iteration result in condition (typical loop pattern)
        let value = Value::String("{{evaluate.sufficient | last}}".to_string());
        let result = resolve_template_value(&value, &context).unwrap();
        assert_eq!(result, Value::Bool(true));

        // Test with false value
        let mut iteration_outputs_false = HashMap::new();
        iteration_outputs_false.insert(
            "sufficient".to_string(),
            vec![
                serde_json::json!(false),
                serde_json::json!(false),
                serde_json::json!(false),
            ],
        );

        let mut context2 = TemplateContext::new();
        context2.add_activity_state(
            "evaluate".to_string(),
            vec![],
            Some(iteration_outputs_false),
            2,
            rust_decimal::Decimal::ZERO,
            "completed".to_string(),
        );

        let value2 = Value::String("{{evaluate.sufficient | last}}".to_string());
        let result2 = resolve_template_value(&value2, &context2).unwrap();
        assert_eq!(result2, Value::Bool(false));
    }

    #[test]
    fn test_activity_context_only_for_current_activity() {
        let mut context = TemplateContext::new();

        let mut iteration_outputs = HashMap::new();
        iteration_outputs.insert("results".to_string(), vec![serde_json::json!("result1")]);

        context.add_activity_state(
            "search".to_string(),
            vec![],
            Some(iteration_outputs),
            2,
            rust_decimal::Decimal::new(750, 2),
            "completed".to_string(),
        );

        // Don't set current_activity_key - ACTIVITY context should not be available
        // This should fail because ACTIVITY is undefined and we use strict mode
        let value = Value::String("{{ACTIVITY.iteration}}".to_string());
        let result = resolve_template_value(&value, &context);
        assert!(result.is_err()); // Should fail with undefined error

        // Now set current activity
        context = context.with_current_activity("search".to_string(), None);

        let value = Value::String("{{ACTIVITY.iteration}}".to_string());
        let result = resolve_template_value(&value, &context).unwrap();
        assert_eq!(result, Value::Number(2.into())); // Should return actual value
    }

    // ============================================================================
    // Coalesce Filter Tests
    // Regression tests for: docs/bugs/2026-01-08-minijinja-default-filter-null.md
    // ============================================================================

    #[test]
    fn test_coalesce_filter_on_null_value() {
        // The coalesce filter should return the default when value is null
        use crate::workflow::{ActivityOutput, OutputType};

        let mut context = TemplateContext::new();
        let outputs = vec![ActivityOutput {
            name: "result".to_string(),
            output_type: OutputType::Value,
            value: serde_json::json!({
                "rows": [{"doi": null}]  // Explicit null value from database
            }),
        }];
        context.add_activity_output("check_source".to_string(), outputs);

        // Without coalesce, accessing doi returns null
        let value = Value::String("{{check_source.result.rows[0].doi}}".to_string());
        let result = resolve_template_value(&value, &context).unwrap();
        assert_eq!(result, Value::Null);

        // With coalesce, null values get the fallback
        let value = Value::String("{{check_source.result.rows[0].doi | coalesce('')}}".to_string());
        let result = resolve_template_value(&value, &context).unwrap();
        assert_eq!(result, Value::String("".to_string()));
    }

    #[test]
    fn test_coalesce_filter_with_length() {
        // This is the exact bug case: {{value | coalesce('') | length > 0}}
        // The default filter doesn't work because it only handles undefined, not null
        use crate::workflow::{ActivityOutput, OutputType};

        let mut context = TemplateContext::new();
        let outputs = vec![ActivityOutput {
            name: "result".to_string(),
            output_type: OutputType::Value,
            value: serde_json::json!({
                "rows": [{"doi": null}]
            }),
        }];
        context.add_activity_output("check_source".to_string(), outputs);

        // This should work: coalesce handles null, then length works on empty string
        let value = Value::String(
            "{{check_source.result.rows[0].doi | coalesce('') | length}}".to_string(),
        );
        let result = resolve_template_value(&value, &context).unwrap();
        assert_eq!(result, Value::Number(0.into()));

        // Test the full condition pattern
        let value = Value::String(
            "{{check_source.result.rows[0].doi | coalesce('') | length > 0}}".to_string(),
        );
        let result = resolve_template_value(&value, &context).unwrap();
        assert_eq!(result, Value::Bool(false));
    }

    #[test]
    fn test_coalesce_filter_on_undefined_value() {
        // Coalesce should also handle undefined values (like default does)
        let context = TemplateContext::new();

        // Missing key on INPUT should trigger coalesce fallback
        let value = Value::String("{{INPUT.missing | coalesce('fallback')}}".to_string());
        let result = resolve_template_value(&value, &context).unwrap();
        assert_eq!(result, Value::String("fallback".to_string()));
    }

    #[test]
    fn test_coalesce_filter_on_non_null_value() {
        // Coalesce should pass through non-null values unchanged
        use crate::workflow::{ActivityOutput, OutputType};

        let mut context = TemplateContext::new();
        let outputs = vec![ActivityOutput {
            name: "result".to_string(),
            output_type: OutputType::Value,
            value: serde_json::json!({
                "rows": [{"doi": "10.1234/test"}]
            }),
        }];
        context.add_activity_output("check_source".to_string(), outputs);

        // When value is present, coalesce returns it unchanged
        let value = Value::String("{{check_source.result.rows[0].doi | coalesce('')}}".to_string());
        let result = resolve_template_value(&value, &context).unwrap();
        assert_eq!(result, Value::String("10.1234/test".to_string()));

        // Length check should work on the actual value
        let value = Value::String(
            "{{check_source.result.rows[0].doi | coalesce('') | length > 0}}".to_string(),
        );
        let result = resolve_template_value(&value, &context).unwrap();
        assert_eq!(result, Value::Bool(true));
    }

    #[test]
    fn test_coalesce_filter_preserves_type() {
        // Coalesce should preserve types of non-null values
        use crate::workflow::{ActivityOutput, OutputType};

        let mut context = TemplateContext::new();
        let outputs = vec![ActivityOutput {
            name: "result".to_string(),
            output_type: OutputType::Value,
            value: serde_json::json!({
                "count": 42,
                "active": true,
                "items": ["a", "b", "c"]
            }),
        }];
        context.add_activity_output("data".to_string(), outputs);

        // Number type preserved
        let value = Value::String("{{data.result.count | coalesce(0)}}".to_string());
        let result = resolve_template_value(&value, &context).unwrap();
        assert_eq!(result, Value::Number(42.into()));

        // Boolean type preserved
        let value = Value::String("{{data.result.active | coalesce(false)}}".to_string());
        let result = resolve_template_value(&value, &context).unwrap();
        assert_eq!(result, Value::Bool(true));

        // Array type preserved
        let value = Value::String("{{data.result.items | coalesce([])}}".to_string());
        let result = resolve_template_value(&value, &context).unwrap();
        assert_eq!(
            result,
            Value::Array(vec![
                Value::String("a".to_string()),
                Value::String("b".to_string()),
                Value::String("c".to_string())
            ])
        );
    }

    #[test]
    fn test_coalesce_vs_default_on_null() {
        // Demonstrate the difference between coalesce and default filters
        use crate::workflow::{ActivityOutput, OutputType};

        let mut context = TemplateContext::new();
        let outputs = vec![ActivityOutput {
            name: "result".to_string(),
            output_type: OutputType::Value,
            value: serde_json::json!({
                "doi": null  // Explicit null
            }),
        }];
        context.add_activity_output("source".to_string(), outputs);

        // default filter: does NOT handle null (only undefined)
        // This returns null because the key exists (just has null value)
        let value = Value::String("{{source.result.doi | default('fallback')}}".to_string());
        let result = resolve_template_value(&value, &context).unwrap();
        assert_eq!(result, Value::Null); // default doesn't help with null!

        // coalesce filter: handles BOTH null and undefined
        let value = Value::String("{{source.result.doi | coalesce('fallback')}}".to_string());
        let result = resolve_template_value(&value, &context).unwrap();
        assert_eq!(result, Value::String("fallback".to_string())); // coalesce works!
    }

    #[test]
    fn test_coalesce_with_zero_and_empty_string() {
        // Zero and empty string are valid non-null values, should NOT trigger fallback
        use crate::workflow::{ActivityOutput, OutputType};

        let mut context = TemplateContext::new();
        let outputs = vec![ActivityOutput {
            name: "result".to_string(),
            output_type: OutputType::Value,
            value: serde_json::json!({
                "count": 0,
                "name": ""
            }),
        }];
        context.add_activity_output("data".to_string(), outputs);

        // Zero should NOT be replaced
        let value = Value::String("{{data.result.count | coalesce(99)}}".to_string());
        let result = resolve_template_value(&value, &context).unwrap();
        assert_eq!(result, Value::Number(0.into()));

        // Empty string should NOT be replaced
        let value = Value::String("{{data.result.name | coalesce('default')}}".to_string());
        let result = resolve_template_value(&value, &context).unwrap();
        assert_eq!(result, Value::String("".to_string()));
    }

    #[test]
    fn test_coalesce_in_condition_expression() {
        // Test coalesce in a full conditional expression (typical use case)
        use crate::workflow::{ActivityOutput, OutputType};

        let mut context = TemplateContext::new();
        let outputs = vec![ActivityOutput {
            name: "result".to_string(),
            output_type: OutputType::Value,
            value: serde_json::json!({
                "rows": [{"doi": null}]
            }),
        }];
        context.add_activity_output("check_source".to_string(), outputs);

        // The bug scenario: using coalesce to safely check length of possibly-null field
        let condition =
            "{{check_source.result.rows[0].doi | coalesce('') | length > 0}}".to_string();
        let result = resolve_template(&condition, &context).unwrap();
        assert_eq!(result, "false");

        // With a non-null DOI
        let mut context2 = TemplateContext::new();
        let outputs2 = vec![ActivityOutput {
            name: "result".to_string(),
            output_type: OutputType::Value,
            value: serde_json::json!({
                "rows": [{"doi": "10.1093/mind/fzab057"}]
            }),
        }];
        context2.add_activity_output("check_source".to_string(), outputs2);

        let result2 = resolve_template(&condition, &context2).unwrap();
        assert_eq!(result2, "true");
    }

    // ============================================================================
    // Activity Status Exposure Tests
    // Regression tests for: docs/bugs/2026-01-10-orchestrator-logic.md
    // ============================================================================

    #[test]
    fn test_activity_status_completed() {
        let mut context = TemplateContext::new();
        context.add_activity_state(
            "fetch".to_string(),
            vec![],
            None,
            0,
            rust_decimal::Decimal::ZERO,
            "completed".to_string(),
        );

        // Status should be accessible via {{activity.status}}
        let value = Value::String("{{fetch.status}}".to_string());
        let result = resolve_template_value(&value, &context).unwrap();
        assert_eq!(result, Value::String("completed".to_string()));

        // Condition checking status should work
        let value = Value::String("{{fetch.status == 'completed'}}".to_string());
        let result = resolve_template_value(&value, &context).unwrap();
        assert_eq!(result, Value::Bool(true));
    }

    #[test]
    fn test_activity_status_skipped() {
        let mut context = TemplateContext::new();
        context.add_activity_state(
            "optional_step".to_string(),
            vec![],
            None,
            0,
            rust_decimal::Decimal::ZERO,
            "skipped".to_string(),
        );

        // Status should show skipped
        let value = Value::String("{{optional_step.status}}".to_string());
        let result = resolve_template_value(&value, &context).unwrap();
        assert_eq!(result, Value::String("skipped".to_string()));

        // Condition checking for completed should be false
        let value = Value::String("{{optional_step.status == 'completed'}}".to_string());
        let result = resolve_template_value(&value, &context).unwrap();
        assert_eq!(result, Value::Bool(false));

        // Condition checking for skipped should be true
        let value = Value::String("{{optional_step.status == 'skipped'}}".to_string());
        let result = resolve_template_value(&value, &context).unwrap();
        assert_eq!(result, Value::Bool(true));
    }

    #[test]
    fn test_activity_status_failed() {
        let mut context = TemplateContext::new();
        context.add_activity_state(
            "risky_step".to_string(),
            vec![],
            None,
            0,
            rust_decimal::Decimal::ZERO,
            "failed".to_string(),
        );

        let value = Value::String("{{risky_step.status}}".to_string());
        let result = resolve_template_value(&value, &context).unwrap();
        assert_eq!(result, Value::String("failed".to_string()));

        let value = Value::String("{{risky_step.status == 'failed'}}".to_string());
        let result = resolve_template_value(&value, &context).unwrap();
        assert_eq!(result, Value::Bool(true));
    }

    #[test]
    fn test_activity_status_not_scheduled() {
        let mut context = TemplateContext::new();
        context.add_activity_state(
            "pending_step".to_string(),
            vec![],
            None,
            0,
            rust_decimal::Decimal::ZERO,
            "not_scheduled".to_string(),
        );

        let value = Value::String("{{pending_step.status}}".to_string());
        let result = resolve_template_value(&value, &context).unwrap();
        assert_eq!(result, Value::String("not_scheduled".to_string()));
    }

    #[test]
    fn test_activity_status_running() {
        let mut context = TemplateContext::new();
        context.add_activity_state(
            "active_step".to_string(),
            vec![],
            None,
            0,
            rust_decimal::Decimal::ZERO,
            "running".to_string(),
        );

        let value = Value::String("{{active_step.status}}".to_string());
        let result = resolve_template_value(&value, &context).unwrap();
        assert_eq!(result, Value::String("running".to_string()));
    }

    #[test]
    fn test_activity_status_pending() {
        let mut context = TemplateContext::new();
        context.add_activity_state(
            "queued_step".to_string(),
            vec![],
            None,
            0,
            rust_decimal::Decimal::ZERO,
            "pending".to_string(),
        );

        let value = Value::String("{{queued_step.status}}".to_string());
        let result = resolve_template_value(&value, &context).unwrap();
        assert_eq!(result, Value::String("pending".to_string()));
    }

    #[test]
    fn test_status_first_pattern_prevents_template_error() {
        // This test verifies the fix for:
        // docs/bugs/2026-01-10-orchestrator-logic.md - Problem 2
        //
        // When a dependency is skipped, accessing its result would cause a template error.
        // The status-first pattern uses short-circuit evaluation to avoid this:
        //   {{dep.status == 'completed' and dep.result.field}}
        //
        // If status != 'completed', the right side is never evaluated.

        let mut context = TemplateContext::new();
        context.add_activity_state(
            "find_bibliography".to_string(),
            vec![], // No outputs - activity was skipped
            None,
            0,
            rust_decimal::Decimal::ZERO,
            "skipped".to_string(),
        );

        // The status-first pattern: check status before accessing result
        // MiniJinja's `and` short-circuits, so if left side is false,
        // the right side (which would error) is never evaluated
        let condition = "{{find_bibliography.status == 'completed' and find_bibliography.result.rows | length > 0}}";
        let value = Value::String(condition.to_string());
        let result = resolve_template_value(&value, &context).unwrap();

        // Should be false (status != 'completed'), not a template error
        assert_eq!(result, Value::Bool(false));
    }

    #[test]
    fn test_status_first_pattern_passes_when_completed() {
        use crate::workflow::{ActivityOutput, OutputType};

        let mut context = TemplateContext::new();
        let outputs = vec![ActivityOutput {
            name: "result".to_string(),
            output_type: OutputType::Value,
            value: serde_json::json!({
                "rows": [{"id": 1}, {"id": 2}]
            }),
        }];
        context.add_activity_state(
            "find_bibliography".to_string(),
            outputs,
            None,
            0,
            rust_decimal::Decimal::ZERO,
            "completed".to_string(),
        );

        // When activity is completed, the full condition is evaluated
        let condition = "{{find_bibliography.status == 'completed' and find_bibliography.result.rows | length > 0}}";
        let value = Value::String(condition.to_string());
        let result = resolve_template_value(&value, &context).unwrap();

        // Should be true (status == 'completed' AND rows.length > 0)
        assert_eq!(result, Value::Bool(true));
    }

    #[test]
    fn test_converging_paths_with_status_conditions() {
        // This test verifies the pattern for converging exclusive paths:
        // docs/bugs/2026-01-10-orchestrator-logic.md - Problem 3
        //
        // When multiple mutually exclusive paths converge, we use status conditions:
        //   depends_on:
        //     - activity_key: path_a
        //       condition: "{{path_a.status == 'completed'}}"
        //     - activity_key: path_b
        //       condition: "{{path_b.status == 'completed'}}"

        // Scenario: path_a completed, path_b was skipped
        let mut context = TemplateContext::new();
        context.add_activity_state(
            "path_a".to_string(),
            vec![],
            None,
            0,
            rust_decimal::Decimal::ZERO,
            "completed".to_string(),
        );
        context.add_activity_state(
            "path_b".to_string(),
            vec![],
            None,
            0,
            rust_decimal::Decimal::ZERO,
            "skipped".to_string(),
        );

        // Condition for path_a dependency (should be true)
        let value = Value::String("{{path_a.status == 'completed'}}".to_string());
        let result = resolve_template_value(&value, &context).unwrap();
        assert_eq!(result, Value::Bool(true));

        // Condition for path_b dependency (should be false)
        let value = Value::String("{{path_b.status == 'completed'}}".to_string());
        let result = resolve_template_value(&value, &context).unwrap();
        assert_eq!(result, Value::Bool(false));

        // At least one path completed - finalize should run
        let value = Value::String(
            "{{path_a.status == 'completed' or path_b.status == 'completed'}}".to_string(),
        );
        let result = resolve_template_value(&value, &context).unwrap();
        assert_eq!(result, Value::Bool(true));
    }

    #[test]
    fn test_converging_paths_both_skipped() {
        // When all converging paths are skipped, the dependent activity should also be skipped

        let mut context = TemplateContext::new();
        context.add_activity_state(
            "path_a".to_string(),
            vec![],
            None,
            0,
            rust_decimal::Decimal::ZERO,
            "skipped".to_string(),
        );
        context.add_activity_state(
            "path_b".to_string(),
            vec![],
            None,
            0,
            rust_decimal::Decimal::ZERO,
            "skipped".to_string(),
        );

        // Neither path completed
        let value = Value::String(
            "{{path_a.status == 'completed' or path_b.status == 'completed'}}".to_string(),
        );
        let result = resolve_template_value(&value, &context).unwrap();
        assert_eq!(result, Value::Bool(false));
    }

    #[test]
    fn test_status_in_activity_context() {
        // Verify status is also available via ACTIVITY context for current activity
        let mut context = TemplateContext::new();
        context.add_activity_state(
            "my_activity".to_string(),
            vec![],
            None,
            3,
            rust_decimal::Decimal::ZERO,
            "running".to_string(),
        );

        // Set current activity context
        context = context.with_current_activity("my_activity".to_string(), None);

        // ACTIVITY.status should be accessible
        let value = Value::String("{{ACTIVITY.status}}".to_string());
        let result = resolve_template_value(&value, &context).unwrap();
        assert_eq!(result, Value::String("running".to_string()));
    }

    #[test]
    fn test_status_with_iteration_scoped_activity() {
        // Status should work with iteration-scoped activities too
        let mut context = TemplateContext::new();
        let mut iteration_outputs = HashMap::new();
        iteration_outputs.insert(
            "results".to_string(),
            vec![serde_json::json!("result1"), serde_json::json!("result2")],
        );

        context.add_activity_state(
            "search".to_string(),
            vec![],
            Some(iteration_outputs),
            2,
            rust_decimal::Decimal::ZERO,
            "completed".to_string(),
        );

        // Status should be accessible
        let value = Value::String("{{search.status}}".to_string());
        let result = resolve_template_value(&value, &context).unwrap();
        assert_eq!(result, Value::String("completed".to_string()));

        // Outputs should still be accessible as arrays
        let value = Value::String("{{search.results | length}}".to_string());
        let result = resolve_template_value(&value, &context).unwrap();
        assert_eq!(result, Value::Number(2.into()));
    }

    // ============================================================================
    // Template Helper Function Tests
    // ============================================================================

    #[test]
    fn test_contains_templates_true() {
        assert!(contains_templates("{{INPUT.x}}"));
        assert!(contains_templates("Hello {{name}}!"));
        assert!(contains_templates("{{a}} and {{b}}"));
    }

    #[test]
    fn test_contains_templates_false() {
        assert!(!contains_templates("no templates here"));
        assert!(!contains_templates("{{ only opening"));
        assert!(!contains_templates("only closing }}"));
        assert!(!contains_templates(""));
    }

    #[test]
    fn test_is_whole_template() {
        assert!(is_whole_template("{{INPUT.x}}"));
        assert!(is_whole_template("  {{INPUT.x}}  "));
        assert!(!is_whole_template("prefix {{INPUT.x}}"));
        assert!(!is_whole_template("{{INPUT.x}} suffix"));
        assert!(!is_whole_template("{{a}} {{b}}")); // nested
    }

    #[test]
    fn test_extract_expression() {
        assert_eq!(extract_expression("{{INPUT.x}}"), "INPUT.x");
        assert_eq!(extract_expression("{{ INPUT.x }}"), "INPUT.x");
        assert_eq!(extract_expression("not a template"), "not a template");
    }

    // ============================================================================
    // Type Conversion Edge Cases
    // ============================================================================

    #[test]
    fn test_serde_json_to_minijinja_null() {
        let mini = serde_json_to_minijinja(&Value::Null);
        assert!(mini.is_none());
    }

    #[test]
    fn test_serde_json_to_minijinja_bool() {
        let mini_true = serde_json_to_minijinja(&Value::Bool(true));
        assert!(mini_true.is_true());
        let mini_false = serde_json_to_minijinja(&Value::Bool(false));
        assert!(!mini_false.is_true());
    }

    #[test]
    fn test_serde_json_to_minijinja_numbers() {
        // Integer
        let mini_int = serde_json_to_minijinja(&serde_json::json!(42));
        assert_eq!(mini_int.as_i64(), Some(42));

        // Float
        let mini_float = serde_json_to_minijinja(&serde_json::json!(3.14));
        assert!(mini_float.to_string().starts_with("3.14"));

        // Large u64 (beyond i64 range)
        let large_u64 = serde_json::json!(u64::MAX);
        let mini_large = serde_json_to_minijinja(&large_u64);
        // Should still produce a value (via u64 path)
        assert!(!mini_large.is_undefined());
    }

    #[test]
    fn test_serde_json_to_minijinja_array() {
        let arr = serde_json::json!([1, "two", true]);
        let mini = serde_json_to_minijinja(&arr);
        let items: Vec<_> = mini.try_iter().unwrap().collect();
        assert_eq!(items.len(), 3);
    }

    #[test]
    fn test_serde_json_to_minijinja_object() {
        let obj = serde_json::json!({"key": "value", "num": 42});
        let mini = serde_json_to_minijinja(&obj);
        let val = mini.get_item(&MiniValue::from("key")).unwrap();
        assert_eq!(val.to_string(), "value");
    }

    #[test]
    fn test_minijinja_to_serde_json_bool() {
        let mini = MiniValue::from(true);
        let json = minijinja_to_serde_json(&mini);
        assert_eq!(json, Value::Bool(true));
    }

    #[test]
    fn test_minijinja_to_serde_json_integer() {
        let mini = MiniValue::from(42_i64);
        let json = minijinja_to_serde_json(&mini);
        assert_eq!(json, serde_json::json!(42));
    }

    #[test]
    fn test_minijinja_to_serde_json_string() {
        let mini = MiniValue::from("hello");
        let json = minijinja_to_serde_json(&mini);
        assert_eq!(json, Value::String("hello".to_string()));
    }

    #[test]
    fn test_minijinja_to_serde_json_undefined() {
        let mini = MiniValue::UNDEFINED;
        let json = minijinja_to_serde_json(&mini);
        assert_eq!(json, Value::Null);
    }

    #[test]
    fn test_minijinja_to_serde_json_none() {
        let mini = MiniValue::from(());
        let json = minijinja_to_serde_json(&mini);
        assert_eq!(json, Value::Null);
    }

    #[test]
    fn test_minijinja_to_serde_json_seq() {
        let mini = MiniValue::from(vec![MiniValue::from(1), MiniValue::from(2)]);
        let json = minijinja_to_serde_json(&mini);
        assert_eq!(json, serde_json::json!([1, 2]));
    }

    // ============================================================================
    // Template Resolution Edge Cases
    // ============================================================================

    #[test]
    fn test_resolve_template_value_passthrough_number() {
        let context = TemplateContext::new();
        let value = serde_json::json!(42);
        let result = resolve_template_value(&value, &context).unwrap();
        assert_eq!(result, serde_json::json!(42));
    }

    #[test]
    fn test_resolve_template_value_passthrough_bool() {
        let context = TemplateContext::new();
        let value = Value::Bool(true);
        let result = resolve_template_value(&value, &context).unwrap();
        assert_eq!(result, Value::Bool(true));
    }

    #[test]
    fn test_resolve_template_value_passthrough_null() {
        let context = TemplateContext::new();
        let value = Value::Null;
        let result = resolve_template_value(&value, &context).unwrap();
        assert_eq!(result, Value::Null);
    }

    #[test]
    fn test_resolve_template_value_no_template_string() {
        let context = TemplateContext::new();
        let value = Value::String("plain string".to_string());
        let result = resolve_template_value(&value, &context).unwrap();
        assert_eq!(result, Value::String("plain string".to_string()));
    }

    #[test]
    fn test_resolve_template_invalid_syntax() {
        let context = TemplateContext::new();
        let value = Value::String("{{a +}}".to_string());
        let result = resolve_template_value(&value, &context);
        assert!(result.is_err());
    }

    #[test]
    fn test_resolve_template_no_templates_returns_string() {
        let context = TemplateContext::new();
        let result = resolve_template("no templates", &context).unwrap();
        assert_eq!(result, "no templates");
    }

    // ============================================================================
    // FILE and FOLDER Context Tests
    // ============================================================================

    #[test]
    fn test_file_context() {
        use crate::workflow::{ActivityOutput, OutputType};

        let mut context = TemplateContext::new();
        let outputs = vec![ActivityOutput {
            name: "document".to_string(),
            output_type: OutputType::File,
            value: Value::String("file:abc123:doc.pdf".to_string()),
        }];
        context.add_activity_output("upload".to_string(), outputs);

        // FILE.upload.document should be accessible
        let value = Value::String("{{FILE.upload.document}}".to_string());
        let result = resolve_template_value(&value, &context).unwrap();
        assert_eq!(result, Value::String("file:abc123:doc.pdf".to_string()));
    }

    #[test]
    fn test_folder_context() {
        use crate::workflow::{ActivityOutput, OutputType};

        let mut context = TemplateContext::new();
        let outputs = vec![ActivityOutput {
            name: "output_dir".to_string(),
            output_type: OutputType::Folder,
            value: Value::String("folder:xyz789:output".to_string()),
        }];
        context.add_activity_output("process".to_string(), outputs);

        // FOLDER.process.output_dir should be accessible
        let value = Value::String("{{FOLDER.process.output_dir}}".to_string());
        let result = resolve_template_value(&value, &context).unwrap();
        assert_eq!(result, Value::String("folder:xyz789:output".to_string()));
    }

    #[test]
    fn test_file_output_non_string_value_ignored() {
        use crate::workflow::{ActivityOutput, OutputType};

        let mut context = TemplateContext::new();
        let outputs = vec![ActivityOutput {
            name: "document".to_string(),
            output_type: OutputType::File,
            value: serde_json::json!(42), // Not a string - should be ignored for FILE context
        }];
        context.add_activity_output("upload".to_string(), outputs);

        // FILE context should not have upload entry since value is not a string
        let value = Value::String("{{FILE.upload.document}}".to_string());
        let result = resolve_template_value(&value, &context);
        // FILE might not even exist if no file outputs added
        assert!(result.is_err() || result.unwrap() == Value::Null);
    }

    // ============================================================================
    // SIGNAL Context Tests
    // ============================================================================

    #[test]
    fn test_signal_context() {
        let mut context = TemplateContext::new();
        context.signal = Some(serde_json::json!({"approved": true, "comment": "looks good"}));

        let value = Value::String("{{SIGNAL.approved}}".to_string());
        let result = resolve_template_value(&value, &context).unwrap();
        assert_eq!(result, Value::Bool(true));

        let value = Value::String("{{SIGNAL.comment}}".to_string());
        let result = resolve_template_value(&value, &context).unwrap();
        assert_eq!(result, Value::String("looks good".to_string()));
    }

    #[test]
    fn test_signal_context_not_set() {
        let context = TemplateContext::new();
        // SIGNAL is not set, should error in strict mode
        let value = Value::String("{{SIGNAL.field}}".to_string());
        let result = resolve_template_value(&value, &context);
        assert!(result.is_err());
    }

    // ============================================================================
    // Remaining Budget Edge Cases
    // ============================================================================

    #[test]
    fn test_remaining_budget_overspent() {
        let mut context = TemplateContext::new();
        context.add_activity_state(
            "expensive".to_string(),
            vec![],
            None,
            0,
            rust_decimal::Decimal::new(1500, 2), // $15.00 spent
            "running".to_string(),
        );

        // Budget limit is $10.00 but already spent $15.00
        context = context.with_current_activity(
            "expensive".to_string(),
            Some(rust_decimal::Decimal::new(1000, 2)),
        );

        let value = Value::String("{{ACTIVITY.remaining_budget_usd}}".to_string());
        let result = resolve_template_value(&value, &context).unwrap();
        // Should be clamped to 0 (not negative)
        assert_eq!(result, Value::String("0".to_string()));
    }

    #[test]
    fn test_remaining_budget_no_limit() {
        let mut context = TemplateContext::new();
        context.add_activity_state(
            "unlimited".to_string(),
            vec![],
            None,
            0,
            rust_decimal::Decimal::new(500, 2),
            "running".to_string(),
        );

        // No budget limit
        context = context.with_current_activity("unlimited".to_string(), None);

        let value = Value::String("{{ACTIVITY.remaining_budget_usd}}".to_string());
        let result = resolve_template_value(&value, &context).unwrap();
        assert_eq!(result, Value::String("0.00".to_string()));
    }

    // ============================================================================
    // Empty Iteration Outputs
    // ============================================================================

    #[test]
    fn test_empty_iteration_outputs() {
        let mut context = TemplateContext::new();
        let iteration_outputs = HashMap::new(); // Empty

        context.add_activity_state(
            "loop_step".to_string(),
            vec![],
            Some(iteration_outputs),
            0,
            rust_decimal::Decimal::ZERO,
            "pending".to_string(),
        );

        // Status should still be accessible
        let value = Value::String("{{loop_step.status}}".to_string());
        let result = resolve_template_value(&value, &context).unwrap();
        assert_eq!(result, Value::String("pending".to_string()));
    }

    // ============================================================================
    // WORKFLOW Context Tests
    // ============================================================================

    #[test]
    fn test_workflow_context() {
        let mut context = TemplateContext::new();
        context
            .workflow
            .insert("id".to_string(), serde_json::json!("wf-123"));
        context
            .workflow
            .insert("name".to_string(), serde_json::json!("my_workflow"));

        let value = Value::String("{{WORKFLOW.id}}".to_string());
        let result = resolve_template_value(&value, &context).unwrap();
        assert_eq!(result, Value::String("wf-123".to_string()));
    }

    // ============================================================================
    // TemplateContext Builder Methods
    // ============================================================================

    #[test]
    fn test_template_context_default() {
        let context = TemplateContext::default();
        assert!(context.inputs.is_empty());
        assert!(context.activity_states.is_empty());
        assert!(context.secrets.is_empty());
        assert!(context.workflow.is_empty());
        assert!(context.current_activity_key.is_none());
        assert!(context.current_activity_budget_limit.is_none());
        assert!(context.signal.is_none());
    }

    #[test]
    fn test_template_context_with_inputs() {
        let mut inputs = HashMap::new();
        inputs.insert("key".to_string(), serde_json::json!("value"));
        let context = TemplateContext::new().with_inputs(inputs);
        assert_eq!(context.inputs.len(), 1);
        assert_eq!(context.inputs.get("key").unwrap(), "value");
    }

    #[test]
    fn test_add_activity_output_legacy() {
        use crate::workflow::{ActivityOutput, OutputType};

        let mut context = TemplateContext::new();
        let outputs = vec![ActivityOutput {
            name: "result".to_string(),
            output_type: OutputType::Value,
            value: serde_json::json!("done"),
        }];
        context.add_activity_output("step".to_string(), outputs);

        // Should default to completed status
        let info = context.activity_states.get("step").unwrap();
        assert_eq!(info.status, "completed");
        assert_eq!(info.iteration, 0);
        assert!(info.iteration_outputs.is_none());
    }

    // ============================================================================
    // TemplateError Tests
    // ============================================================================

    #[test]
    fn test_template_error_display() {
        let err = TemplateError::ReferenceNotFound("INPUT.missing".to_string());
        assert_eq!(
            err.to_string(),
            "Template reference not found: INPUT.missing"
        );

        let err = TemplateError::InvalidSyntax("bad syntax".to_string());
        assert_eq!(err.to_string(), "Invalid template syntax: bad syntax");

        let err = TemplateError::TypeError("expected string".to_string());
        assert_eq!(err.to_string(), "Type error: expected string");

        let err = TemplateError::EvaluationError("eval failed".to_string());
        assert_eq!(err.to_string(), "Template evaluation error: eval failed");
    }

    #[test]
    fn test_template_error_from_minijinja() {
        // Create a minijinja error by attempting invalid evaluation
        let env = Environment::new();
        let expr = env.compile_expression("1 / 0");
        if let Ok(expr) = expr {
            let result = expr.eval(MiniValue::UNDEFINED);
            if let Err(e) = result {
                let template_err: TemplateError = e.into();
                match template_err {
                    TemplateError::EvaluationError(_) => {} // expected
                    _ => panic!("Expected EvaluationError"),
                }
            }
        }
    }

    #[test]
    fn test_activity_without_outputs_has_status() {
        // Even activities without any outputs should have status accessible
        let mut context = TemplateContext::new();
        context.add_activity_state(
            "setup".to_string(),
            vec![], // No outputs
            None,
            0,
            rust_decimal::Decimal::ZERO,
            "completed".to_string(),
        );

        // Status should still be accessible
        let value = Value::String("{{setup.status}}".to_string());
        let result = resolve_template_value(&value, &context).unwrap();
        assert_eq!(result, Value::String("completed".to_string()));
    }
}
