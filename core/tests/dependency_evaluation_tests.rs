use kruxiaflow_core::events::WorkflowStatus;
use kruxiaflow_core::orchestrator::{
    ActivityState, WorkflowActivityStatus, WorkflowState, build_condition_context,
    evaluate_condition, find_ready_activities, find_skipped_activities, is_workflow_complete,
    is_workflow_failed,
};
use kruxiaflow_core::workflow::{
    ActivityDefinition, ActivityOutput, ActivityRelationship, OutputType, WorkflowDefinition,
};
use rust_decimal::Decimal;
use serde_json::json;
use std::collections::HashMap;
use uuid::Uuid;

fn create_test_state_with_activities(
    activities: Vec<(
        &str,
        WorkflowActivityStatus,
        Option<Vec<kruxiaflow_core::workflow::ActivityOutput>>,
    )>,
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
                    attempt: 0,
                    last_error: None,
                    accumulated_cost_usd: Decimal::ZERO,
                    iteration: 0,
                    iteration_outputs: None,
                    signal_data: None,
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
        input: json!({}),
    }
}

#[test]
fn test_find_ready_root_activities() {
    let definition = WorkflowDefinition {
        name: "test_workflow".to_string(),
        activities: vec![
            ActivityDefinition {
                key: "root1".to_string(),
                worker: "test".to_string(),
                activity_name: Some("activity".to_string()),
                parameters: Some(HashMap::new()),
                settings: None,
                depends_on: None,
                dependency_of: None,
                output_definitions: None,
                iteration_scoped: false,
                iteration_limit: None,
                is_loop_activity: false,
                streaming: Default::default(),
            },
            ActivityDefinition {
                key: "root2".to_string(),
                worker: "test".to_string(),
                activity_name: Some("activity".to_string()),
                parameters: Some(HashMap::new()),
                settings: None,
                depends_on: None,
                dependency_of: None,
                output_definitions: None,
                iteration_scoped: false,
                iteration_limit: None,
                is_loop_activity: false,
                streaming: Default::default(),
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
        name: "test_workflow".to_string(),
        activities: vec![
            ActivityDefinition {
                key: "activity1".to_string(),
                worker: "test".to_string(),
                activity_name: Some("activity".to_string()),
                parameters: Some(HashMap::new()),
                settings: None,
                depends_on: None,
                dependency_of: None,
                output_definitions: None,
                iteration_scoped: false,
                iteration_limit: None,
                is_loop_activity: false,
                streaming: Default::default(),
            },
            ActivityDefinition {
                key: "activity2".to_string(),
                worker: "test".to_string(),
                activity_name: Some("activity".to_string()),
                parameters: Some(HashMap::new()),
                settings: None,
                depends_on: Some(vec![ActivityRelationship {
                    activity_key: "activity1".to_string(),
                    conditions: None,
                    is_back_edge: false,
                }]),
                dependency_of: None,
                output_definitions: None,
                iteration_scoped: false,
                iteration_limit: None,
                is_loop_activity: false,
                streaming: Default::default(),
            },
            ActivityDefinition {
                key: "activity3".to_string(),
                worker: "test".to_string(),
                activity_name: Some("activity".to_string()),
                parameters: Some(HashMap::new()),
                settings: None,
                depends_on: Some(vec![ActivityRelationship {
                    activity_key: "activity2".to_string(),
                    conditions: None,
                    is_back_edge: false,
                }]),
                dependency_of: None,
                output_definitions: None,
                iteration_scoped: false,
                iteration_limit: None,
                is_loop_activity: false,
                streaming: Default::default(),
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
            Some(vec![ActivityOutput {
                name: "result".to_string(),
                output_type: OutputType::Value,
                value: json!("success"),
            }]),
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
            Some(vec![ActivityOutput {
                name: "result".to_string(),
                output_type: OutputType::Value,
                value: json!("success"),
            }]),
        ),
        (
            "activity2",
            WorkflowActivityStatus::Completed,
            Some(vec![ActivityOutput {
                name: "result".to_string(),
                output_type: OutputType::Value,
                value: json!("success"),
            }]),
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
        name: "test_workflow".to_string(),
        activities: vec![
            ActivityDefinition {
                key: "root".to_string(),
                worker: "test".to_string(),
                activity_name: Some("activity".to_string()),
                parameters: Some(HashMap::new()),
                settings: None,
                depends_on: None,
                dependency_of: Some(vec![
                    ActivityRelationship {
                        activity_key: "parallel1".to_string(),
                        conditions: None,
                        is_back_edge: false,
                    },
                    ActivityRelationship {
                        activity_key: "parallel2".to_string(),
                        conditions: None,
                        is_back_edge: false,
                    },
                    ActivityRelationship {
                        activity_key: "parallel3".to_string(),
                        conditions: None,
                        is_back_edge: false,
                    },
                ]),
                output_definitions: None,
                iteration_scoped: false,
                iteration_limit: None,
                is_loop_activity: false,
                streaming: Default::default(),
            },
            ActivityDefinition {
                key: "parallel1".to_string(),
                worker: "test".to_string(),
                activity_name: Some("activity".to_string()),
                parameters: Some(HashMap::new()),
                settings: None,
                depends_on: None,
                dependency_of: None,
                output_definitions: None,
                iteration_scoped: false,
                iteration_limit: None,
                is_loop_activity: false,
                streaming: Default::default(),
            },
            ActivityDefinition {
                key: "parallel2".to_string(),
                worker: "test".to_string(),
                activity_name: Some("activity".to_string()),
                parameters: Some(HashMap::new()),
                settings: None,
                depends_on: None,
                dependency_of: None,
                output_definitions: None,
                iteration_scoped: false,
                iteration_limit: None,
                is_loop_activity: false,
                streaming: Default::default(),
            },
            ActivityDefinition {
                key: "parallel3".to_string(),
                worker: "test".to_string(),
                activity_name: Some("activity".to_string()),
                parameters: Some(HashMap::new()),
                settings: None,
                depends_on: None,
                dependency_of: None,
                output_definitions: None,
                iteration_scoped: false,
                iteration_limit: None,
                is_loop_activity: false,
                streaming: Default::default(),
            },
        ],
    };

    // After root completes - all parallel activities become ready
    let state = create_test_state_with_activities(vec![
        (
            "root",
            WorkflowActivityStatus::Completed,
            Some(vec![ActivityOutput {
                name: "result".to_string(),
                output_type: OutputType::Value,
                value: json!("success"),
            }]),
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
        name: "test_workflow".to_string(),
        activities: vec![
            ActivityDefinition {
                key: "parallel1".to_string(),
                worker: "test".to_string(),
                activity_name: Some("activity".to_string()),
                parameters: Some(HashMap::new()),
                settings: None,
                depends_on: None,
                dependency_of: None,
                output_definitions: None,
                iteration_scoped: false,
                iteration_limit: None,
                is_loop_activity: false,
                streaming: Default::default(),
            },
            ActivityDefinition {
                key: "parallel2".to_string(),
                worker: "test".to_string(),
                activity_name: Some("activity".to_string()),
                parameters: Some(HashMap::new()),
                settings: None,
                depends_on: None,
                dependency_of: None,
                output_definitions: None,
                iteration_scoped: false,
                iteration_limit: None,
                is_loop_activity: false,
                streaming: Default::default(),
            },
            ActivityDefinition {
                key: "join".to_string(),
                worker: "test".to_string(),
                activity_name: Some("activity".to_string()),
                parameters: Some(HashMap::new()),
                settings: None,
                depends_on: Some(vec![
                    ActivityRelationship {
                        activity_key: "parallel1".to_string(),
                        conditions: None,
                        is_back_edge: false,
                    },
                    ActivityRelationship {
                        activity_key: "parallel2".to_string(),
                        conditions: None,
                        is_back_edge: false,
                    },
                ]),
                dependency_of: None,
                output_definitions: None,
                iteration_scoped: false,
                iteration_limit: None,
                is_loop_activity: false,
                streaming: Default::default(),
            },
        ],
    };

    // Only one parallel activity completed - join not ready
    let state = create_test_state_with_activities(vec![
        (
            "parallel1",
            WorkflowActivityStatus::Completed,
            Some(vec![ActivityOutput {
                name: "result".to_string(),
                output_type: OutputType::Value,
                value: json!("success"),
            }]),
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
            Some(vec![ActivityOutput {
                name: "result".to_string(),
                output_type: OutputType::Value,
                value: json!("success"),
            }]),
        ),
        (
            "parallel2",
            WorkflowActivityStatus::Completed,
            Some(vec![ActivityOutput {
                name: "result".to_string(),
                output_type: OutputType::Value,
                value: json!("success"),
            }]),
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
        Some(vec![ActivityOutput {
            name: "valid".to_string(),
            output_type: OutputType::Value,
            value: json!(true),
        }]),
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
        Some(vec![ActivityOutput {
            name: "valid".to_string(),
            output_type: OutputType::Value,
            value: json!(false),
        }]),
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
        Some(vec![ActivityOutput {
            name: "status".to_string(),
            output_type: OutputType::Value,
            value: json!("success"),
        }]),
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
        Some(vec![ActivityOutput {
            name: "result".to_string(),
            output_type: OutputType::Value,
            value: json!("approved"),
        }]),
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
        Some(vec![ActivityOutput {
            name: "response".to_string(),
            output_type: OutputType::Value,
            value: json!({"status": 200, "success": true}),
        }]),
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
        name: "test_workflow".to_string(),
        activities: vec![
            ActivityDefinition {
                key: "validate".to_string(),
                worker: "test".to_string(),
                activity_name: Some("activity".to_string()),
                parameters: Some(HashMap::new()),
                settings: None,
                depends_on: None,
                dependency_of: None,
                output_definitions: None,
                iteration_scoped: false,
                iteration_limit: None,
                is_loop_activity: false,
                streaming: Default::default(),
            },
            ActivityDefinition {
                key: "approve".to_string(),
                worker: "test".to_string(),
                activity_name: Some("activity".to_string()),
                parameters: Some(HashMap::new()),
                settings: None,
                depends_on: Some(vec![ActivityRelationship {
                    activity_key: "validate".to_string(),
                    conditions: Some(vec!["{{validate.valid == true}}".to_string()]),
                    is_back_edge: false,
                }]),
                dependency_of: None,
                output_definitions: None,
                iteration_scoped: false,
                iteration_limit: None,
                is_loop_activity: false,
                streaming: Default::default(),
            },
            ActivityDefinition {
                key: "reject".to_string(),
                worker: "test".to_string(),
                activity_name: Some("activity".to_string()),
                parameters: Some(HashMap::new()),
                settings: None,
                depends_on: Some(vec![ActivityRelationship {
                    activity_key: "validate".to_string(),
                    conditions: Some(vec!["{{validate.valid == false}}".to_string()]),
                    is_back_edge: false,
                }]),
                dependency_of: None,
                output_definitions: None,
                iteration_scoped: false,
                iteration_limit: None,
                is_loop_activity: false,
                streaming: Default::default(),
            },
        ],
    };

    // Valid path - should schedule approve
    let state = create_test_state_with_activities(vec![
        (
            "validate",
            WorkflowActivityStatus::Completed,
            Some(vec![ActivityOutput {
                name: "valid".to_string(),
                output_type: OutputType::Value,
                value: json!(true),
            }]),
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
            Some(vec![ActivityOutput {
                name: "valid".to_string(),
                output_type: OutputType::Value,
                value: json!(false),
            }]),
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
            Some(vec![ActivityOutput {
                name: "result".to_string(),
                output_type: OutputType::Value,
                value: json!("success"),
            }]),
        ),
        (
            "activity2",
            WorkflowActivityStatus::Completed,
            Some(vec![ActivityOutput {
                name: "result".to_string(),
                output_type: OutputType::Value,
                value: json!("success"),
            }]),
        ),
    ]);

    assert!(is_workflow_complete(&state));

    // Not complete - one activity still pending
    let state = create_test_state_with_activities(vec![
        (
            "activity1",
            WorkflowActivityStatus::Completed,
            Some(vec![ActivityOutput {
                name: "result".to_string(),
                output_type: OutputType::Value,
                value: json!("success"),
            }]),
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
            Some(vec![ActivityOutput {
                name: "result".to_string(),
                output_type: OutputType::Value,
                value: json!("success"),
            }]),
        ),
        ("activity2", WorkflowActivityStatus::Failed, None),
    ]);

    assert!(is_workflow_failed(&state));

    // Not failed - all successful
    let state = create_test_state_with_activities(vec![
        (
            "activity1",
            WorkflowActivityStatus::Completed,
            Some(vec![ActivityOutput {
                name: "result".to_string(),
                output_type: OutputType::Value,
                value: json!("success"),
            }]),
        ),
        (
            "activity2",
            WorkflowActivityStatus::Completed,
            Some(vec![ActivityOutput {
                name: "result".to_string(),
                output_type: OutputType::Value,
                value: json!("success"),
            }]),
        ),
    ]);

    assert!(!is_workflow_failed(&state));
}

#[test]
fn test_skip_already_scheduled_activities() {
    let definition = WorkflowDefinition {
        name: "test_workflow".to_string(),
        activities: vec![ActivityDefinition {
            key: "activity1".to_string(),
            worker: "test".to_string(),
            activity_name: Some("activity".to_string()),
            parameters: Some(HashMap::new()),
            settings: None,
            depends_on: None,
            dependency_of: None,
            output_definitions: None,
            iteration_scoped: false,
            iteration_limit: None,
            is_loop_activity: false,
            streaming: Default::default(),
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
        Some(vec![ActivityOutput {
            name: "result".to_string(),
            output_type: OutputType::Value,
            value: json!("success"),
        }]),
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
        name: "test_workflow".to_string(),
        activities: vec![
            ActivityDefinition {
                key: "step1".to_string(),
                worker: "test".to_string(),
                activity_name: Some("activity".to_string()),
                parameters: Some(HashMap::new()),
                settings: None,
                depends_on: None,
                dependency_of: None,
                output_definitions: None,
                iteration_scoped: false,
                iteration_limit: None,
                is_loop_activity: false,
                streaming: Default::default(),
            },
            ActivityDefinition {
                key: "step2".to_string(),
                worker: "test".to_string(),
                activity_name: Some("activity".to_string()),
                parameters: Some(HashMap::new()),
                settings: None,
                depends_on: Some(vec![ActivityRelationship {
                    activity_key: "step1".to_string(),
                    conditions: None, // No conditions = success path only
                    is_back_edge: false,
                }]),
                dependency_of: None,
                output_definitions: None,
                iteration_scoped: false,
                iteration_limit: None,
                is_loop_activity: false,
                streaming: Default::default(),
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
        name: "test_workflow".to_string(),
        activities: vec![
            ActivityDefinition {
                key: "process".to_string(),
                worker: "test".to_string(),
                activity_name: Some("activity".to_string()),
                parameters: Some(HashMap::new()),
                settings: None,
                depends_on: None,
                dependency_of: None,
                output_definitions: None,
                iteration_scoped: false,
                iteration_limit: None,
                is_loop_activity: false,
                streaming: Default::default(),
            },
            ActivityDefinition {
                key: "handle_success".to_string(),
                worker: "test".to_string(),
                activity_name: Some("activity".to_string()),
                parameters: Some(HashMap::new()),
                settings: None,
                depends_on: Some(vec![ActivityRelationship {
                    activity_key: "process".to_string(),
                    conditions: Some(vec!["{{process.success == true}}".to_string()]),
                    is_back_edge: false,
                }]),
                dependency_of: None,
                output_definitions: None,
                iteration_scoped: false,
                iteration_limit: None,
                is_loop_activity: false,
                streaming: Default::default(),
            },
            ActivityDefinition {
                key: "handle_failure".to_string(),
                worker: "test".to_string(),
                activity_name: Some("activity".to_string()),
                parameters: Some(HashMap::new()),
                settings: None,
                depends_on: Some(vec![ActivityRelationship {
                    activity_key: "process".to_string(),
                    // Explicit condition checking for failure
                    conditions: Some(vec!["{{process.error != null}}".to_string()]),
                    is_back_edge: false,
                }]),
                dependency_of: None,
                output_definitions: None,
                iteration_scoped: false,
                iteration_limit: None,
                is_loop_activity: false,
                streaming: Default::default(),
            },
        ],
    };

    // process failed with error
    let state = create_test_state_with_activities(vec![
        (
            "process",
            WorkflowActivityStatus::Failed,
            Some(vec![
                ActivityOutput {
                    name: "error".to_string(),
                    output_type: OutputType::Value,
                    value: json!("Something went wrong"),
                },
                ActivityOutput {
                    name: "success".to_string(),
                    output_type: OutputType::Value,
                    value: json!(false),
                },
            ]),
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

// ============================================================================
// Additional tests for edge cases and improved coverage
// ============================================================================

#[test]
fn test_evaluate_condition_numeric_comparison() {
    let state = create_test_state_with_activities(vec![(
        "activity1",
        WorkflowActivityStatus::Completed,
        Some(vec![ActivityOutput {
            name: "count".to_string(),
            output_type: OutputType::Value,
            value: json!(42),
        }]),
    )]);

    let context = build_condition_context(&state);

    // Test greater than
    let result = evaluate_condition("{{activity1.count > 40}}", &context)
        .expect("Failed to evaluate condition");
    assert!(result);

    // Test less than
    let result = evaluate_condition("{{activity1.count < 50}}", &context)
        .expect("Failed to evaluate condition");
    assert!(result);

    // Test equality
    let result = evaluate_condition("{{activity1.count == 42}}", &context)
        .expect("Failed to evaluate condition");
    assert!(result);
}

#[test]
fn test_evaluate_condition_null_handling() {
    let state = create_test_state_with_activities(vec![(
        "activity1",
        WorkflowActivityStatus::Completed,
        Some(vec![ActivityOutput {
            name: "maybe_value".to_string(),
            output_type: OutputType::Value,
            value: json!(null),
        }]),
    )]);

    let context = build_condition_context(&state);

    // Null should evaluate to false
    let result = evaluate_condition("{{activity1.maybe_value}}", &context)
        .expect("Failed to evaluate condition");
    assert!(!result, "Null value should be falsy");

    // Null comparison using Jinja2 'none' keyword
    let result = evaluate_condition("{{activity1.maybe_value is none}}", &context)
        .expect("Failed to evaluate condition");
    assert!(result, "Null value should test as 'none'");
}

#[test]
fn test_evaluate_condition_empty_string() {
    let state = create_test_state_with_activities(vec![(
        "activity1",
        WorkflowActivityStatus::Completed,
        Some(vec![ActivityOutput {
            name: "text".to_string(),
            output_type: OutputType::Value,
            value: json!(""),
        }]),
    )]);

    let context = build_condition_context(&state);

    // Empty string should be falsy
    let result =
        evaluate_condition("{{activity1.text}}", &context).expect("Failed to evaluate condition");
    assert!(!result, "Empty string should be falsy");
}

#[test]
fn test_evaluate_condition_non_empty_string() {
    let state = create_test_state_with_activities(vec![(
        "activity1",
        WorkflowActivityStatus::Completed,
        Some(vec![ActivityOutput {
            name: "text".to_string(),
            output_type: OutputType::Value,
            value: json!("hello"),
        }]),
    )]);

    let context = build_condition_context(&state);

    // Non-empty string should be truthy
    let result =
        evaluate_condition("{{activity1.text}}", &context).expect("Failed to evaluate condition");
    assert!(result, "Non-empty string should be truthy");
}

#[test]
fn test_evaluate_condition_zero_is_falsy() {
    let state = create_test_state_with_activities(vec![(
        "activity1",
        WorkflowActivityStatus::Completed,
        Some(vec![ActivityOutput {
            name: "count".to_string(),
            output_type: OutputType::Value,
            value: json!(0),
        }]),
    )]);

    let context = build_condition_context(&state);

    // Zero should be falsy
    let result =
        evaluate_condition("{{activity1.count}}", &context).expect("Failed to evaluate condition");
    assert!(!result, "Zero should be falsy");
}

#[test]
fn test_evaluate_condition_array_is_truthy() {
    let state = create_test_state_with_activities(vec![(
        "activity1",
        WorkflowActivityStatus::Completed,
        Some(vec![ActivityOutput {
            name: "items".to_string(),
            output_type: OutputType::Value,
            value: json!(["a", "b", "c"]),
        }]),
    )]);

    let context = build_condition_context(&state);

    // Array should be truthy
    let result =
        evaluate_condition("{{activity1.items}}", &context).expect("Failed to evaluate condition");
    assert!(result, "Array should be truthy");
}

#[test]
fn test_find_skipped_activities_conditional_branch_not_taken() {
    // Create a workflow with conditional branches where one branch won't be taken
    let definition = WorkflowDefinition {
        name: "test_workflow".to_string(),
        activities: vec![
            ActivityDefinition {
                key: "validate".to_string(),
                worker: "test".to_string(),
                activity_name: Some("activity".to_string()),
                parameters: Some(HashMap::new()),
                settings: None,
                depends_on: None,
                dependency_of: None,
                output_definitions: None,
                iteration_scoped: false,
                iteration_limit: None,
                is_loop_activity: false,
                streaming: Default::default(),
            },
            ActivityDefinition {
                key: "path_a".to_string(),
                worker: "test".to_string(),
                activity_name: Some("activity".to_string()),
                parameters: Some(HashMap::new()),
                settings: None,
                depends_on: Some(vec![ActivityRelationship {
                    activity_key: "validate".to_string(),
                    conditions: Some(vec!["{{validate.choice == 'A'}}".to_string()]),
                    is_back_edge: false,
                }]),
                dependency_of: None,
                output_definitions: None,
                iteration_scoped: false,
                iteration_limit: None,
                is_loop_activity: false,
                streaming: Default::default(),
            },
            ActivityDefinition {
                key: "path_b".to_string(),
                worker: "test".to_string(),
                activity_name: Some("activity".to_string()),
                parameters: Some(HashMap::new()),
                settings: None,
                depends_on: Some(vec![ActivityRelationship {
                    activity_key: "validate".to_string(),
                    conditions: Some(vec!["{{validate.choice == 'B'}}".to_string()]),
                    is_back_edge: false,
                }]),
                dependency_of: None,
                output_definitions: None,
                iteration_scoped: false,
                iteration_limit: None,
                is_loop_activity: false,
                streaming: Default::default(),
            },
        ],
    };

    // Validate completed with choice='A', so path_b should be skipped
    let state = create_test_state_with_activities(vec![
        (
            "validate",
            WorkflowActivityStatus::Completed,
            Some(vec![ActivityOutput {
                name: "choice".to_string(),
                output_type: OutputType::Value,
                value: json!("A"),
            }]),
        ),
        ("path_a", WorkflowActivityStatus::NotScheduled, None),
        ("path_b", WorkflowActivityStatus::NotScheduled, None),
    ]);

    let skipped =
        find_skipped_activities(&definition, &state).expect("Failed to find skipped activities");

    // path_b should be skipped because its condition will never be met
    assert_eq!(skipped.len(), 1);
    assert_eq!(skipped[0].key, "path_b");
}

#[test]
fn test_find_skipped_activities_upstream_failed() {
    // When an upstream activity fails (no conditions), downstream should be skippable
    let definition = WorkflowDefinition {
        name: "test_workflow".to_string(),
        activities: vec![
            ActivityDefinition {
                key: "step1".to_string(),
                worker: "test".to_string(),
                activity_name: Some("activity".to_string()),
                parameters: Some(HashMap::new()),
                settings: None,
                depends_on: None,
                dependency_of: None,
                output_definitions: None,
                iteration_scoped: false,
                iteration_limit: None,
                is_loop_activity: false,
                streaming: Default::default(),
            },
            ActivityDefinition {
                key: "step2".to_string(),
                worker: "test".to_string(),
                activity_name: Some("activity".to_string()),
                parameters: Some(HashMap::new()),
                settings: None,
                depends_on: Some(vec![ActivityRelationship {
                    activity_key: "step1".to_string(),
                    conditions: None, // Unconditional - requires success
                    is_back_edge: false,
                }]),
                dependency_of: None,
                output_definitions: None,
                iteration_scoped: false,
                iteration_limit: None,
                is_loop_activity: false,
                streaming: Default::default(),
            },
        ],
    };

    // step1 failed, step2 cannot run
    let state = create_test_state_with_activities(vec![
        ("step1", WorkflowActivityStatus::Failed, None),
        ("step2", WorkflowActivityStatus::NotScheduled, None),
    ]);

    let skipped =
        find_skipped_activities(&definition, &state).expect("Failed to find skipped activities");

    // step2 should be skipped because step1 failed
    assert_eq!(skipped.len(), 1);
    assert_eq!(skipped[0].key, "step2");
}

#[test]
fn test_find_skipped_activities_already_scheduled_not_skipped() {
    // Activities that are already Pending/Running/Completed should not be marked as skipped
    let definition = WorkflowDefinition {
        name: "test_workflow".to_string(),
        activities: vec![
            ActivityDefinition {
                key: "root".to_string(),
                worker: "test".to_string(),
                activity_name: Some("activity".to_string()),
                parameters: Some(HashMap::new()),
                settings: None,
                depends_on: None,
                dependency_of: None,
                output_definitions: None,
                iteration_scoped: false,
                iteration_limit: None,
                is_loop_activity: false,
                streaming: Default::default(),
            },
            ActivityDefinition {
                key: "child".to_string(),
                worker: "test".to_string(),
                activity_name: Some("activity".to_string()),
                parameters: Some(HashMap::new()),
                settings: None,
                depends_on: Some(vec![ActivityRelationship {
                    activity_key: "root".to_string(),
                    conditions: None,
                    is_back_edge: false,
                }]),
                dependency_of: None,
                output_definitions: None,
                iteration_scoped: false,
                iteration_limit: None,
                is_loop_activity: false,
                streaming: Default::default(),
            },
        ],
    };

    // Child is already Pending (scheduled)
    let state = create_test_state_with_activities(vec![
        (
            "root",
            WorkflowActivityStatus::Completed,
            Some(vec![ActivityOutput {
                name: "result".to_string(),
                output_type: OutputType::Value,
                value: json!("success"),
            }]),
        ),
        ("child", WorkflowActivityStatus::Pending, None),
    ]);

    let skipped =
        find_skipped_activities(&definition, &state).expect("Failed to find skipped activities");

    // Nothing should be skipped - child is already pending
    assert_eq!(skipped.len(), 0);
}

#[test]
fn test_is_workflow_complete_with_skipped() {
    // Workflow should be complete if all activities are in terminal states including Skipped
    let state = create_test_state_with_activities(vec![
        (
            "activity1",
            WorkflowActivityStatus::Completed,
            Some(vec![ActivityOutput {
                name: "result".to_string(),
                output_type: OutputType::Value,
                value: json!("success"),
            }]),
        ),
        ("activity2", WorkflowActivityStatus::Skipped, None),
    ]);

    assert!(is_workflow_complete(&state));
}

#[test]
fn test_is_workflow_complete_with_running_activity() {
    // Workflow should NOT be complete if any activity is still Running
    let mut activities_map = HashMap::new();
    activities_map.insert(
        "activity1".to_string(),
        ActivityState {
            key: "activity1".to_string(),
            status: WorkflowActivityStatus::Completed,
            outputs: Some(vec![ActivityOutput {
                name: "result".to_string(),
                output_type: OutputType::Value,
                value: json!("success"),
            }]),
            error: None,
            started_at: None,
            completed_at: None,
            attempt: 0,
            last_error: None,
            accumulated_cost_usd: Decimal::ZERO,
            iteration: 0,
            iteration_outputs: None,
            signal_data: None,
        },
    );
    activities_map.insert(
        "activity2".to_string(),
        ActivityState {
            key: "activity2".to_string(),
            status: WorkflowActivityStatus::Running,
            outputs: None,
            error: None,
            started_at: None,
            completed_at: None,
            attempt: 0,
            last_error: None,
            accumulated_cost_usd: Decimal::ZERO,
            iteration: 0,
            iteration_outputs: None,
            signal_data: None,
        },
    );

    let state = WorkflowState {
        workflow_id: Uuid::now_v7(),
        definition_name: "test_workflow".to_string(),
        status: WorkflowStatus::Running,
        activities: activities_map,
        state_data: json!({}),
        input: json!({}),
    };

    assert!(!is_workflow_complete(&state));
}

#[test]
fn test_loop_activity_not_ready_when_max_iterations_exceeded() {
    // A loop activity should not be ready when iteration limit is exceeded
    let definition = WorkflowDefinition {
        name: "test_workflow".to_string(),
        activities: vec![ActivityDefinition {
            key: "loop_task".to_string(),
            worker: "test".to_string(),
            activity_name: Some("activity".to_string()),
            parameters: Some(HashMap::new()),
            settings: None,
            depends_on: Some(vec![ActivityRelationship {
                activity_key: "loop_task".to_string(),
                conditions: Some(vec!["{{true}}".to_string()]),
                is_back_edge: true,
            }]),
            dependency_of: None,
            output_definitions: None,
            iteration_scoped: false,
            iteration_limit: Some(3), // Max 3 iterations
            is_loop_activity: true,
            streaming: Default::default(),
        }],
    };

    // Activity has completed 3 iterations - should not loop back
    let mut activities_map = HashMap::new();
    activities_map.insert(
        "loop_task".to_string(),
        ActivityState {
            key: "loop_task".to_string(),
            status: WorkflowActivityStatus::Completed,
            outputs: Some(vec![ActivityOutput {
                name: "result".to_string(),
                output_type: OutputType::Value,
                value: json!("done"),
            }]),
            error: None,
            started_at: None,
            completed_at: None,
            attempt: 0,
            last_error: None,
            accumulated_cost_usd: Decimal::ZERO,
            iteration: 3, // At limit
            iteration_outputs: None,
            signal_data: None,
        },
    );

    let state = WorkflowState {
        workflow_id: Uuid::now_v7(),
        definition_name: "test_workflow".to_string(),
        status: WorkflowStatus::Running,
        activities: activities_map,
        state_data: json!({}),
        input: json!({}),
    };

    let ready =
        find_ready_activities(&definition, &state).expect("Failed to find ready activities");
    assert_eq!(
        ready.len(),
        0,
        "Loop activity should not be ready when iteration limit is reached"
    );
}

#[test]
fn test_back_edge_first_iteration_auto_satisfied() {
    // On first iteration (iteration 0), back-edge dependencies should be auto-satisfied
    let definition = WorkflowDefinition {
        name: "test_workflow".to_string(),
        activities: vec![ActivityDefinition {
            key: "process".to_string(),
            worker: "test".to_string(),
            activity_name: Some("activity".to_string()),
            parameters: Some(HashMap::new()),
            settings: None,
            depends_on: Some(vec![ActivityRelationship {
                activity_key: "process".to_string(),
                conditions: Some(vec!["{{process.done == false}}".to_string()]),
                is_back_edge: true,
            }]),
            dependency_of: None,
            output_definitions: None,
            iteration_scoped: false,
            iteration_limit: Some(5),
            is_loop_activity: true,
            streaming: Default::default(),
        }],
    };

    // First iteration - activity is NotScheduled with iteration 0
    let mut activities_map = HashMap::new();
    activities_map.insert(
        "process".to_string(),
        ActivityState {
            key: "process".to_string(),
            status: WorkflowActivityStatus::NotScheduled,
            outputs: None,
            error: None,
            started_at: None,
            completed_at: None,
            attempt: 0,
            last_error: None,
            accumulated_cost_usd: Decimal::ZERO,
            iteration: 0, // First iteration
            iteration_outputs: None,
            signal_data: None,
        },
    );

    let state = WorkflowState {
        workflow_id: Uuid::now_v7(),
        definition_name: "test_workflow".to_string(),
        status: WorkflowStatus::Running,
        activities: activities_map,
        state_data: json!({}),
        input: json!({}),
    };

    let ready =
        find_ready_activities(&definition, &state).expect("Failed to find ready activities");
    assert_eq!(
        ready.len(),
        1,
        "Loop activity should be ready on first iteration (back-edge auto-satisfied)"
    );
    assert_eq!(ready[0].key, "process");
}

#[test]
fn test_diamond_dependency_pattern() {
    // Test diamond pattern: A -> (B, C) -> D
    let definition = WorkflowDefinition {
        name: "test_workflow".to_string(),
        activities: vec![
            ActivityDefinition {
                key: "A".to_string(),
                worker: "test".to_string(),
                activity_name: Some("activity".to_string()),
                parameters: Some(HashMap::new()),
                settings: None,
                depends_on: None,
                dependency_of: None,
                output_definitions: None,
                iteration_scoped: false,
                iteration_limit: None,
                is_loop_activity: false,
                streaming: Default::default(),
            },
            ActivityDefinition {
                key: "B".to_string(),
                worker: "test".to_string(),
                activity_name: Some("activity".to_string()),
                parameters: Some(HashMap::new()),
                settings: None,
                depends_on: Some(vec![ActivityRelationship {
                    activity_key: "A".to_string(),
                    conditions: None,
                    is_back_edge: false,
                }]),
                dependency_of: None,
                output_definitions: None,
                iteration_scoped: false,
                iteration_limit: None,
                is_loop_activity: false,
                streaming: Default::default(),
            },
            ActivityDefinition {
                key: "C".to_string(),
                worker: "test".to_string(),
                activity_name: Some("activity".to_string()),
                parameters: Some(HashMap::new()),
                settings: None,
                depends_on: Some(vec![ActivityRelationship {
                    activity_key: "A".to_string(),
                    conditions: None,
                    is_back_edge: false,
                }]),
                dependency_of: None,
                output_definitions: None,
                iteration_scoped: false,
                iteration_limit: None,
                is_loop_activity: false,
                streaming: Default::default(),
            },
            ActivityDefinition {
                key: "D".to_string(),
                worker: "test".to_string(),
                activity_name: Some("activity".to_string()),
                parameters: Some(HashMap::new()),
                settings: None,
                depends_on: Some(vec![
                    ActivityRelationship {
                        activity_key: "B".to_string(),
                        conditions: None,
                        is_back_edge: false,
                    },
                    ActivityRelationship {
                        activity_key: "C".to_string(),
                        conditions: None,
                        is_back_edge: false,
                    },
                ]),
                dependency_of: None,
                output_definitions: None,
                iteration_scoped: false,
                iteration_limit: None,
                is_loop_activity: false,
                streaming: Default::default(),
            },
        ],
    };

    // Only B completed, C still pending - D should not be ready
    let state = create_test_state_with_activities(vec![
        (
            "A",
            WorkflowActivityStatus::Completed,
            Some(vec![ActivityOutput {
                name: "result".to_string(),
                output_type: OutputType::Value,
                value: json!("done"),
            }]),
        ),
        (
            "B",
            WorkflowActivityStatus::Completed,
            Some(vec![ActivityOutput {
                name: "result".to_string(),
                output_type: OutputType::Value,
                value: json!("done"),
            }]),
        ),
        ("C", WorkflowActivityStatus::Pending, None),
        ("D", WorkflowActivityStatus::NotScheduled, None),
    ]);

    let ready =
        find_ready_activities(&definition, &state).expect("Failed to find ready activities");
    assert!(
        !ready.iter().any(|a| a.key == "D"),
        "D should not be ready when C is still pending"
    );

    // Now both B and C completed - D should be ready
    let state = create_test_state_with_activities(vec![
        (
            "A",
            WorkflowActivityStatus::Completed,
            Some(vec![ActivityOutput {
                name: "result".to_string(),
                output_type: OutputType::Value,
                value: json!("done"),
            }]),
        ),
        (
            "B",
            WorkflowActivityStatus::Completed,
            Some(vec![ActivityOutput {
                name: "result".to_string(),
                output_type: OutputType::Value,
                value: json!("done"),
            }]),
        ),
        (
            "C",
            WorkflowActivityStatus::Completed,
            Some(vec![ActivityOutput {
                name: "result".to_string(),
                output_type: OutputType::Value,
                value: json!("done"),
            }]),
        ),
        ("D", WorkflowActivityStatus::NotScheduled, None),
    ]);

    let ready =
        find_ready_activities(&definition, &state).expect("Failed to find ready activities");
    assert_eq!(ready.len(), 1);
    assert_eq!(ready[0].key, "D");
}
