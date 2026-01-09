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
    ) {
        self.activity_states.insert(
            activity_key,
            ActivityContextInfo {
                outputs,
                iteration_outputs,
                iteration,
                accumulated_cost_usd,
            },
        );
    }

    /// Legacy method for backward compatibility
    pub fn add_activity_output(&mut self, activity_key: String, outputs: Vec<ActivityOutput>) {
        self.add_activity_state(activity_key, outputs, None, 0, rust_decimal::Decimal::ZERO);
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
                let value_map: serde_json::Map<String, Value> = iteration_outputs
                    .iter()
                    .map(|(name, values)| (name.clone(), Value::Array(values.clone())))
                    .collect();

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

                // Add value outputs as top-level activity key
                if !value_outputs.is_empty() {
                    context_map.insert(
                        activity_key.clone(),
                        serde_json_to_minijinja(&Value::Object(
                            value_outputs.into_iter().collect(),
                        )),
                    );
                }

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
            });

            context_map.insert(
                "ACTIVITY".to_string(),
                serde_json_to_minijinja(&activity_context),
            );
        }

        MiniValue::from_object(context_map)
    }
}

impl Default for TemplateContext {
    fn default() -> Self {
        Self::new()
    }
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
    let mut env = Environment::new();
    // Configure strict undefined behavior - errors on undefined variables
    env.set_undefined_behavior(minijinja::UndefinedBehavior::Strict);
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
    let mut env = Environment::new();
    // Configure strict undefined behavior - errors on undefined variables
    env.set_undefined_behavior(minijinja::UndefinedBehavior::Strict);
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
}
