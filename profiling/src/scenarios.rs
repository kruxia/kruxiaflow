use kruxiaflow_core::workflow::definition::{
    ActivityDefinition, ActivityRelationship, WorkflowDefinition,
};
use std::collections::HashMap;

/// Create a sequential workflow definition with specified number of activities
pub fn create_sequential_workflow(num_activities: usize) -> WorkflowDefinition {
    let mut activities = Vec::new();

    for i in 0..num_activities {
        let key = format!("activity_{}", i);
        let dependency_of = if i < num_activities - 1 {
            Some(vec![ActivityRelationship {
                activity_key: format!("activity_{}", i + 1),
                conditions: None,
                is_back_edge: false,
            }])
        } else {
            None
        };

        activities.push(ActivityDefinition {
            key,
            worker: "std".to_string(),
            activity_name: Some("echo".to_string()),
            parameters: Some(HashMap::new()),
            output_definitions: None,
            depends_on: None,
            dependency_of,
            settings: None,
            iteration_scoped: false,
            iteration_limit: None,
            is_loop_activity: false,
            streaming: Default::default(),
        });
    }

    WorkflowDefinition {
        name: format!("sequential_bench_{}", num_activities),
        activities,
    }
}

/// Create a parallel workflow definition (fan-out, then fan-in)
pub fn create_parallel_workflow(num_parallel: usize) -> WorkflowDefinition {
    let mut activities = vec![
        // Start activity
        ActivityDefinition {
            key: "start".to_string(),
            worker: "std".to_string(),
            activity_name: Some("echo".to_string()),
            parameters: Some(HashMap::new()),
            output_definitions: None,
            depends_on: None,
            dependency_of: Some(
                (0..num_parallel)
                    .map(|i| ActivityRelationship {
                        activity_key: format!("parallel_{}", i),
                        conditions: None,
                        is_back_edge: false,
                    })
                    .collect(),
            ),
            settings: None,
            iteration_scoped: false,
            iteration_limit: None,
            is_loop_activity: false,
            streaming: Default::default(),
        },
    ];

    // Parallel activities
    for i in 0..num_parallel {
        activities.push(ActivityDefinition {
            key: format!("parallel_{}", i),
            worker: "std".to_string(),
            activity_name: Some("echo".to_string()),
            parameters: Some(HashMap::new()),
            output_definitions: None,
            depends_on: Some(vec![ActivityRelationship {
                activity_key: "start".to_string(),
                conditions: None,
                is_back_edge: false,
            }]),
            dependency_of: Some(vec![ActivityRelationship {
                activity_key: "end".to_string(),
                conditions: None,
                is_back_edge: false,
            }]),
            settings: None,
            iteration_scoped: false,
            iteration_limit: None,
            is_loop_activity: false,
            streaming: Default::default(),
        });
    }

    // End activity (fan-in)
    activities.push(ActivityDefinition {
        key: "end".to_string(),
        worker: "std".to_string(),
        activity_name: Some("echo".to_string()),
        parameters: Some(HashMap::new()),
        output_definitions: None,
        depends_on: Some(
            (0..num_parallel)
                .map(|i| ActivityRelationship {
                    activity_key: format!("parallel_{}", i),
                    conditions: None,
                    is_back_edge: false,
                })
                .collect(),
        ),
        dependency_of: None,
        settings: None,
        iteration_scoped: false,
        iteration_limit: None,
        is_loop_activity: false,
        streaming: Default::default(),
    });

    WorkflowDefinition {
        name: format!("parallel_bench_{}", num_parallel),
        activities,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_sequential_workflow() {
        let mut workflow = create_sequential_workflow(5);
        assert_eq!(workflow.name, "sequential_bench_5");
        assert_eq!(workflow.activities.len(), 5);

        // Validate structure
        assert!(workflow.validate().is_ok());

        // Check first activity has following
        assert!(workflow.activities[0].dependency_of.is_some());

        // Check last activity has no following
        assert!(workflow.activities[4].dependency_of.is_none());
    }

    #[test]
    fn test_create_parallel_workflow() {
        let mut workflow = create_parallel_workflow(10);
        assert_eq!(workflow.name, "parallel_bench_10");

        // Start + 10 parallel + end = 12 total
        assert_eq!(workflow.activities.len(), 12);

        // Validate structure
        assert!(workflow.validate().is_ok());

        // Check start activity fans out to 10 activities
        let start_activity = &workflow.activities[0];
        assert_eq!(start_activity.key, "start");
        assert_eq!(start_activity.dependency_of.as_ref().unwrap().len(), 10);

        // Check end activity has 10 preceding
        let end_activity = &workflow.activities[11];
        assert_eq!(end_activity.key, "end");
        assert_eq!(end_activity.depends_on.as_ref().unwrap().len(), 10);
    }
}
