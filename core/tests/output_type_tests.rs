use serde_json::json;
use kruxiaflow_core::workflow::{ActivityOutput, ActivityOutputDefinition, OutputType};

#[test]
fn test_output_type_default() {
    let output_type = OutputType::default();
    assert_eq!(output_type, OutputType::Value);
}

#[test]
fn test_output_type_equality() {
    assert_eq!(OutputType::Value, OutputType::Value);
    assert_eq!(OutputType::File, OutputType::File);
    assert_eq!(OutputType::Folder, OutputType::Folder);
    assert_ne!(OutputType::Value, OutputType::File);
    assert_ne!(OutputType::File, OutputType::Folder);
}

#[test]
fn test_output_type_serialization() {
    // Test Value type
    let value_type = OutputType::Value;
    let serialized = serde_json::to_string(&value_type).expect("Failed to serialize");
    assert_eq!(serialized, r#""value""#);

    // Test File type
    let file_type = OutputType::File;
    let serialized = serde_json::to_string(&file_type).expect("Failed to serialize");
    assert_eq!(serialized, r#""file""#);

    // Test Folder type
    let folder_type = OutputType::Folder;
    let serialized = serde_json::to_string(&folder_type).expect("Failed to serialize");
    assert_eq!(serialized, r#""folder""#);
}

#[test]
fn test_output_type_deserialization() {
    // Test Value type
    let deserialized: OutputType =
        serde_json::from_str(r#""value""#).expect("Failed to deserialize");
    assert_eq!(deserialized, OutputType::Value);

    // Test File type
    let deserialized: OutputType =
        serde_json::from_str(r#""file""#).expect("Failed to deserialize");
    assert_eq!(deserialized, OutputType::File);

    // Test Folder type
    let deserialized: OutputType =
        serde_json::from_str(r#""folder""#).expect("Failed to deserialize");
    assert_eq!(deserialized, OutputType::Folder);
}

#[test]
fn test_activity_output_definition_default_type() {
    let yaml = r#"
name: result
"#;

    let definition: ActivityOutputDefinition =
        serde_yaml::from_str(yaml).expect("Failed to parse YAML");

    assert_eq!(definition.name, "result");
    assert_eq!(definition.output_type, OutputType::Value);
}

#[test]
fn test_activity_output_definition_explicit_value_type() {
    let yaml = r#"
name: result
type: value
"#;

    let definition: ActivityOutputDefinition =
        serde_yaml::from_str(yaml).expect("Failed to parse YAML");

    assert_eq!(definition.name, "result");
    assert_eq!(definition.output_type, OutputType::Value);
}

#[test]
fn test_activity_output_definition_file_type() {
    let yaml = r#"
name: document
type: file
"#;

    let definition: ActivityOutputDefinition =
        serde_yaml::from_str(yaml).expect("Failed to parse YAML");

    assert_eq!(definition.name, "document");
    assert_eq!(definition.output_type, OutputType::File);
}

#[test]
fn test_activity_output_definition_folder_type() {
    let yaml = r#"
name: output_dir
type: folder
"#;

    let definition: ActivityOutputDefinition =
        serde_yaml::from_str(yaml).expect("Failed to parse YAML");

    assert_eq!(definition.name, "output_dir");
    assert_eq!(definition.output_type, OutputType::Folder);
}

#[test]
fn test_activity_output_definition_list() {
    let yaml = r#"
- name: status
  type: value
- name: document
  type: file
- name: logs
  type: folder
"#;

    let definitions: Vec<ActivityOutputDefinition> =
        serde_yaml::from_str(yaml).expect("Failed to parse YAML");

    assert_eq!(definitions.len(), 3);
    assert_eq!(definitions[0].name, "status");
    assert_eq!(definitions[0].output_type, OutputType::Value);
    assert_eq!(definitions[1].name, "document");
    assert_eq!(definitions[1].output_type, OutputType::File);
    assert_eq!(definitions[2].name, "logs");
    assert_eq!(definitions[2].output_type, OutputType::Folder);
}

#[test]
fn test_activity_output_value_helper() {
    let output = ActivityOutput::value("result", json!({"status": "success"}));

    assert_eq!(output.name, "result");
    assert_eq!(output.output_type, OutputType::Value);
    assert_eq!(output.value, json!({"status": "success"}));
}

#[test]
fn test_activity_output_file_helper() {
    let reference = "postgres://workflow-123/fetch_doc/document.pdf";
    let output = ActivityOutput::file("document", reference);

    assert_eq!(output.name, "document");
    assert_eq!(output.output_type, OutputType::File);
    assert_eq!(output.value, json!(reference));
}

#[test]
fn test_activity_output_folder_helper() {
    let reference = "postgres://workflow-123/process_docs/output/";
    let output = ActivityOutput::folder("output_dir", reference);

    assert_eq!(output.name, "output_dir");
    assert_eq!(output.output_type, OutputType::Folder);
    assert_eq!(output.value, json!(reference));
}

#[test]
fn test_activity_output_serialization() {
    let output = ActivityOutput::value("result", json!({"count": 42}));
    let serialized = serde_json::to_value(&output).expect("Failed to serialize");

    assert_eq!(
        serialized,
        json!({
            "name": "result",
            "type": "value",
            "value": {"count": 42}
        })
    );
}

#[test]
fn test_activity_output_file_serialization() {
    let output = ActivityOutput::file("document", "postgres://workflow/activity/file.pdf");
    let serialized = serde_json::to_value(&output).expect("Failed to serialize");

    assert_eq!(
        serialized,
        json!({
            "name": "document",
            "type": "file",
            "value": "postgres://workflow/activity/file.pdf"
        })
    );
}

#[test]
fn test_activity_output_deserialization() {
    let json = json!({
        "name": "result",
        "type": "value",
        "value": {"count": 42}
    });

    let output: ActivityOutput = serde_json::from_value(json).expect("Failed to deserialize");

    assert_eq!(output.name, "result");
    assert_eq!(output.output_type, OutputType::Value);
    assert_eq!(output.value, json!({"count": 42}));
}

#[test]
fn test_activity_output_file_deserialization() {
    let json = json!({
        "name": "document",
        "type": "file",
        "value": "postgres://workflow/activity/file.pdf"
    });

    let output: ActivityOutput = serde_json::from_value(json).expect("Failed to deserialize");

    assert_eq!(output.name, "document");
    assert_eq!(output.output_type, OutputType::File);
    assert_eq!(output.value, json!("postgres://workflow/activity/file.pdf"));
}

#[test]
fn test_activity_output_list_serialization() {
    let outputs = vec![
        ActivityOutput::value("status", json!("success")),
        ActivityOutput::file("document", "postgres://workflow/activity/doc.pdf"),
        ActivityOutput::value("count", json!(42)),
    ];

    let serialized = serde_json::to_value(&outputs).expect("Failed to serialize");

    assert_eq!(
        serialized,
        json!([
            {
                "name": "status",
                "type": "value",
                "value": "success"
            },
            {
                "name": "document",
                "type": "file",
                "value": "postgres://workflow/activity/doc.pdf"
            },
            {
                "name": "count",
                "type": "value",
                "value": 42
            }
        ])
    );
}

#[test]
fn test_activity_output_equality() {
    let output1 = ActivityOutput::value("result", json!({"count": 42}));
    let output2 = ActivityOutput::value("result", json!({"count": 42}));
    let output3 = ActivityOutput::value("result", json!({"count": 43}));

    assert_eq!(output1, output2);
    assert_ne!(output1, output3);
}

#[test]
fn test_activity_output_definition_serialization() {
    let definition = ActivityOutputDefinition {
        name: "document".to_string(),
        output_type: OutputType::File,
    };

    let serialized = serde_json::to_value(&definition).expect("Failed to serialize");

    assert_eq!(
        serialized,
        json!({
            "name": "document",
            "type": "file"
        })
    );
}

#[test]
fn test_activity_output_definition_deserialization() {
    let json = json!({
        "name": "document",
        "type": "file"
    });

    let definition: ActivityOutputDefinition =
        serde_json::from_value(json).expect("Failed to deserialize");

    assert_eq!(definition.name, "document");
    assert_eq!(definition.output_type, OutputType::File);
}

#[test]
fn test_activity_output_definition_default_deserialization() {
    let json = json!({
        "name": "result"
    });

    let definition: ActivityOutputDefinition =
        serde_json::from_value(json).expect("Failed to deserialize");

    assert_eq!(definition.name, "result");
    assert_eq!(definition.output_type, OutputType::Value);
}

#[test]
fn test_mixed_output_types() {
    // Test that we can work with a mix of value, file, and folder outputs
    let outputs = vec![
        ActivityOutput {
            name: "status".to_string(),
            output_type: OutputType::Value,
            value: json!("success"),
        },
        ActivityOutput {
            name: "config_file".to_string(),
            output_type: OutputType::File,
            value: json!("postgres://wf/act/config.json"),
        },
        ActivityOutput {
            name: "data_folder".to_string(),
            output_type: OutputType::Folder,
            value: json!("postgres://wf/act/data/"),
        },
        ActivityOutput {
            name: "count".to_string(),
            output_type: OutputType::Value,
            value: json!(42),
        },
    ];

    // Serialize and deserialize
    let serialized = serde_json::to_value(&outputs).expect("Failed to serialize");
    let deserialized: Vec<ActivityOutput> =
        serde_json::from_value(serialized).expect("Failed to deserialize");

    assert_eq!(deserialized.len(), 4);
    assert_eq!(deserialized[0].output_type, OutputType::Value);
    assert_eq!(deserialized[1].output_type, OutputType::File);
    assert_eq!(deserialized[2].output_type, OutputType::Folder);
    assert_eq!(deserialized[3].output_type, OutputType::Value);
}

#[test]
fn test_yaml_workflow_with_file_outputs() {
    // Test parsing a realistic workflow YAML with file outputs
    let yaml = r#"
activities:
  fetch_doc:
    activity: http_request
    parameters:
      method: GET
      url: "https://example.com/document.pdf"
    outputs:
      - name: document
        type: file
      - name: status_code
        type: value

  process_doc:
    activity: python_script
    parameters:
      script: "process.py"
      input: "{{FILE.fetch_doc.document}}"
    outputs:
      - name: result
        type: file
      - name: log
        type: value
"#;

    let parsed: serde_yaml::Value = serde_yaml::from_str(yaml).expect("Failed to parse YAML");

    // Verify fetch_doc outputs
    let fetch_outputs = parsed["activities"]["fetch_doc"]["outputs"]
        .as_sequence()
        .expect("outputs should be a sequence");
    assert_eq!(fetch_outputs.len(), 2);

    let doc_output: ActivityOutputDefinition =
        serde_yaml::from_value(fetch_outputs[0].clone()).expect("Failed to parse output");
    assert_eq!(doc_output.name, "document");
    assert_eq!(doc_output.output_type, OutputType::File);

    let status_output: ActivityOutputDefinition =
        serde_yaml::from_value(fetch_outputs[1].clone()).expect("Failed to parse output");
    assert_eq!(status_output.name, "status_code");
    assert_eq!(status_output.output_type, OutputType::Value);

    // Verify process_doc outputs
    let process_outputs = parsed["activities"]["process_doc"]["outputs"]
        .as_sequence()
        .expect("outputs should be a sequence");
    assert_eq!(process_outputs.len(), 2);

    let result_output: ActivityOutputDefinition =
        serde_yaml::from_value(process_outputs[0].clone()).expect("Failed to parse output");
    assert_eq!(result_output.name, "result");
    assert_eq!(result_output.output_type, OutputType::File);
}
