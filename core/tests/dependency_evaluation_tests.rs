use serde_json::json;
use streamflow_core::events::{
    ActivityDefinition, DependencyEdge, WorkflowDefinition, WorkflowStatus,
};
use streamflow_core::orchestrator::{
    ActivityState, WorkflowActivityStatus, WorkflowState, build_condition_context,
    evaluate_condition, find_ready_activities, is_workflow_complete, is_workflow_failed,
};
use uuid::Uuid;

fn create_test_state_with_activities(
    activities: Vec<(&str, WorkflowActivityStatus, Option<serde_json::Value>)>,
) -> WorkflowState {
    let workflow_id = Uuid::now_v7();
    let activities_map = activities
        .into_iter()
        .map(|(key, status, outputs)| {
            (
                key.to_string(),
                ActivityState {
                    key: key.to_string(),
                    status,
                    outputs,
                    error: None,
                    started_at: None,
                    completed_at: None,
                    retry_count: 0,
                },
            )
        })
        .collect();

    WorkflowState {
        workflow_id,
        definition_name: "test_workflow".to_string(),
        status: WorkflowStatus::Running,
        activities: activities_map,
        state_data: json!({}),
    }
}

#[test]
fn test_find_ready_root_activities() {
    let definition = WorkflowDefinition {
        id: Uuid::now_v7(),
        name: "test_workflow".to_string(),
        version: "1.0".to_string(),
        activities: vec![
            ActivityDefinition {
                key: "root1".to_string(),
                worker: "test".to_string(),
                activity_name: "activity".to_string(),
                parameters: json!({}),
                settings: None,
                depends_on: None,
                dependency_of: None,
            },
            ActivityDefinition {
                key: "root2".to_string(),
                worker: "test".to_string(),
                activity_name: "activity".to_string(),
                parameters: json!({}),
                settings: None,
                depends_on: None,
                dependency_of: None,
            },
        ],
    };

    let state = create_test_state_with_activities(vec![
        ("root1", WorkflowActivityStatus::NotScheduled, None),
        ("root2", WorkflowActivityStatus::NotScheduled, None),
    ]);

    let ready =
        find_ready_activities(&definition, &state).expect("Failed to find ready activities");

    assert_eq!(ready.len(), 2);
    assert!(ready.iter().any(|a| a.key == "root1"));
    assert!(ready.iter().any(|a| a.key == "root2"));
}

#[test]
fn test_find_ready_sequential_workflow() {
    let definition = WorkflowDefinition {
        id: Uuid::now_v7(),
        name: "test_workflow".to_string(),
        version: "1.0".to_string(),
        activities: vec![
            ActivityDefinition {
                key: "activity1".to_string(),
                worker: "test".to_string(),
                activity_name: "activity".to_string(),
                parameters: json!({}),
                settings: None,
                depends_on: None,
                dependency_of: None,
            },
            ActivityDefinition {
                key: "activity2".to_string(),
                worker: "test".to_string(),
                activity_name: "activity".to_string(),
                parameters: json!({}),
                settings: None,
                depends_on: Some(vec![DependencyEdge {
                    activity_key: "activity1".to_string(),
                    conditions: None,
                }]),
                dependency_of: None,
            },
            ActivityDefinition {
                key: "activity3".to_string(),
                worker: "test".to_string(),
                activity_name: "activity".to_string(),
                parameters: json!({}),
                settings: None,
                depends_on: Some(vec![DependencyEdge {
                    activity_key: "activity2".to_string(),
                    conditions: None,
                }]),
                dependency_of: None,
            },
        ],
    };

    // Initial state - only first activity ready
    let state = create_test_state_with_activities(vec![
        ("activity1", WorkflowActivityStatus::NotScheduled, None),
        ("activity2", WorkflowActivityStatus::NotScheduled, None),
        ("activity3", WorkflowActivityStatus::NotScheduled, None),
    ]);

    let ready =
        find_ready_activities(&definition, &state).expect("Failed to find ready activities");
    assert_eq!(ready.len(), 1);
    assert_eq!(ready[0].key, "activity1");

    // After activity1 completes - activity2 becomes ready
    let state = create_test_state_with_activities(vec![
        (
            "activity1",
            WorkflowActivityStatus::Completed,
            Some(json!({"result": "success"})),
        ),
        ("activity2", WorkflowActivityStatus::NotScheduled, None),
        ("activity3", WorkflowActivityStatus::NotScheduled, None),
    ]);

    let ready =
        find_ready_activities(&definition, &state).expect("Failed to find ready activities");
    assert_eq!(ready.len(), 1);
    assert_eq!(ready[0].key, "activity2");

    // After activity2 completes - activity3 becomes ready
    let state = create_test_state_with_activities(vec![
        (
            "activity1",
            WorkflowActivityStatus::Completed,
            Some(json!({"result": "success"})),
        ),
        (
            "activity2",
            WorkflowActivityStatus::Completed,
            Some(json!({"result": "success"})),
        ),
        ("activity3", WorkflowActivityStatus::NotScheduled, None),
    ]);

    let ready =
        find_ready_activities(&definition, &state).expect("Failed to find ready activities");
    assert_eq!(ready.len(), 1);
    assert_eq!(ready[0].key, "activity3");
}

#[test]
fn test_find_ready_parallel_fanout() {
    let definition = WorkflowDefinition {
        id: Uuid::now_v7(),
        name: "test_workflow".to_string(),
        version: "1.0".to_string(),
        activities: vec![
            ActivityDefinition {
                key: "root".to_string(),
                worker: "test".to_string(),
                activity_name: "activity".to_string(),
                parameters: json!({}),
                settings: None,
                depends_on: None,
                dependency_of: Some(vec![
                    DependencyEdge {
                        activity_key: "parallel1".to_string(),
                        conditions: None,
                    },
                    DependencyEdge {
                        activity_key: "parallel2".to_string(),
                        conditions: None,
                    },
                    DependencyEdge {
                        activity_key: "parallel3".to_string(),
                        conditions: None,
                    },
                ]),
            },
            ActivityDefinition {
                key: "parallel1".to_string(),
                worker: "test".to_string(),
                activity_name: "activity".to_string(),
                parameters: json!({}),
                settings: None,
                depends_on: None,
                dependency_of: None,
            },
            ActivityDefinition {
                key: "parallel2".to_string(),
                worker: "test".to_string(),
                activity_name: "activity".to_string(),
                parameters: json!({}),
                settings: None,
                depends_on: None,
                dependency_of: None,
            },
            ActivityDefinition {
                key: "parallel3".to_string(),
                worker: "test".to_string(),
                activity_name: "activity".to_string(),
                parameters: json!({}),
                settings: None,
                depends_on: None,
                dependency_of: None,
            },
        ],
    };

    // After root completes - all parallel activities become ready
    let state = create_test_state_with_activities(vec![
        (
            "root",
            WorkflowActivityStatus::Completed,
            Some(json!({"result": "success"})),
        ),
        ("parallel1", WorkflowActivityStatus::NotScheduled, None),
        ("parallel2", WorkflowActivityStatus::NotScheduled, None),
        ("parallel3", WorkflowActivityStatus::NotScheduled, None),
    ]);

    let ready =
        find_ready_activities(&definition, &state).expect("Failed to find ready activities");
    assert_eq!(ready.len(), 3);
    assert!(ready.iter().any(|a| a.key == "parallel1"));
    assert!(ready.iter().any(|a| a.key == "parallel2"));
    assert!(ready.iter().any(|a| a.key == "parallel3"));
}

#[test]
fn test_find_ready_parallel_fanin() {
    let definition = WorkflowDefinition {
        id: Uuid::now_v7(),
        name: "test_workflow".to_string(),
        version: "1.0".to_string(),
        activities: vec![
            ActivityDefinition {
                key: "parallel1".to_string(),
                worker: "test".to_string(),
                activity_name: "activity".to_string(),
                parameters: json!({}),
                settings: None,
                depends_on: None,
                dependency_of: None,
            },
            ActivityDefinition {
                key: "parallel2".to_string(),
                worker: "test".to_string(),
                activity_name: "activity".to_string(),
                parameters: json!({}),
                settings: None,
                depends_on: None,
                dependency_of: None,
            },
            ActivityDefinition {
                key: "join".to_string(),
                worker: "test".to_string(),
                activity_name: "activity".to_string(),
                parameters: json!({}),
                settings: None,
                depends_on: Some(vec![
                    DependencyEdge {
                        activity_key: "parallel1".to_string(),
                        conditions: None,
                    },
                    DependencyEdge {
                        activity_key: "parallel2".to_string(),
                        conditions: None,
                    },
                ]),
                dependency_of: None,
            },
        ],
    };

    // Only one parallel activity completed - join not ready
    let state = create_test_state_with_activities(vec![
        (
            "parallel1",
            WorkflowActivityStatus::Completed,
            Some(json!({"result": "success"})),
        ),
        ("parallel2", WorkflowActivityStatus::NotScheduled, None),
        ("join", WorkflowActivityStatus::NotScheduled, None),
    ]);

    let ready =
        find_ready_activities(&definition, &state).expect("Failed to find ready activities");
    assert_eq!(ready.len(), 1);
    assert_eq!(ready[0].key, "parallel2");

    // Both parallel activities completed - join becomes ready
    let state = create_test_state_with_activities(vec![
        (
            "parallel1",
            WorkflowActivityStatus::Completed,
            Some(json!({"result": "success"})),
        ),
        (
            "parallel2",
            WorkflowActivityStatus::Completed,
            Some(json!({"result": "success"})),
        ),
        ("join", WorkflowActivityStatus::NotScheduled, None),
    ]);

    let ready =
        find_ready_activities(&definition, &state).expect("Failed to find ready activities");
    assert_eq!(ready.len(), 1);
    assert_eq!(ready[0].key, "join");
}

#[test]
fn test_evaluate_condition_true() {
    let state = create_test_state_with_activities(vec![(
        "activity1",
        WorkflowActivityStatus::Completed,
        Some(json!({"valid": true})),
    )]);

    let context = build_condition_context(&state);
    let result = evaluate_condition("{{activity1.valid == true}}", &context)
        .expect("Failed to evaluate condition");
    assert!(result);
}

#[test]
fn test_evaluate_condition_false() {
    let state = create_test_state_with_activities(vec![(
        "activity1",
        WorkflowActivityStatus::Completed,
        Some(json!({"valid": false})),
    )]);

    let context = build_condition_context(&state);
    let result = evaluate_condition("{{activity1.valid == true}}", &context)
        .expect("Failed to evaluate condition");
    assert!(!result);
}

#[test]
fn test_evaluate_condition_not_equal() {
    let state = create_test_state_with_activities(vec![(
        "activity1",
        WorkflowActivityStatus::Completed,
        Some(json!({"status": "success"})),
    )]);

    let context = build_condition_context(&state);
    let result = evaluate_condition("{{activity1.status != 'failed'}}", &context)
        .expect("Failed to evaluate condition");
    assert!(result);
}

#[test]
fn test_evaluate_condition_string_comparison() {
    let state = create_test_state_with_activities(vec![(
        "activity1",
        WorkflowActivityStatus::Completed,
        Some(json!({"result": "approved"})),
    )]);

    let context = build_condition_context(&state);
    let result = evaluate_condition("{{activity1.result == 'approved'}}", &context)
        .expect("Failed to evaluate condition");
    assert!(result);
}

#[test]
fn test_evaluate_condition_nested_boolean() {
    let state = create_test_state_with_activities(vec![(
        "check_health",
        WorkflowActivityStatus::Completed,
        Some(json!({"response": {"status": 200, "success": true}})),
    )]);

    let context = build_condition_context(&state);

    // Test positive condition
    let result = evaluate_condition("{{check_health.response.success == true}}", &context)
        .expect("Failed to evaluate condition");
    assert!(result, "Expected condition to be true");

    // Test negative condition
    let result = evaluate_condition("{{check_health.response.success != true}}", &context)
        .expect("Failed to evaluate condition");
    assert!(!result, "Expected condition to be false");
}

#[test]
fn test_conditional_branching() {
    let definition = WorkflowDefinition {
        id: Uuid::now_v7(),
        name: "test_workflow".to_string(),
        version: "1.0".to_string(),
        activities: vec![
            ActivityDefinition {
                key: "validate".to_string(),
                worker: "test".to_string(),
                activity_name: "activity".to_string(),
                parameters: json!({}),
                settings: None,
                depends_on: None,
                dependency_of: None,
            },
            ActivityDefinition {
                key: "approve".to_string(),
                worker: "test".to_string(),
                activity_name: "activity".to_string(),
                parameters: json!({}),
                settings: None,
                depends_on: Some(vec![DependencyEdge {
                    activity_key: "validate".to_string(),
                    conditions: Some(vec!["{{validate.valid == true}}".to_string()]),
                }]),
                dependency_of: None,
            },
            ActivityDefinition {
                key: "reject".to_string(),
                worker: "test".to_string(),
                activity_name: "activity".to_string(),
                parameters: json!({}),
                settings: None,
                depends_on: Some(vec![DependencyEdge {
                    activity_key: "validate".to_string(),
                    conditions: Some(vec!["{{validate.valid == false}}".to_string()]),
                }]),
                dependency_of: None,
            },
        ],
    };

    // Valid path - should schedule approve
    let state = create_test_state_with_activities(vec![
        (
            "validate",
            WorkflowActivityStatus::Completed,
            Some(json!({"valid": true})),
        ),
        ("approve", WorkflowActivityStatus::NotScheduled, None),
        ("reject", WorkflowActivityStatus::NotScheduled, None),
    ]);

    let ready =
        find_ready_activities(&definition, &state).expect("Failed to find ready activities");
    assert_eq!(ready.len(), 1);
    assert_eq!(ready[0].key, "approve");

    // Invalid path - should schedule reject
    let state = create_test_state_with_activities(vec![
        (
            "validate",
            WorkflowActivityStatus::Completed,
            Some(json!({"valid": false})),
        ),
        ("approve", WorkflowActivityStatus::NotScheduled, None),
        ("reject", WorkflowActivityStatus::NotScheduled, None),
    ]);

    let ready =
        find_ready_activities(&definition, &state).expect("Failed to find ready activities");
    assert_eq!(ready.len(), 1);
    assert_eq!(ready[0].key, "reject");
}

#[test]
fn test_is_workflow_complete() {
    let state = create_test_state_with_activities(vec![
        (
            "activity1",
            WorkflowActivityStatus::Completed,
            Some(json!({"result": "success"})),
        ),
        (
            "activity2",
            WorkflowActivityStatus::Completed,
            Some(json!({"result": "success"})),
        ),
    ]);

    assert!(is_workflow_complete(&state));

    // Not complete - one activity still pending
    let state = create_test_state_with_activities(vec![
        (
            "activity1",
            WorkflowActivityStatus::Completed,
            Some(json!({"result": "success"})),
        ),
        ("activity2", WorkflowActivityStatus::Pending, None),
    ]);

    assert!(!is_workflow_complete(&state));
}

#[test]
fn test_is_workflow_failed() {
    let state = create_test_state_with_activities(vec![
        (
            "activity1",
            WorkflowActivityStatus::Completed,
            Some(json!({"result": "success"})),
        ),
        ("activity2", WorkflowActivityStatus::Failed, None),
    ]);

    assert!(is_workflow_failed(&state));

    // Not failed - all successful
    let state = create_test_state_with_activities(vec![
        (
            "activity1",
            WorkflowActivityStatus::Completed,
            Some(json!({"result": "success"})),
        ),
        (
            "activity2",
            WorkflowActivityStatus::Completed,
            Some(json!({"result": "success"})),
        ),
    ]);

    assert!(!is_workflow_failed(&state));
}

#[test]
fn test_skip_already_scheduled_activities() {
    let definition = WorkflowDefinition {
        id: Uuid::now_v7(),
        name: "test_workflow".to_string(),
        version: "1.0".to_string(),
        activities: vec![ActivityDefinition {
            key: "activity1".to_string(),
            worker: "test".to_string(),
            activity_name: "activity".to_string(),
            parameters: json!({}),
            settings: None,
            depends_on: None,
            dependency_of: None,
        }],
    };

    // Activity already pending - should not be ready
    let state = create_test_state_with_activities(vec![(
        "activity1",
        WorkflowActivityStatus::Pending,
        None,
    )]);

    let ready =
        find_ready_activities(&definition, &state).expect("Failed to find ready activities");
    assert_eq!(ready.len(), 0);

    // Activity already completed - should not be ready
    let state = create_test_state_with_activities(vec![(
        "activity1",
        WorkflowActivityStatus::Completed,
        Some(json!({"result": "success"})),
    )]);

    let ready =
        find_ready_activities(&definition, &state).expect("Failed to find ready activities");
    assert_eq!(ready.len(), 0);
}

#[test]
fn test_failed_activity_without_conditions_blocks_following() {
    // Test that when a preceding activity fails with NO conditions,
    // the following activity should NOT be ready (default = success path only)
    let definition = WorkflowDefinition {
        id: Uuid::now_v7(),
        name: "test_workflow".to_string(),
        version: "1.0".to_string(),
        activities: vec![
            ActivityDefinition {
                key: "step1".to_string(),
                worker: "test".to_string(),
                activity_name: "activity".to_string(),
                parameters: json!({}),
                settings: None,
                depends_on: None,
                dependency_of: None,
            },
            ActivityDefinition {
                key: "step2".to_string(),
                worker: "test".to_string(),
                activity_name: "activity".to_string(),
                parameters: json!({}),
                settings: None,
                depends_on: Some(vec![DependencyEdge {
                    activity_key: "step1".to_string(),
                    conditions: None, // No conditions = success path only
                }]),
                dependency_of: None,
            },
        ],
    };

    // step1 failed, step2 not scheduled yet
    let state = create_test_state_with_activities(vec![
        ("step1", WorkflowActivityStatus::Failed, None),
        ("step2", WorkflowActivityStatus::NotScheduled, None),
    ]);

    // step2 should NOT be ready because step1 failed and there are no explicit conditions
    let ready =
        find_ready_activities(&definition, &state).expect("Failed to find ready activities");
    assert_eq!(
        ready.len(),
        0,
        "Following activity should not be ready when preceding failed without conditions"
    );
}

#[test]
fn test_failed_activity_with_explicit_condition_allows_following() {
    // Test that when a preceding activity fails WITH explicit conditions,
    // the following activity CAN be ready if conditions are satisfied
    let definition = WorkflowDefinition {
        id: Uuid::now_v7(),
        name: "test_workflow".to_string(),
        version: "1.0".to_string(),
        activities: vec![
            ActivityDefinition {
                key: "process".to_string(),
                worker: "test".to_string(),
                activity_name: "activity".to_string(),
                parameters: json!({}),
                settings: None,
                depends_on: None,
                dependency_of: None,
            },
            ActivityDefinition {
                key: "handle_success".to_string(),
                worker: "test".to_string(),
                activity_name: "activity".to_string(),
                parameters: json!({}),
                settings: None,
                depends_on: Some(vec![DependencyEdge {
                    activity_key: "process".to_string(),
                    conditions: Some(vec!["{{process.success == true}}".to_string()]),
                }]),
                dependency_of: None,
            },
            ActivityDefinition {
                key: "handle_failure".to_string(),
                worker: "test".to_string(),
                activity_name: "activity".to_string(),
                parameters: json!({}),
                settings: None,
                depends_on: Some(vec![DependencyEdge {
                    activity_key: "process".to_string(),
                    // Explicit condition checking for failure
                    conditions: Some(vec!["{{process.error != null}}".to_string()]),
                }]),
                dependency_of: None,
            },
        ],
    };

    // process failed with error
    let state = create_test_state_with_activities(vec![
        (
            "process",
            WorkflowActivityStatus::Failed,
            Some(json!({"error": "Something went wrong", "success": false})),
        ),
        ("handle_success", WorkflowActivityStatus::NotScheduled, None),
        ("handle_failure", WorkflowActivityStatus::NotScheduled, None),
    ]);

    // Only handle_failure should be ready (condition satisfied)
    let ready =
        find_ready_activities(&definition, &state).expect("Failed to find ready activities");
    assert_eq!(ready.len(), 1);
    assert_eq!(
        ready[0].key, "handle_failure",
        "Error handler should be ready when process failed"
    );
}
