use super::{ActivityOutput, OutputType};
use minijinja::{Environment, Value as MiniValue};
use serde_json::Value;
use std::collections::HashMap;
use thiserror::Error;

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

/// Template context for resolving expressions
#[derive(Debug, Clone)]
pub struct TemplateContext {
    /// Workflow inputs provided at runtime
    pub inputs: HashMap<String, Value>,

    /// Activity outputs (key = activity_key, value = structured outputs with type info)
    pub outputs: HashMap<String, Vec<ActivityOutput>>,

    /// Secrets (e.g., API keys)
    pub secrets: HashMap<String, String>,

    /// Workflow-level variables
    pub workflow: HashMap<String, Value>,
}

impl TemplateContext {
    pub fn new() -> Self {
        Self {
            inputs: HashMap::new(),
            outputs: HashMap::new(),
            secrets: HashMap::new(),
            workflow: HashMap::new(),
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

    pub fn add_activity_output(&mut self, activity_key: String, outputs: Vec<ActivityOutput>) {
        self.outputs.insert(activity_key, outputs);
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

        // Add activity outputs as top-level keys (for Value-type outputs)
        // Also build FILE and FOLDER context maps
        let mut file_map: HashMap<String, HashMap<String, String>> = HashMap::new();
        let mut folder_map: HashMap<String, HashMap<String, String>> = HashMap::new();

        for (activity_key, outputs) in &self.outputs {
            let mut value_outputs = HashMap::new();
            let mut file_outputs = HashMap::new();
            let mut folder_outputs = HashMap::new();

            for output in outputs {
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
                    serde_json_to_minijinja(&Value::Object(value_outputs.into_iter().collect())),
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
        && trimmed[2..trimmed.len() - 2].find("{{").is_none() // No nested {{
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
                Ok(minijinja_to_serde_json(&mini_result))
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
            let resolved: Result<serde_json::Map<String, Value>, _> = obj
                .iter()
                .map(|(k, v)| {
                    resolve_value_recursive(v, env, context)
                        .map(|resolved_v| (k.clone(), resolved_v))
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
}
