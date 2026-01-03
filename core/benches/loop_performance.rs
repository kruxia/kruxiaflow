// Performance benchmarks for iterative workflow overhead
// Validates that loop detection uses O(1) metadata lookups vs O(V+E) graph traversal

use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use kruxiaflow_core::workflow::{ActivityDefinition, ActivityRelationship, WorkflowDefinition};

/// Create a workflow with N activities in a simple loop
fn create_loop_workflow(num_activities: usize) -> WorkflowDefinition {
    let mut activities = Vec::new();

    // Create a chain of activities: A -> B -> C -> ... -> A (loop back)
    for i in 0..num_activities {
        let current = format!("activity_{}", i);
        let next = format!("activity_{}", (i + 1) % num_activities);

        let is_back_edge = (i + 1) % num_activities == 0; // Last activity loops back to first

        activities.push(ActivityDefinition {
            key: current.clone(),
            worker: "test".to_string(),
            activity_name: Some("task".to_string()),
            parameters: Some(Default::default()),
            settings: None,
            depends_on: Some(vec![ActivityRelationship {
                activity_key: next,
                conditions: Some(vec!["{{true}}".to_string()]),
                is_back_edge, // Precomputed metadata
            }]),
            dependency_of: None,
            output_definitions: None,
            iteration_scoped: true,
            iteration_limit: Some(10),
            is_loop_activity: true, // Precomputed metadata
        });
    }

    WorkflowDefinition {
        name: format!("loop_workflow_{}", num_activities),
        activities,
    }
}

/// Benchmark: Check if activity is in a loop using precomputed metadata (O(1))
fn bench_is_loop_activity_cached(c: &mut Criterion) {
    let mut group = c.benchmark_group("is_loop_activity");

    for size in [5, 10, 20, 50, 100].iter() {
        let workflow = create_loop_workflow(*size);

        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, _| {
            b.iter(|| {
                // O(1) metadata lookup
                let activity = &workflow.activities[0];
                black_box(activity.is_loop_activity)
            });
        });
    }

    group.finish();
}

/// Benchmark: Check if dependency is a back-edge using precomputed metadata (O(1))
fn bench_is_back_edge_cached(c: &mut Criterion) {
    let mut group = c.benchmark_group("is_back_edge");

    for size in [5, 10, 20, 50, 100].iter() {
        let workflow = create_loop_workflow(*size);

        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, _| {
            b.iter(|| {
                // O(1) metadata lookup
                let activity = &workflow.activities[0];
                let dep = &activity.depends_on.as_ref().unwrap()[0];
                black_box(dep.is_back_edge)
            });
        });
    }

    group.finish();
}

/// Benchmark: Simulate the OLD way - checking if activity is in loop via graph traversal
/// This demonstrates what we AVOIDED by using cached metadata
fn bench_is_loop_activity_traversal(c: &mut Criterion) {
    let mut group = c.benchmark_group("is_loop_activity_traversal");

    for size in [5, 10, 20, 50, 100].iter() {
        let workflow = create_loop_workflow(*size);

        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, _| {
            b.iter(|| {
                // O(V+E) graph traversal (OLD way - not used anymore)
                // This simulates searching through all activities and dependencies
                let activity = &workflow.activities[0];
                let mut found = false;

                // Traverse all activities (V)
                for other in &workflow.activities {
                    // Check all dependencies (E)
                    if let Some(deps) = &other.depends_on {
                        for dep in deps {
                            if dep.activity_key == activity.key {
                                // Need to check if this creates a cycle
                                // (simplified - real implementation would use DFS/BFS)
                                found = true;
                                break;
                            }
                        }
                    }
                }

                black_box(found)
            });
        });
    }

    group.finish();
}

/// Benchmark: Iteration over N iterations with cached metadata
fn bench_iteration_loop_overhead(c: &mut Criterion) {
    let mut group = c.benchmark_group("iteration_loop_overhead");

    for iterations in [10, 50, 100].iter() {
        let workflow = create_loop_workflow(5); // 5 activities

        group.bench_with_input(
            BenchmarkId::from_parameter(iterations),
            iterations,
            |b, &n| {
                b.iter(|| {
                    // Simulate N iterations checking loop metadata
                    for _i in 0..n {
                        let activity = &workflow.activities[0];
                        // O(1) checks (what orchestrator does on every iteration)
                        black_box(activity.is_loop_activity);
                        if let Some(deps) = &activity.depends_on {
                            for dep in deps {
                                black_box(dep.is_back_edge);
                            }
                        }
                    }
                });
            },
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_is_loop_activity_cached,
    bench_is_back_edge_cached,
    bench_is_loop_activity_traversal,
    bench_iteration_loop_overhead
);
criterion_main!(benches);
