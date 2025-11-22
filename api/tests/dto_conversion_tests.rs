//! Tests for DTO conversion functions

use std::collections::HashMap;
use streamflow_api::dto;
use streamflow_core::workflow;

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
                }),
                output_definitions: None,
                iteration_scoped: false,
                iteration_limit: None,
                is_loop_activity: false,
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
            }),
            output_definitions: None,
            iteration_scoped: false,
            iteration_limit: None,
            is_loop_activity: false,
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
                }),
                output_definitions: None,
                iteration_scoped: false,
                iteration_limit: None,
                is_loop_activity: false,
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
                }),
                output_definitions: None,
                iteration_scoped: false,
                iteration_limit: None,
                is_loop_activity: false,
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
