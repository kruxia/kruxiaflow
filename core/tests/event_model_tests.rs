// core/tests/event_model_tests.rs
//! Unit tests for event models (serialization, deserialization, Display impls)

use chrono::Utc;
use serde_json::json;
use kruxiaflow_core::events::models::*;
use uuid::Uuid;

// Helper to create a test UUID
fn test_uuid() -> Uuid {
    Uuid::now_v7()
}

// ============================================================================
// WorkflowEventType Tests
// ============================================================================

#[test]
fn test_workflow_event_type_display() {
    assert_eq!(
        WorkflowEventType::WorkflowCreated.to_string(),
        "WorkflowCreated"
    );
    assert_eq!(
        WorkflowEventType::WorkflowUpdated.to_string(),
        "WorkflowUpdated"
    );
    assert_eq!(
        WorkflowEventType::ActivityScheduled.to_string(),
        "ActivityScheduled"
    );
    assert_eq!(
        WorkflowEventType::ActivityCompleted.to_string(),
        "ActivityCompleted"
    );
    assert_eq!(
        WorkflowEventType::ActivityFailed.to_string(),
        "ActivityFailed"
    );
    assert_eq!(
        WorkflowEventType::WorkflowCompleted.to_string(),
        "WorkflowCompleted"
    );
    assert_eq!(
        WorkflowEventType::WorkflowFailed.to_string(),
        "WorkflowFailed"
    );
}

#[test]
fn test_workflow_event_type_serialization() {
    let event_type = WorkflowEventType::WorkflowCreated;
    let json = serde_json::to_string(&event_type).unwrap();
    assert!(json.contains("WorkflowCreated"));

    let event_type = WorkflowEventType::ActivityCompleted;
    let json = serde_json::to_string(&event_type).unwrap();
    assert!(json.contains("ActivityCompleted"));
}

#[test]
fn test_workflow_event_type_deserialization() {
    let json = r#""WorkflowCreated""#;
    let event_type: WorkflowEventType = serde_json::from_str(json).unwrap();
    assert_eq!(event_type, WorkflowEventType::WorkflowCreated);

    let json = r#""ActivityCompleted""#;
    let event_type: WorkflowEventType = serde_json::from_str(json).unwrap();
    assert_eq!(event_type, WorkflowEventType::ActivityCompleted);
}

#[test]
fn test_workflow_event_type_all_variants() {
    let variants = vec![
        WorkflowEventType::WorkflowCreated,
        WorkflowEventType::WorkflowUpdated,
        WorkflowEventType::ActivityScheduled,
        WorkflowEventType::ActivityCompleted,
        WorkflowEventType::ActivityFailed,
        WorkflowEventType::WorkflowCompleted,
        WorkflowEventType::WorkflowFailed,
    ];

    for variant in variants {
        // Test round-trip serialization
        let json = serde_json::to_string(&variant).unwrap();
        let deserialized: WorkflowEventType = serde_json::from_str(&json).unwrap();
        assert_eq!(variant, deserialized);
    }
}

// ============================================================================
// WorkflowStatus Tests
// ============================================================================

#[test]
fn test_workflow_status_display() {
    assert_eq!(WorkflowStatus::Running.to_string(), "running");
    assert_eq!(WorkflowStatus::Completed.to_string(), "completed");
    assert_eq!(WorkflowStatus::Failed.to_string(), "failed");
    assert_eq!(WorkflowStatus::Paused.to_string(), "paused");
}

#[test]
fn test_workflow_status_serialization() {
    let status = WorkflowStatus::Running;
    let json = serde_json::to_string(&status).unwrap();
    assert_eq!(json, "\"running\"");

    let status = WorkflowStatus::Completed;
    let json = serde_json::to_string(&status).unwrap();
    assert_eq!(json, "\"completed\"");
}

#[test]
fn test_workflow_status_deserialization() {
    let json = "\"running\"";
    let status: WorkflowStatus = serde_json::from_str(json).unwrap();
    assert_eq!(status, WorkflowStatus::Running);

    let json = "\"completed\"";
    let status: WorkflowStatus = serde_json::from_str(json).unwrap();
    assert_eq!(status, WorkflowStatus::Completed);
}

#[test]
fn test_workflow_status_all_variants() {
    let variants = vec![
        WorkflowStatus::Running,
        WorkflowStatus::Completed,
        WorkflowStatus::Failed,
        WorkflowStatus::Paused,
    ];

    for variant in variants {
        // Test round-trip serialization
        let json = serde_json::to_string(&variant).unwrap();
        let deserialized: WorkflowStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(variant, deserialized);
    }
}

// ============================================================================
// NewWorkflowEvent Tests
// ============================================================================

#[test]
fn test_new_workflow_event_serialization() {
    let workflow_id = test_uuid();
    let event = NewWorkflowEvent {
        workflow_id,
        event_type: WorkflowEventType::WorkflowCreated,
        activity_key: Some("activity1".to_string()),
        payload: json!({"key": "value"}),
        iteration: None,
    };

    let json = serde_json::to_string(&event).unwrap();
    assert!(json.contains(&workflow_id.to_string()));
    assert!(json.contains("WorkflowCreated"));
    assert!(json.contains("activity1"));
    assert!(json.contains("value"));
}

#[test]
fn test_new_workflow_event_deserialization() {
    let workflow_id = test_uuid();
    let json = json!({
        "workflow_id": workflow_id,
        "event_type": "WorkflowCreated",
        "activity_key": "activity1",
        "payload": {"key": "value"}
    });

    let event: NewWorkflowEvent = serde_json::from_value(json).unwrap();
    assert_eq!(event.workflow_id, workflow_id);
    assert_eq!(event.event_type, WorkflowEventType::WorkflowCreated);
    assert_eq!(event.activity_key, Some("activity1".to_string()));
    assert_eq!(event.payload["key"], "value");
}

#[test]
fn test_new_workflow_event_without_activity_key() {
    let workflow_id = test_uuid();
    let event = NewWorkflowEvent {
        workflow_id,
        event_type: WorkflowEventType::WorkflowCreated,
        activity_key: None,
        payload: json!({}),
        iteration: None,
    };

    let json = serde_json::to_string(&event).unwrap();
    let deserialized: NewWorkflowEvent = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.activity_key, None);
}

#[test]
fn test_new_workflow_event_clone() {
    let workflow_id = test_uuid();
    let event1 = NewWorkflowEvent {
        workflow_id,
        event_type: WorkflowEventType::ActivityCompleted,
        activity_key: Some("act1".to_string()),
        payload: json!({"result": "success"}),
        iteration: None,
    };

    let event2 = event1.clone();
    assert_eq!(event1.workflow_id, event2.workflow_id);
    assert_eq!(event1.event_type, event2.event_type);
    assert_eq!(event1.activity_key, event2.activity_key);
}

// ============================================================================
// WorkflowEvent Tests
// ============================================================================

#[test]
fn test_workflow_event_serialization() {
    let id = test_uuid();
    let workflow_id = test_uuid();
    let event = WorkflowEvent {
        id,
        workflow_id,
        event_type: WorkflowEventType::ActivityCompleted,
        activity_key: Some("activity1".to_string()),
        payload: json!({"result": "ok"}),
        timestamp: Utc::now(),
        iteration: None,
    };

    let json = serde_json::to_string(&event).unwrap();
    assert!(json.contains(&id.to_string()));
    assert!(json.contains(&workflow_id.to_string()));
    assert!(json.contains("ActivityCompleted"));
}

#[test]
fn test_workflow_event_deserialization() {
    let id = test_uuid();
    let workflow_id = test_uuid();
    let timestamp = Utc::now();

    let json = json!({
        "id": id,
        "workflow_id": workflow_id,
        "event_type": "ActivityCompleted",
        "activity_key": "activity1",
        "payload": {"result": "ok"},
        "timestamp": timestamp
    });

    let event: WorkflowEvent = serde_json::from_value(json).unwrap();
    assert_eq!(event.id, id);
    assert_eq!(event.workflow_id, workflow_id);
    assert_eq!(event.event_type, WorkflowEventType::ActivityCompleted);
}

// ============================================================================
// WorkflowDefinition Tests
// ============================================================================

#[test]
fn test_workflow_definition_serialization() {
    let id = test_uuid();
    let def = WorkflowDefinition {
        id,
        name: "test-workflow".to_string(),
        version: "1.0.0".to_string(),
        activities: vec![],
    };

    let json = serde_json::to_string(&def).unwrap();
    assert!(json.contains(&id.to_string()));
    assert!(json.contains("test-workflow"));
    assert!(json.contains("1.0.0"));
}

#[test]
fn test_workflow_definition_with_activities() {
    let id = test_uuid();
    let activity = ActivityDefinition {
        key: "act1".to_string(),
        worker: "builtin".to_string(),
        activity_name: "TestActivity".to_string(),
        parameters: json!({"param": "value"}),
        settings: None,
        depends_on: None,
        dependency_of: None,
        output_definitions: None,
    };

    let def = WorkflowDefinition {
        id,
        name: "test".to_string(),
        version: "1.0.0".to_string(),
        activities: vec![activity],
    };

    let json = serde_json::to_string(&def).unwrap();
    assert!(json.contains("act1"));
    assert!(json.contains("TestActivity"));
}

// ============================================================================
// ActivityDefinition Tests
// ============================================================================

#[test]
fn test_activity_definition_serialization() {
    let activity = ActivityDefinition {
        key: "activity1".to_string(),
        worker: "test".to_string(),
        activity_name: "TestActivity".to_string(),
        parameters: json!({"input": "data"}),
        settings: None,
        depends_on: None,
        dependency_of: None,
        output_definitions: None,
    };

    let json = serde_json::to_string(&activity).unwrap();
    assert!(json.contains("activity1"));
    assert!(json.contains("test"));
    assert!(json.contains("TestActivity"));
    assert!(json.contains("input"));
}

#[test]
fn test_activity_definition_with_dependencies() {
    let activity = ActivityDefinition {
        key: "act2".to_string(),
        worker: "builtin".to_string(),
        activity_name: "DependentActivity".to_string(),
        parameters: json!({}),
        settings: None,
        depends_on: Some(vec![DependencyEdge {
            activity_key: "act1".to_string(),
            conditions: None,
        }]),
        dependency_of: Some(vec![DependencyEdge {
            activity_key: "act3".to_string(),
            conditions: Some(vec!["success".to_string()]),
        }]),
        output_definitions: None,
    };

    let json = serde_json::to_string(&activity).unwrap();
    assert!(json.contains("act1"));
    assert!(json.contains("act3"));
    assert!(json.contains("success"));
}

#[test]
fn test_activity_definition_clone() {
    let activity1 = ActivityDefinition {
        key: "act1".to_string(),
        worker: "ns".to_string(),
        activity_name: "Activity".to_string(),
        parameters: json!({"key": "value"}),
        settings: None,
        depends_on: None,
        dependency_of: None,
        output_definitions: None,
    };

    let activity2 = activity1.clone();
    assert_eq!(activity1.key, activity2.key);
    assert_eq!(activity1.worker, activity2.worker);
    assert_eq!(activity1.activity_name, activity2.activity_name);
}

// ============================================================================
// DependencyEdge Tests
// ============================================================================

#[test]
fn test_dependency_edge_without_conditions() {
    let edge = DependencyEdge {
        activity_key: "act1".to_string(),
        conditions: None,
    };

    let json = serde_json::to_string(&edge).unwrap();
    assert!(json.contains("act1"));
    assert!(!json.contains("conditions"));
}

#[test]
fn test_dependency_edge_with_conditions() {
    let edge = DependencyEdge {
        activity_key: "act1".to_string(),
        conditions: Some(vec!["success".to_string(), "result > 0".to_string()]),
    };

    let json = serde_json::to_string(&edge).unwrap();
    assert!(json.contains("act1"));
    assert!(json.contains("success"));
    assert!(json.contains("result > 0"));
}

#[test]
fn test_dependency_edge_clone() {
    let edge1 = DependencyEdge {
        activity_key: "act1".to_string(),
        conditions: Some(vec!["cond1".to_string()]),
    };

    let edge2 = edge1.clone();
    assert_eq!(edge1.activity_key, edge2.activity_key);
    assert_eq!(edge1.conditions, edge2.conditions);
}
