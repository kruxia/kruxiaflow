//! Tests for DTO conversion functions

use rust_decimal::Decimal;
use std::collections::HashMap;
use kruxiaflow_api::dto;
use kruxiaflow_core::workflow;

#[test]
fn test_workflow_definition_from_core_to_dto() {
    // Create a core WorkflowDefinition
    let core_def = workflow::WorkflowDefinition {
        name: "test_workflow".to_string(),
        activities: vec![
            workflow::ActivityDefinition {
                key: "step1".to_string(),
                worker: "test".to_string(),
                activity_name: Some("action".to_string()),
                parameters: Some(HashMap::from([(
                    "key1".to_string(),
                    serde_json::json!("value1"),
                )])),
                depends_on: None,
                dependency_of: Some(vec![workflow::ActivityRelationship {
                    activity_key: "step2".to_string(),
                    conditions: Some(vec!["condition1".to_string()]),
                    is_back_edge: false,
                }]),
                settings: Some(workflow::ActivitySettings {
                    timeout_seconds: Some(300),
                    retry: Some(workflow::RetryPolicy {
                        max_attempts: 3,
                        strategy: workflow::BackoffStrategy::Exponential,
                        base_seconds: 2,
                        factor: 2.0,
                        max_seconds: 300,
                    }),
                    budget: None,
                    cache: false,
                    cache_ttl: None,
                    iteration_limit: None,
                    delay: None,
                    scheduled_for: None,
                }),
                output_definitions: None,
                iteration_scoped: false,
                iteration_limit: None,
                is_loop_activity: false,
                streaming: Default::default(),
            },
            workflow::ActivityDefinition {
                key: "step2".to_string(),
                worker: "test".to_string(),
                activity_name: None,
                parameters: None,
                depends_on: Some(vec![workflow::ActivityRelationship {
                    activity_key: "step1".to_string(),
                    conditions: None,
                    is_back_edge: false,
                }]),
                dependency_of: None,
                settings: None,
                output_definitions: None,
                iteration_scoped: false,
                iteration_limit: None,
                is_loop_activity: false,
                streaming: Default::default(),
            },
        ],
    };

    // Convert to DTO
    let dto_def: dto::WorkflowDefinition = core_def.clone().into();

    // Verify conversion
    assert_eq!(dto_def.name, "test_workflow");
    assert_eq!(dto_def.activities.len(), 2);

    // Check first activity
    let activity1 = &dto_def.activities[0];
    assert_eq!(activity1.key, "step1");
    assert_eq!(activity1.worker, "test");
    assert_eq!(activity1.activity_name, Some("action".to_string()));
    assert!(activity1.parameters.is_some());
    assert!(activity1.depends_on.is_none());
    assert!(activity1.dependency_of.is_some());

    let following = activity1.dependency_of.as_ref().unwrap();
    assert_eq!(following.len(), 1);
    assert_eq!(following[0].activity_key, "step2");
    assert_eq!(
        following[0].conditions,
        Some(vec!["condition1".to_string()])
    );

    // Check settings conversion
    let settings = activity1.settings.as_ref().unwrap();
    assert_eq!(settings.timeout_seconds, Some(300));

    let retry = settings.retry.as_ref().unwrap();
    assert_eq!(retry.max_attempts, 3);
    assert!(matches!(retry.strategy, dto::BackoffStrategy::Exponential));

    // Check second activity
    let activity2 = &dto_def.activities[1];
    assert_eq!(activity2.key, "step2");
    assert_eq!(activity2.worker, "test");
    assert_eq!(activity2.activity_name, None);
    assert!(activity2.parameters.is_none());
    assert!(activity2.depends_on.is_some());
    assert!(activity2.dependency_of.is_none());
    assert!(activity2.settings.is_none());
}

#[test]
fn test_workflow_definition_from_dto_to_core() {
    // Create a DTO WorkflowDefinition
    let dto_def = dto::WorkflowDefinition {
        name: "test_workflow".to_string(),
        activities: vec![dto::ActivityDefinition {
            key: "step1".to_string(),
            worker: "test".to_string(),
            activity_name: Some("action".to_string()),
            parameters: Some(HashMap::from([(
                "key1".to_string(),
                serde_json::json!("value1"),
            )])),
            depends_on: None,
            dependency_of: Some(vec![dto::ActivityRelationship {
                activity_key: "step2".to_string(),
                conditions: None,
                is_back_edge: false,
            }]),
            settings: Some(dto::ActivitySettings {
                timeout_seconds: Some(300),
                retry: Some(dto::RetrySettings {
                    max_attempts: 5,
                    strategy: dto::BackoffStrategy::Fixed,
                    base_seconds: 2,
                    factor: 2.0,
                    max_seconds: 300,
                }),
                budget: None,
                cache: false,
                cache_ttl: None,
                iteration_limit: None,
                delay: None,
                scheduled_for: None,
            }),
            output_definitions: None,
            iteration_scoped: false,
            iteration_limit: None,
            is_loop_activity: false,
            streaming: Default::default(),
        }],
    };

    // Convert to core
    let core_def: workflow::WorkflowDefinition = dto_def.clone().into();

    // Verify conversion
    assert_eq!(core_def.name, "test_workflow");
    assert_eq!(core_def.activities.len(), 1);

    let activity = &core_def.activities[0];
    assert_eq!(activity.key, "step1");
    assert_eq!(activity.worker, "test");
    assert_eq!(activity.activity_name, Some("action".to_string()));
    assert!(activity.parameters.is_some());
    assert!(activity.dependency_of.is_some());

    // Check settings conversion
    let settings = activity.settings.as_ref().unwrap();
    assert_eq!(settings.timeout_seconds, Some(300));

    let retry = settings.retry.as_ref().unwrap();
    assert_eq!(retry.max_attempts, 5);
    assert!(matches!(retry.strategy, workflow::BackoffStrategy::Fixed));
}

#[test]
fn test_activity_relationship_conversions() {
    // Test core to DTO
    let core_rel = workflow::ActivityRelationship {
        activity_key: "next_step".to_string(),
        conditions: Some(vec!["success".to_string()]),
        is_back_edge: false,
    };

    let dto_rel: dto::ActivityRelationship = core_rel.clone().into();
    assert_eq!(dto_rel.activity_key, "next_step");
    assert_eq!(dto_rel.conditions, Some(vec!["success".to_string()]));

    // Test DTO to core
    let dto_rel2 = dto::ActivityRelationship {
        activity_key: "prev_step".to_string(),
        conditions: None,
        is_back_edge: false,
    };

    let core_rel2: workflow::ActivityRelationship = dto_rel2.clone().into();
    assert_eq!(core_rel2.activity_key, "prev_step");
    assert_eq!(core_rel2.conditions, None);
}

#[test]
fn test_activity_settings_conversions() {
    // Test core to DTO with all fields
    let core_settings = workflow::ActivitySettings {
        timeout_seconds: Some(600),
        retry: Some(workflow::RetryPolicy {
            max_attempts: 10,
            strategy: workflow::BackoffStrategy::Exponential,
            base_seconds: 2,
            factor: 2.0,
            max_seconds: 300,
        }),
        budget: None,
        cache: false,
        cache_ttl: None,
        iteration_limit: None,
        delay: None,
        scheduled_for: None,
    };

    let dto_settings: dto::ActivitySettings = core_settings.clone().into();
    assert_eq!(dto_settings.timeout_seconds, Some(600));
    let retry = dto_settings.retry.as_ref().unwrap();
    assert_eq!(retry.max_attempts, 10);
    assert!(matches!(retry.strategy, dto::BackoffStrategy::Exponential));

    // Test DTO to core with all fields
    let dto_settings2 = dto::ActivitySettings {
        timeout_seconds: Some(1200),
        retry: Some(dto::RetrySettings {
            max_attempts: 7,
            strategy: dto::BackoffStrategy::Fixed,
            base_seconds: 2,
            factor: 2.0,
            max_seconds: 300,
        }),
        budget: None,
        cache: false,
        cache_ttl: None,
        iteration_limit: None,
        delay: None,
        scheduled_for: None,
    };

    let core_settings2: workflow::ActivitySettings = dto_settings2.clone().into();
    assert_eq!(core_settings2.timeout_seconds, Some(1200));
    let retry2 = core_settings2.retry.as_ref().unwrap();
    assert_eq!(retry2.max_attempts, 7);
    assert!(matches!(retry2.strategy, workflow::BackoffStrategy::Fixed));
}

#[test]
fn test_retry_settings_conversions() {
    // Test core to DTO - Exponential
    let core_retry = workflow::RetryPolicy {
        max_attempts: 3,
        strategy: workflow::BackoffStrategy::Exponential,
        base_seconds: 2,
        factor: 2.0,
        max_seconds: 300,
    };

    let dto_retry: dto::RetrySettings = core_retry.clone().into();
    assert_eq!(dto_retry.max_attempts, 3);
    assert!(matches!(
        dto_retry.strategy,
        dto::BackoffStrategy::Exponential
    ));

    // Test core to DTO - Fixed
    let core_retry2 = workflow::RetryPolicy {
        max_attempts: 5,
        strategy: workflow::BackoffStrategy::Fixed,
        base_seconds: 2,
        factor: 2.0,
        max_seconds: 300,
    };

    let dto_retry2: dto::RetrySettings = core_retry2.clone().into();
    assert_eq!(dto_retry2.max_attempts, 5);
    assert!(matches!(dto_retry2.strategy, dto::BackoffStrategy::Fixed));

    // Test DTO to core - Exponential
    let dto_retry3 = dto::RetrySettings {
        max_attempts: 8,
        strategy: dto::BackoffStrategy::Exponential,
        base_seconds: 2,
        factor: 2.0,
        max_seconds: 300,
    };

    let core_retry3: workflow::RetryPolicy = dto_retry3.clone().into();
    assert_eq!(core_retry3.max_attempts, 8);
    assert!(matches!(
        core_retry3.strategy,
        workflow::BackoffStrategy::Exponential
    ));

    // Test DTO to core - Fixed
    let dto_retry4 = dto::RetrySettings {
        max_attempts: 12,
        strategy: dto::BackoffStrategy::Fixed,
        base_seconds: 2,
        factor: 2.0,
        max_seconds: 300,
    };

    let core_retry4: workflow::RetryPolicy = dto_retry4.clone().into();
    assert_eq!(core_retry4.max_attempts, 12);
    assert!(matches!(
        core_retry4.strategy,
        workflow::BackoffStrategy::Fixed
    ));
}

#[test]
fn test_backoff_strategy_conversions() {
    // Test core to DTO
    let core_exponential = workflow::BackoffStrategy::Exponential;
    let dto_exponential: dto::BackoffStrategy = core_exponential.into();
    assert!(matches!(dto_exponential, dto::BackoffStrategy::Exponential));

    let core_fixed = workflow::BackoffStrategy::Fixed;
    let dto_fixed: dto::BackoffStrategy = core_fixed.into();
    assert!(matches!(dto_fixed, dto::BackoffStrategy::Fixed));

    // Test DTO to core
    let dto_exponential2 = dto::BackoffStrategy::Exponential;
    let core_exponential2: workflow::BackoffStrategy = dto_exponential2.into();
    assert!(matches!(
        core_exponential2,
        workflow::BackoffStrategy::Exponential
    ));

    let dto_fixed2 = dto::BackoffStrategy::Fixed;
    let core_fixed2: workflow::BackoffStrategy = dto_fixed2.into();
    assert!(matches!(core_fixed2, workflow::BackoffStrategy::Fixed));
}

#[test]
fn test_activity_definition_with_minimal_fields() {
    // Test core to DTO with minimal fields
    let core_activity = workflow::ActivityDefinition {
        key: "minimal".to_string(),
        worker: "test".to_string(),
        activity_name: None,
        parameters: None,
        depends_on: None,
        dependency_of: None,
        settings: None,
        output_definitions: None,
        iteration_scoped: false,
        iteration_limit: None,
        is_loop_activity: false,
        streaming: Default::default(),
    };

    let dto_activity: dto::ActivityDefinition = core_activity.clone().into();
    assert_eq!(dto_activity.key, "minimal");
    assert_eq!(dto_activity.worker, "test");
    assert!(dto_activity.activity_name.is_none());
    assert!(dto_activity.parameters.is_none());
    assert!(dto_activity.depends_on.is_none());
    assert!(dto_activity.dependency_of.is_none());
    assert!(dto_activity.settings.is_none());

    // Test DTO to core with minimal fields
    let dto_activity2 = dto::ActivityDefinition {
        key: "minimal2".to_string(),
        worker: "test2".to_string(),
        activity_name: None,
        parameters: None,
        depends_on: None,
        dependency_of: None,
        settings: None,
        output_definitions: None,
        iteration_scoped: false,
        iteration_limit: None,
        is_loop_activity: false,
        streaming: Default::default(),
    };

    let core_activity2: workflow::ActivityDefinition = dto_activity2.clone().into();
    assert_eq!(core_activity2.key, "minimal2");
    assert_eq!(core_activity2.worker, "test2");
    assert!(core_activity2.activity_name.is_none());
    assert!(core_activity2.parameters.is_none());
    assert!(core_activity2.depends_on.is_none());
    assert!(core_activity2.dependency_of.is_none());
    assert!(core_activity2.settings.is_none());
}

#[test]
fn test_workflow_with_multiple_activities_and_relationships() {
    // Create a complex workflow
    let core_def = workflow::WorkflowDefinition {
        name: "complex_workflow".to_string(),
        activities: vec![
            workflow::ActivityDefinition {
                key: "start".to_string(),
                worker: "control".to_string(),
                activity_name: None,
                parameters: None,
                depends_on: None,
                dependency_of: Some(vec![
                    workflow::ActivityRelationship {
                        activity_key: "middle1".to_string(),
                        conditions: None,
                        is_back_edge: false,
                    },
                    workflow::ActivityRelationship {
                        activity_key: "middle2".to_string(),
                        conditions: Some(vec!["branch".to_string()]),
                        is_back_edge: false,
                    },
                ]),
                settings: None,
                output_definitions: None,
                iteration_scoped: false,
                iteration_limit: None,
                is_loop_activity: false,
                streaming: Default::default(),
            },
            workflow::ActivityDefinition {
                key: "middle1".to_string(),
                worker: "processing".to_string(),
                activity_name: Some("process".to_string()),
                parameters: Some(HashMap::from([(
                    "type".to_string(),
                    serde_json::json!("fast"),
                )])),
                depends_on: Some(vec![workflow::ActivityRelationship {
                    activity_key: "start".to_string(),
                    conditions: None,
                    is_back_edge: false,
                }]),
                dependency_of: Some(vec![workflow::ActivityRelationship {
                    activity_key: "end".to_string(),
                    conditions: None,
                    is_back_edge: false,
                }]),
                settings: Some(workflow::ActivitySettings {
                    timeout_seconds: Some(100),
                    retry: None,
                    budget: None,
                    cache: false,
                    cache_ttl: None,
                    iteration_limit: None,
                    delay: None,
                    scheduled_for: None,
                }),
                output_definitions: None,
                iteration_scoped: false,
                iteration_limit: None,
                is_loop_activity: false,
                streaming: Default::default(),
            },
            workflow::ActivityDefinition {
                key: "middle2".to_string(),
                worker: "processing".to_string(),
                activity_name: Some("process".to_string()),
                parameters: Some(HashMap::from([(
                    "type".to_string(),
                    serde_json::json!("slow"),
                )])),
                depends_on: Some(vec![workflow::ActivityRelationship {
                    activity_key: "start".to_string(),
                    conditions: Some(vec!["branch".to_string()]),
                    is_back_edge: false,
                }]),
                dependency_of: Some(vec![workflow::ActivityRelationship {
                    activity_key: "end".to_string(),
                    conditions: None,
                    is_back_edge: false,
                }]),
                settings: Some(workflow::ActivitySettings {
                    timeout_seconds: Some(500),
                    retry: Some(workflow::RetryPolicy {
                        max_attempts: 2,
                        strategy: workflow::BackoffStrategy::Fixed,
                        base_seconds: 2,
                        factor: 2.0,
                        max_seconds: 300,
                    }),
                    budget: None,
                    cache: false,
                    cache_ttl: None,
                    iteration_limit: None,
                    delay: None,
                    scheduled_for: None,
                }),
                output_definitions: None,
                iteration_scoped: false,
                iteration_limit: None,
                is_loop_activity: false,
                streaming: Default::default(),
            },
            workflow::ActivityDefinition {
                key: "end".to_string(),
                worker: "control".to_string(),
                activity_name: None,
                parameters: None,
                depends_on: Some(vec![
                    workflow::ActivityRelationship {
                        activity_key: "middle1".to_string(),
                        conditions: None,
                        is_back_edge: false,
                    },
                    workflow::ActivityRelationship {
                        activity_key: "middle2".to_string(),
                        conditions: None,
                        is_back_edge: false,
                    },
                ]),
                dependency_of: None,
                settings: None,
                output_definitions: None,
                iteration_scoped: false,
                iteration_limit: None,
                is_loop_activity: false,
                streaming: Default::default(),
            },
        ],
    };

    // Convert to DTO and back
    let dto_def: dto::WorkflowDefinition = core_def.clone().into();
    let core_def_back: workflow::WorkflowDefinition = dto_def.clone().into();

    // Verify round-trip conversion preserves structure
    assert_eq!(core_def_back.name, core_def.name);
    assert_eq!(core_def_back.activities.len(), core_def.activities.len());

    for (original, round_trip) in core_def
        .activities
        .iter()
        .zip(core_def_back.activities.iter())
    {
        assert_eq!(original.key, round_trip.key);
        assert_eq!(original.worker, round_trip.worker);
        assert_eq!(original.activity_name, round_trip.activity_name);

        // Check relationship counts match
        assert_eq!(
            original.depends_on.as_ref().map(|p| p.len()),
            round_trip.depends_on.as_ref().map(|p| p.len())
        );
        assert_eq!(
            original.dependency_of.as_ref().map(|f| f.len()),
            round_trip.dependency_of.as_ref().map(|f| f.len())
        );
    }
}

// ============================================================================
// Budget settings tests
// ============================================================================

#[test]
fn test_budget_settings_core_to_dto() {
    let core_budget = workflow::BudgetSettings {
        limit: Decimal::new(1000, 2), // $10.00
        action: workflow::BudgetAction::Abort,
    };

    let dto_budget: dto::BudgetSettings = core_budget.into();
    assert_eq!(dto_budget.limit, Decimal::new(1000, 2));
    assert!(matches!(dto_budget.action, dto::BudgetAction::Abort));
}

#[test]
fn test_budget_settings_dto_to_core() {
    let dto_budget = dto::BudgetSettings {
        limit: Decimal::new(5000, 2), // $50.00
        action: dto::BudgetAction::Continue,
    };

    let core_budget: workflow::BudgetSettings = dto_budget.into();
    assert_eq!(core_budget.limit, Decimal::new(5000, 2));
    assert!(matches!(
        core_budget.action,
        workflow::BudgetAction::Continue
    ));
}

#[test]
fn test_budget_action_abort_conversion() {
    let core_abort = workflow::BudgetAction::Abort;
    let dto_abort: dto::BudgetAction = core_abort.into();
    assert!(matches!(dto_abort, dto::BudgetAction::Abort));

    let core_back: workflow::BudgetAction = dto_abort.into();
    assert!(matches!(core_back, workflow::BudgetAction::Abort));
}

#[test]
fn test_budget_action_continue_conversion() {
    let core_continue = workflow::BudgetAction::Continue;
    let dto_continue: dto::BudgetAction = core_continue.into();
    assert!(matches!(dto_continue, dto::BudgetAction::Continue));

    let core_back: workflow::BudgetAction = dto_continue.into();
    assert!(matches!(core_back, workflow::BudgetAction::Continue));
}

// ============================================================================
// Output definition tests
// ============================================================================

#[test]
fn test_activity_output_definition_core_to_dto() {
    let core_output = workflow::ActivityOutputDefinition {
        name: "result".to_string(),
        output_type: workflow::OutputType::Value,
    };

    let dto_output: dto::ActivityOutputDefinition = core_output.into();
    assert_eq!(dto_output.name, "result");
    assert!(matches!(dto_output.output_type, dto::OutputType::Value));
}

#[test]
fn test_activity_output_definition_dto_to_core() {
    let dto_output = dto::ActivityOutputDefinition {
        name: "file_output".to_string(),
        output_type: dto::OutputType::File,
    };

    let core_output: workflow::ActivityOutputDefinition = dto_output.into();
    assert_eq!(core_output.name, "file_output");
    assert!(matches!(
        core_output.output_type,
        workflow::OutputType::File
    ));
}

#[test]
fn test_output_type_value_conversion() {
    let core_value = workflow::OutputType::Value;
    let dto_value: dto::OutputType = core_value.into();
    assert!(matches!(dto_value, dto::OutputType::Value));

    let core_back: workflow::OutputType = dto_value.into();
    assert!(matches!(core_back, workflow::OutputType::Value));
}

#[test]
fn test_output_type_file_conversion() {
    let core_file = workflow::OutputType::File;
    let dto_file: dto::OutputType = core_file.into();
    assert!(matches!(dto_file, dto::OutputType::File));

    let core_back: workflow::OutputType = dto_file.into();
    assert!(matches!(core_back, workflow::OutputType::File));
}

#[test]
fn test_output_type_folder_conversion() {
    let core_folder = workflow::OutputType::Folder;
    let dto_folder: dto::OutputType = core_folder.into();
    assert!(matches!(dto_folder, dto::OutputType::Folder));

    let core_back: workflow::OutputType = dto_folder.into();
    assert!(matches!(core_back, workflow::OutputType::Folder));
}

// ============================================================================
// Activity with output definitions
// ============================================================================

#[test]
fn test_activity_with_output_definitions() {
    let core_activity = workflow::ActivityDefinition {
        key: "generate_report".to_string(),
        worker: "reports".to_string(),
        activity_name: Some("generate".to_string()),
        parameters: None,
        depends_on: None,
        dependency_of: None,
        settings: None,
        output_definitions: Some(vec![
            workflow::ActivityOutputDefinition {
                name: "summary".to_string(),
                output_type: workflow::OutputType::Value,
            },
            workflow::ActivityOutputDefinition {
                name: "report_file".to_string(),
                output_type: workflow::OutputType::File,
            },
        ]),
        iteration_scoped: false,
        iteration_limit: None,
        is_loop_activity: false,
        streaming: Default::default(),
    };

    let dto_activity: dto::ActivityDefinition = core_activity.into();

    assert!(dto_activity.output_definitions.is_some());
    let outputs = dto_activity.output_definitions.unwrap();
    assert_eq!(outputs.len(), 2);
    assert_eq!(outputs[0].name, "summary");
    assert!(matches!(outputs[0].output_type, dto::OutputType::Value));
    assert_eq!(outputs[1].name, "report_file");
    assert!(matches!(outputs[1].output_type, dto::OutputType::File));
}

// ============================================================================
// Activity with budget settings
// ============================================================================

#[test]
fn test_activity_with_budget_settings() {
    let core_activity = workflow::ActivityDefinition {
        key: "expensive_task".to_string(),
        worker: "compute".to_string(),
        activity_name: Some("process".to_string()),
        parameters: None,
        depends_on: None,
        dependency_of: None,
        settings: Some(workflow::ActivitySettings {
            timeout_seconds: Some(600),
            retry: None,
            budget: Some(workflow::BudgetSettings {
                limit: Decimal::new(10000, 2), // $100.00
                action: workflow::BudgetAction::Abort,
            }),
            cache: false,
            cache_ttl: None,
            iteration_limit: None,
            delay: None,
            scheduled_for: None,
        }),
        output_definitions: None,
        iteration_scoped: false,
        iteration_limit: None,
        is_loop_activity: false,
        streaming: Default::default(),
    };

    let dto_activity: dto::ActivityDefinition = core_activity.into();

    let settings = dto_activity.settings.unwrap();
    assert!(settings.budget.is_some());
    let budget = settings.budget.unwrap();
    assert_eq!(budget.limit, Decimal::new(10000, 2));
    assert!(matches!(budget.action, dto::BudgetAction::Abort));
}

// ============================================================================
// Activity settings with delay and scheduled_for
// ============================================================================

#[test]
fn test_activity_settings_with_scheduling() {
    let core_settings = workflow::ActivitySettings {
        timeout_seconds: None,
        retry: None,
        budget: None,
        cache: false,
        cache_ttl: None,
        iteration_limit: None,
        delay: Some("30s".to_string()),
        scheduled_for: None,
    };

    let dto_settings: dto::ActivitySettings = core_settings.into();
    assert_eq!(dto_settings.delay, Some("30s".to_string()));
    assert!(dto_settings.scheduled_for.is_none());
}

#[test]
fn test_activity_settings_with_absolute_schedule() {
    let core_settings = workflow::ActivitySettings {
        timeout_seconds: None,
        retry: None,
        budget: None,
        cache: false,
        cache_ttl: None,
        iteration_limit: None,
        delay: None,
        scheduled_for: Some("2025-12-01T09:00:00Z".to_string()),
    };

    let dto_settings: dto::ActivitySettings = core_settings.into();
    assert!(dto_settings.delay.is_none());
    assert_eq!(
        dto_settings.scheduled_for,
        Some("2025-12-01T09:00:00Z".to_string())
    );
}

// ============================================================================
// Activity settings with caching
// ============================================================================

#[test]
fn test_activity_settings_with_caching() {
    let core_settings = workflow::ActivitySettings {
        timeout_seconds: None,
        retry: None,
        budget: None,
        cache: true,
        cache_ttl: Some(3600),
        iteration_limit: None,
        delay: None,
        scheduled_for: None,
    };

    let dto_settings: dto::ActivitySettings = core_settings.into();
    assert!(dto_settings.cache);
    assert_eq!(dto_settings.cache_ttl, Some(3600));
}

// ============================================================================
// Back-edge relationship tests
// ============================================================================

#[test]
fn test_activity_relationship_with_back_edge() {
    let core_rel = workflow::ActivityRelationship {
        activity_key: "loop_start".to_string(),
        conditions: Some(vec!["{{result.continue}}".to_string()]),
        is_back_edge: true,
    };

    let dto_rel: dto::ActivityRelationship = core_rel.into();
    assert!(dto_rel.is_back_edge);
    assert_eq!(
        dto_rel.conditions,
        Some(vec!["{{result.continue}}".to_string()])
    );

    // Convert back
    let core_back: workflow::ActivityRelationship = dto_rel.into();
    assert!(core_back.is_back_edge);
}

// ============================================================================
// Loop activity tests
// ============================================================================

#[test]
fn test_activity_with_loop_configuration() {
    let core_activity = workflow::ActivityDefinition {
        key: "iterating_task".to_string(),
        worker: "processor".to_string(),
        activity_name: Some("process_batch".to_string()),
        parameters: None,
        depends_on: Some(vec![workflow::ActivityRelationship {
            activity_key: "iterating_task".to_string(),
            conditions: Some(vec!["{{iterating_task.has_more}}".to_string()]),
            is_back_edge: true,
        }]),
        dependency_of: None,
        settings: None,
        output_definitions: None,
        iteration_scoped: true,
        iteration_limit: Some(100),
        is_loop_activity: true,
        streaming: Default::default(),
    };

    let dto_activity: dto::ActivityDefinition = core_activity.into();

    assert!(dto_activity.iteration_scoped);
    assert_eq!(dto_activity.iteration_limit, Some(100));
    assert!(dto_activity.is_loop_activity);

    // Check back-edge preserved
    let deps = dto_activity.depends_on.unwrap();
    assert!(deps[0].is_back_edge);
}
