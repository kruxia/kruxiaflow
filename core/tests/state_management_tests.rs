use chrono::Utc;
use serde_json::json;
use streamflow_core::events::{WorkflowEvent, WorkflowEventType, WorkflowStatus};
use streamflow_core::orchestrator::{
    ActivityState, WorkflowActivityStatus, WorkflowState, apply_event_to_state,
};
use uuid::Uuid;

#[test]
fn test_apply_workflow_created_event() {
    let workflow_id = Uuid::now_v7();
    let mut state = WorkflowState {
        workflow_id,
        workflow_type: "test_workflow".to_string(),
        status: WorkflowStatus::Running,
        activities: Default::default(),
        state_data: json!({}),
    };

    let event = WorkflowEvent {
        id: Uuid::now_v7(),
        workflow_id,
        event_type: WorkflowEventType::WorkflowCreated,
        activity_key: None,
        payload: json!({"state_data": {"custom": "value"}}),
        timestamp: Utc::now(),
    };

    apply_event_to_state(&mut state, &event).expect("Failed to apply event");

    assert_eq!(state.state_data, json!({"custom": "value"}));
}

#[test]
fn test_apply_activity_scheduled_event() {
    let workflow_id = Uuid::now_v7();
    let mut state = WorkflowState {
        workflow_id,
        workflow_type: "test_workflow".to_string(),
        status: WorkflowStatus::Running,
        activities: vec![(
            "activity1".to_string(),
            ActivityState {
                key: "activity1".to_string(),
                status: WorkflowActivityStatus::NotScheduled,
                outputs: None,
                error: None,
                started_at: None,
                completed_at: None,
                retry_count: 0,
            },
        )]
        .into_iter()
        .collect(),
        state_data: json!({}),
    };

    let event = WorkflowEvent {
        id: Uuid::now_v7(),
        workflow_id,
        event_type: WorkflowEventType::ActivityScheduled,
        activity_key: Some("activity1".to_string()),
        payload: json!({}),
        timestamp: Utc::now(),
    };

    apply_event_to_state(&mut state, &event).expect("Failed to apply event");

    let activity = state.activities.get("activity1").unwrap();
    assert_eq!(activity.status, WorkflowActivityStatus::Pending);
    assert!(activity.started_at.is_some());
}

#[test]
fn test_apply_activity_completed_event() {
    let workflow_id = Uuid::now_v7();
    let mut state = WorkflowState {
        workflow_id,
        workflow_type: "test_workflow".to_string(),
        status: WorkflowStatus::Running,
        activities: vec![(
            "activity1".to_string(),
            ActivityState {
                key: "activity1".to_string(),
                status: WorkflowActivityStatus::Pending,
                outputs: None,
                error: None,
                started_at: Some(Utc::now()),
                completed_at: None,
                retry_count: 0,
            },
        )]
        .into_iter()
        .collect(),
        state_data: json!({}),
    };

    let event = WorkflowEvent {
        id: Uuid::now_v7(),
        workflow_id,
        event_type: WorkflowEventType::ActivityCompleted,
        activity_key: Some("activity1".to_string()),
        payload: json!({"outputs": {"result": "success"}}),
        timestamp: Utc::now(),
    };

    apply_event_to_state(&mut state, &event).expect("Failed to apply event");

    let activity = state.activities.get("activity1").unwrap();
    assert_eq!(activity.status, WorkflowActivityStatus::Completed);
    assert_eq!(activity.outputs, Some(json!({"result": "success"})));
    assert!(activity.completed_at.is_some());
}

#[test]
fn test_apply_activity_failed_event() {
    let workflow_id = Uuid::now_v7();
    let mut state = WorkflowState {
        workflow_id,
        workflow_type: "test_workflow".to_string(),
        status: WorkflowStatus::Running,
        activities: vec![(
            "activity1".to_string(),
            ActivityState {
                key: "activity1".to_string(),
                status: WorkflowActivityStatus::Pending,
                outputs: None,
                error: None,
                started_at: Some(Utc::now()),
                completed_at: None,
                retry_count: 0,
            },
        )]
        .into_iter()
        .collect(),
        state_data: json!({}),
    };

    let event = WorkflowEvent {
        id: Uuid::now_v7(),
        workflow_id,
        event_type: WorkflowEventType::ActivityFailed,
        activity_key: Some("activity1".to_string()),
        payload: json!({"error": "Connection timeout"}),
        timestamp: Utc::now(),
    };

    apply_event_to_state(&mut state, &event).expect("Failed to apply event");

    let activity = state.activities.get("activity1").unwrap();
    assert_eq!(activity.status, WorkflowActivityStatus::Failed);
    assert_eq!(activity.error, Some("Connection timeout".to_string()));
    assert!(activity.completed_at.is_some());
}

#[test]
fn test_apply_workflow_completed_event() {
    let workflow_id = Uuid::now_v7();
    let mut state = WorkflowState {
        workflow_id,
        workflow_type: "test_workflow".to_string(),
        status: WorkflowStatus::Running,
        activities: Default::default(),
        state_data: json!({}),
    };

    let event = WorkflowEvent {
        id: Uuid::now_v7(),
        workflow_id,
        event_type: WorkflowEventType::WorkflowCompleted,
        activity_key: None,
        payload: json!({}),
        timestamp: Utc::now(),
    };

    apply_event_to_state(&mut state, &event).expect("Failed to apply event");

    assert_eq!(state.status, WorkflowStatus::Completed);
}

#[test]
fn test_apply_workflow_failed_event() {
    let workflow_id = Uuid::now_v7();
    let mut state = WorkflowState {
        workflow_id,
        workflow_type: "test_workflow".to_string(),
        status: WorkflowStatus::Running,
        activities: Default::default(),
        state_data: json!({}),
    };

    let event = WorkflowEvent {
        id: Uuid::now_v7(),
        workflow_id,
        event_type: WorkflowEventType::WorkflowFailed,
        activity_key: None,
        payload: json!({"reason": "Activity timeout"}),
        timestamp: Utc::now(),
    };

    apply_event_to_state(&mut state, &event).expect("Failed to apply event");

    assert_eq!(state.status, WorkflowStatus::Failed);
}

#[test]
fn test_apply_multiple_events_sequential() {
    let workflow_id = Uuid::now_v7();
    let mut state = WorkflowState {
        workflow_id,
        workflow_type: "test_workflow".to_string(),
        status: WorkflowStatus::Running,
        activities: vec![
            (
                "activity1".to_string(),
                ActivityState {
                    key: "activity1".to_string(),
                    status: WorkflowActivityStatus::NotScheduled,
                    outputs: None,
                    error: None,
                    started_at: None,
                    completed_at: None,
                    retry_count: 0,
                },
            ),
            (
                "activity2".to_string(),
                ActivityState {
                    key: "activity2".to_string(),
                    status: WorkflowActivityStatus::NotScheduled,
                    outputs: None,
                    error: None,
                    started_at: None,
                    completed_at: None,
                    retry_count: 0,
                },
            ),
        ]
        .into_iter()
        .collect(),
        state_data: json!({}),
    };

    // Apply sequence of events
    let events = vec![
        WorkflowEvent {
            id: Uuid::now_v7(),
            workflow_id,
            event_type: WorkflowEventType::ActivityScheduled,
            activity_key: Some("activity1".to_string()),
            payload: json!({}),
            timestamp: Utc::now(),
        },
        WorkflowEvent {
            id: Uuid::now_v7(),
            workflow_id,
            event_type: WorkflowEventType::ActivityCompleted,
            activity_key: Some("activity1".to_string()),
            payload: json!({"outputs": {"value": 42}}),
            timestamp: Utc::now(),
        },
        WorkflowEvent {
            id: Uuid::now_v7(),
            workflow_id,
            event_type: WorkflowEventType::ActivityScheduled,
            activity_key: Some("activity2".to_string()),
            payload: json!({}),
            timestamp: Utc::now(),
        },
        WorkflowEvent {
            id: Uuid::now_v7(),
            workflow_id,
            event_type: WorkflowEventType::ActivityCompleted,
            activity_key: Some("activity2".to_string()),
            payload: json!({"outputs": {"value": 100}}),
            timestamp: Utc::now(),
        },
    ];

    for event in &events {
        apply_event_to_state(&mut state, event).expect("Failed to apply event");
    }

    // Verify final state
    let activity1 = state.activities.get("activity1").unwrap();
    assert_eq!(activity1.status, WorkflowActivityStatus::Completed);
    assert_eq!(activity1.outputs, Some(json!({"value": 42})));

    let activity2 = state.activities.get("activity2").unwrap();
    assert_eq!(activity2.status, WorkflowActivityStatus::Completed);
    assert_eq!(activity2.outputs, Some(json!({"value": 100})));
}
