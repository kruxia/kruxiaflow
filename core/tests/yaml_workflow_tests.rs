use kruxiaflow_core::WorkflowDefinition;

#[test]
fn test_parse_yaml_workflow() {
    let yaml = r#"
name: weather_report
activities:
  - key: fetch_weather
    worker: std
    activity_name: http_request
    parameters:
      method: GET
      url: "https://api.weather.gov/gridpoints/TOP/31,80/forecast"
      headers:
        User-Agent: "Kruxia Flow/0.2"
    dependency_of:
      - activity_key: send_notification

  - key: send_notification
    worker: std
    activity_name: http_request
    parameters:
      method: POST
      url: "{{INPUT.webhook_url}}"
      headers:
        Content-Type: "application/json"
      body:
        temperature: "{{fetch_weather.body.properties.periods[0].temperature}}"
        conditions: "{{fetch_weather.body.properties.periods[0].shortForecast}}"
        workflow_id: "{{WORKFLOW.id}}"
"#;

    let workflow = WorkflowDefinition::from_yaml(yaml).unwrap();

    assert_eq!(workflow.name, "weather_report");
    assert_eq!(workflow.activities.len(), 2);

    // Check first activity
    let fetch_weather = &workflow.activities[0];
    assert_eq!(fetch_weather.key, "fetch_weather");
    assert_eq!(fetch_weather.worker, "std");
    assert_eq!(fetch_weather.activity_name.as_deref(), Some("http_request"));

    let params = fetch_weather.parameters.as_ref().unwrap();
    assert_eq!(params["method"], "GET");
    assert_eq!(
        params["url"],
        "https://api.weather.gov/gridpoints/TOP/31,80/forecast"
    );

    // After normalization, dependency_of is cleared and converted to depends_on on target
    // So fetch_weather.dependency_of is None, but send_notification.depends_on contains fetch_weather
    assert!(fetch_weather.dependency_of.is_none());

    // Check second activity has the normalized dependency
    let send_notification = &workflow.activities[1];
    assert_eq!(send_notification.key, "send_notification");
    assert_eq!(send_notification.worker, "std");

    let params = send_notification.parameters.as_ref().unwrap();
    assert_eq!(params["method"], "POST");
    assert_eq!(params["url"], "{{INPUT.webhook_url}}");

    // After normalization, send_notification should depend on fetch_weather
    let depends_on = send_notification.depends_on.as_ref().unwrap();
    assert_eq!(depends_on.len(), 1);
    assert_eq!(depends_on[0].activity_key, "fetch_weather");
}

#[test]
fn test_yaml_roundtrip() {
    let original_yaml = r#"
name: test_workflow
activities:
  - key: step1
    worker: test
    dependency_of:
      - activity_key: step2
  - key: step2
    worker: test
"#;

    let workflow = WorkflowDefinition::from_yaml(original_yaml).unwrap();
    let generated_yaml = workflow.to_yaml().unwrap();

    // Parse the generated YAML to verify it's valid
    let workflow2 = WorkflowDefinition::from_yaml(&generated_yaml).unwrap();

    assert_eq!(workflow.name, workflow2.name);
    assert_eq!(workflow.activities.len(), workflow2.activities.len());
}

#[test]
fn test_invalid_yaml_syntax() {
    let invalid_yaml = r#"
name: test
activities:
  - key: step1
    worker test  # Missing colon
"#;

    let result = WorkflowDefinition::from_yaml(invalid_yaml);
    assert!(result.is_err());
}

#[test]
fn test_yaml_validation_errors() {
    let yaml_with_cycle = r#"
name: test_cycle
activities:
  - key: step1
    worker: test
    dependency_of:
      - activity_key: step2
  - key: step2
    worker: test
    dependency_of:
      - activity_key: step1
"#;

    let result = WorkflowDefinition::from_yaml(yaml_with_cycle);
    assert!(result.is_err());

    // Check that error message mentions cycle
    let err = result.unwrap_err();
    let err_str = format!("{:?}", err);
    assert!(err_str.contains("cycle") || err_str.contains("Cycle"));
}

#[test]
fn test_worker_defaults_to_std() {
    let yaml = r#"
name: default_worker_test
activities:
  - key: step1
    activity_name: http_request
    parameters:
      method: GET
      url: "https://example.com"
"#;

    let workflow = WorkflowDefinition::from_yaml(yaml).unwrap();
    assert_eq!(workflow.activities[0].worker, "std");
}

#[test]
fn test_explicit_worker_overrides_default() {
    let yaml = r#"
name: explicit_worker_test
activities:
  - key: step1
    worker: custom-python
    activity_name: run_script
"#;

    let workflow = WorkflowDefinition::from_yaml(yaml).unwrap();
    assert_eq!(workflow.activities[0].worker, "custom-python");
}
