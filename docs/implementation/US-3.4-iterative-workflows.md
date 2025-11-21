# US-3.4: Iterative Workflows (Loops) - Implementation Plan

**Epic**: Epic 3 - YAML Workflow Definition Language
**User Story**: US-3.4
**Status**: 🔲 Not Started
**Priority**: High (Required for Example 6 - Agentic Research)
**Estimated Duration**: 4-5 days
**Dependencies**: US-3.1 (Sequential Workflows) ✅ Complete, US-3.3 (Parallel Execution) ✅ Complete

---

## User Story

**As** an AI startup engineer
**I want** workflows to loop until a condition is met and access results from all iterations
**So that** I can implement agentic research patterns (evaluate → search more if needed, building on previous findings)

### Acceptance Criteria

- Edge from later activity back to earlier activity (loop via `depends_on`)
- **Iteration-scoped outputs**: Activities declare `iteration_scoped: true` to store separate results per iteration
- **Access all iterations**: `{{activity_key.output_name}}` returns array of all iteration results.
- Conditional loop exit: `{{evaluate.sufficient == true}}`
- Iteration counter: `{{ACTIVITY.iteration}}`
- Maximum iteration limits to prevent infinite loops
- **Budget accumulation**: `accumulated_cost_usd` tracks total cost across ALL iterations; budget limits apply to total, not per-iteration
- **Example**: Research agent searches → evaluates if sufficient → loops back with context of all previous searches → compiles report from all iterations
- **Storage**: Framework stores iteration-scoped results as arrays, making all iterations accessible to downstream activities

---

## Architecture Overview

### Key Concept: Loops as Back-Edges in DAG

Loops are created by adding a `depends_on` edge from a later activity back to an earlier activity in the workflow graph. This creates a cycle that the orchestrator must handle specially:

```mermaid
flowchart TB
    Start[initialize_search]
    Search[perform_search]
    Evaluate[evaluate_results]
    Report[compile_report]

    Start -->|depends_on| Search
    Search -->|depends_on| Evaluate

    Evaluate -->|Loop Exit:<br/>sufficient=true| Report
    Evaluate -.->|Loop Back:<br/>sufficient=false| Search

    style Search fill:#e1f5ff
    style Evaluate fill:#ffe1e1
```

**Loop Detection**: The orchestrator must detect back-edges during evaluation and handle them as iteration triggers rather than circular dependency errors.

**Iteration-Scoped Storage**: Activities marked with `iteration_scoped: true` store their outputs grouped by name as arrays: `{ "output_name": [value0, value1, value2, ...] }`. This design:
- Matches template access patterns exactly (no transformation needed)
- Enables direct array operations via MiniJinja filters
- Simplifies implementation (no flattening step in template resolver)

```yaml
activities:
  - key: perform_search
    worker: builtin
    activity_name: http_request
    iteration_scoped: true  # Store results from each iteration as array
    parameters:
      query: "{{INPUT.topic}}"
      previous_results: "{{perform_search.results}}"  # Array of all iterations
      latest_result: "{{perform_search.results | last}}"  # Latest iteration only
    outputs:
      - name: results
    depends_on:
      - initialize_search
      - activity_key: evaluate_results
        conditions:
          - "{{evaluate_results.sufficient | last == false}}"  # Check latest iteration

  - key: evaluate_results
    worker: builtin
    activity_name: llm_call
    iteration_scoped: true
    parameters:
      prompt: "Evaluate if search results are sufficient..."
      all_results: "{{perform_search.results}}"  # All iterations (array)
      iteration_count: "{{perform_search.results | length}}"  # Number of iterations
    outputs:
      - name: sufficient
      - name: confidence
    depends_on:
      - perform_search
```

### Current State Analysis

**Existing Capabilities**:
- ✅ Dependency evaluation via `depends_on` relationships
- ✅ Conditional execution via `conditions` on dependencies
- ✅ Activity state tracking in `WorkflowState`
- ✅ Template resolution via MiniJinja
- ✅ PostgreSQL advisory locks prevent race conditions

**What Needs to Change**:
- 🔲 **Loop detection**: Distinguish between invalid circular dependencies and valid iteration loops
- 🔲 **Iteration tracking**: Add iteration counter to `ActivityState`
- 🔲 **Iteration-scoped storage**: Store outputs as arrays for iteration-scoped activities
- 🔲 **Template resolution**: Return arrays for iteration-scoped activities (MiniJinja handles array operations)
- 🔲 **Loop limits**: Enforce maximum iteration count
- 🔲 **Budget tracking**: Track accumulated cost across iterations
- 🔲 **Back-edge evaluation**: Schedule loop-back activities correctly

---

## Performance & Tech Debt Improvements

As part of implementing iterative workflows, we're addressing orchestrator hot-path performance:

### Validation Strategy
- **Validate once** at workflow registration (`POST /api/v1/workflow_definitions`)
- **Cache all computed metadata** in database alongside activities JSON
- **No re-validation** when loading definitions during orchestration
- **Workflow definitions are immutable** - changes create new definitions (no migration needed)
- **Result**: Orchestrator works with pre-analyzed, optimized, immutable definitions

### Precomputed Metadata (No Runtime Cost)
1. **Loop detection** (`is_loop_activity` per activity)
   - Was: O(V+E) transitive dependency traversal on every activity completion
   - Now: O(1) boolean check
2. **Back-edge marking** (`is_back_edge` per dependency)
   - Was: O(V+E) graph analysis on every dependency evaluation
   - Now: O(1) boolean check
3. **Future opportunities**:
   - Topological order (pre-sort activities)
   - Dependency depth (for scheduling priority)
   - Parallel execution groups (static analysis of parallelizable branches)

### Database Storage
- Computed metadata stored in `activities` JSON column
- Uses `#[serde(default, skip_serializing_if = "is_false")]` pattern
- Backward compatible when deserializing (missing fields default to false)
  - New fields won't break deserialization of old definitions (if any existed before immutability)
  - Workflow instances (ActivityState) can have missing fields from earlier code versions
- Clean YAML/JSON (metadata only appears when true)

---

## Implementation Tasks

### 1. 🔲 Extend ActivityState for Iteration Tracking

**File**: `core/src/orchestrator/workflow_state.rs`

**Changes Needed**:

Add iteration tracking fields to `ActivityState`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ActivityState {
    pub key: String,
    pub status: WorkflowActivityStatus,
    pub outputs: Option<Vec<ActivityOutput>>,
    pub error: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub attempt: u32,
    pub last_error: Option<String>,
    pub accumulated_cost_usd: Decimal,

    // NEW: Iteration tracking
    /// Current iteration number (0-based)
    #[serde(default)]
    pub iteration: u32,

    /// History of outputs from all iterations (only for iteration_scoped activities)
    /// Outputs are grouped by name: { "output_name": [value0, value1, value2, ...] }
    /// This matches the template access pattern: {{activity.output_name}} returns the array
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iteration_outputs: Option<HashMap<String, Vec<Value>>>,
}

impl ActivityState {
    /// Increment iteration counter (for ALL looping activities, regardless of iteration_scoped)
    /// NOTE: accumulated_cost_usd is NOT reset - it tracks total across all iterations
    pub fn increment_iteration(&mut self) {
        self.iteration += 1;
        // IMPORTANT: accumulated_cost_usd is NOT reset here
        // Budget limits apply to the sum of all iterations, not per-iteration
    }

    /// Archive outputs to iteration history (only for iteration_scoped activities)
    pub fn archive_iteration_outputs(&mut self, current_outputs: Vec<ActivityOutput>) {
        // Only archive if iteration_outputs is initialized (iteration_scoped activities)
        if let Some(history) = &mut self.iteration_outputs {
            for output in current_outputs {
                history
                    .entry(output.name.clone())
                    .or_insert_with(Vec::new)
                    .push(output.value);
            }
        }
    }

    /// Get the latest value for a specific output across all iterations
    pub fn get_latest_output_value(&self, output_name: &str) -> Option<&Value> {
        self.iteration_outputs
            .as_ref()?
            .get(output_name)?
            .last()
    }

    /// Get all values for a specific output across all iterations
    pub fn get_output_values(&self, output_name: &str) -> Option<&Vec<Value>> {
        self.iteration_outputs.as_ref()?.get(output_name)
    }
}
```

**Runtime State Compatibility**: Running workflow instances (ActivityState in database) created before this feature won't have `iteration` or `iteration_outputs` fields. The `#[serde(default)]` attribute ensures these deserialize as `0` and `None` respectively, allowing existing workflow instances to continue running after code deployment.

---

### 2. 🔲 Extend ActivityDefinition for Loop Configuration

**File**: `core/src/workflow/definition.rs`

**Changes Needed**:

Add loop-related configuration to `ActivityDefinition`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ActivityDefinition {
    pub key: String,
    pub worker: Option<String>,
    pub activity_name: String,
    pub parameters: Option<HashMap<String, Value>>,
    pub outputs: Option<Vec<ActivityOutputDefinition>>,
    pub depends_on: Option<Vec<DependencyRelationship>>,
    pub dependency_of: Option<Vec<DependencyRelationship>>,
    pub settings: Option<ActivitySettings>,

    // NEW: Loop configuration
    /// Whether to store separate outputs for each iteration
    #[serde(default)]
    pub iteration_scoped: bool,

    /// Maximum number of iterations (prevents infinite loops)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iteration_limit: Option<u32>,

    /// Cached metadata: whether this activity is part of a loop (has back-edge)
    /// Computed during validation, stored in database
    /// NOT specified in YAML (ignored if present)
    #[serde(default, skip_serializing_if = "is_false")]
    pub is_loop_activity: bool,
}

// Helper for serde skip_serializing_if
fn is_false(b: &bool) -> bool {
    !b
}

// Mark back-edges on dependency relationships (computed during validation)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DependencyRelationship {
    pub activity_key: String,
    pub conditions: Option<Vec<String>>,

    /// Cached metadata: whether this dependency is a back-edge (loop)
    /// Computed during validation, stored in database
    /// NOT specified in YAML (ignored if present)
    #[serde(default, skip_serializing_if = "is_false")]
    pub is_back_edge: bool,
}

// Also add to ActivitySettings for global loop limits
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ActivitySettings {
    pub timeout_seconds: Option<u64>,
    pub retry: Option<RetrySettings>,
    pub budget: Option<BudgetSettings>,
    pub cache: Option<CacheSettings>,

    // NEW: Per-activity iteration limit
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iteration_limit: Option<u32>,
}
```

**YAML Example**:
```yaml
activities:
  - key: perform_search
    worker: builtin
    activity_name: http_request
    iteration_scoped: true      # Enable iteration tracking
    iteration_limit: 10          # Prevent infinite loops
    parameters:
      query: "{{INPUT.topic}}"
      context: "{{perform_search[*].results}}"
    outputs:
      - name: results
```

**Important Distinction**:
- `is_loop_activity` (computed, not in YAML) → controls whether iteration counter is tracked
  - Automatically detected during validation by analyzing workflow graph
  - Used for iteration limit enforcement
  - Set to `true` for ALL activities participating in loops
- `iteration_scoped` (user-specified in YAML) → controls whether outputs are stored as arrays
  - `iteration_scoped: true` → outputs stored as arrays: `{{activity.output}}` returns `[val1, val2, val3]`
  - `iteration_scoped: false` → outputs stored as single value: `{{activity.output}}` returns `val3` (latest)
  - Choose `true` when you need access to all iteration results
  - Choose `false` (or omit) when you only need the latest result

**Example: Loop without iteration_scoped**:
```yaml
# Valid: iteration counter tracked, iteration limits enforced
# But outputs only retain latest value (no array)
activities:
  - key: poll_service
    # iteration_scoped: false (default)
    iteration_limit: 100
    depends_on:
      - activity_key: check_ready
        conditions:
          - "{{check_ready.status | last}} != 'ready'"  # ❌ Won't work - no array
          - "{{check_ready.status}} != 'ready'"         # ✅ Works - single value
```

---

### Loop Exit Patterns

Loops can use three different exit patterns depending on requirements:

**Pattern 1: Fixed Iterations Only**
Use when you know exactly how many times to loop. No condition needed.

```yaml
# Example: 12-issue newsletter subscription
activities:
  - key: send_monthly_issue
    iteration_scoped: true
    iteration_limit: 12          # Exactly 12 issues, no condition needed
    parameters:
      issue_number: "{{ACTIVITY.iteration + 1}}"
      content: "{{generate_content.text}}"
    depends_on:
      - generate_content
      - activity_key: send_monthly_issue  # Loop back unconditionally
```

**Pattern 2: Conditional Only**
Use when loop should continue indefinitely until a condition is met (e.g., user cancellation, external event).

```yaml
# Example: Ongoing subscription until cancellation
activities:
  - key: process_subscription
    iteration_scoped: true
    # No iteration_limit - runs until canceled
    parameters:
      subscriber_id: "{{INPUT.subscriber_id}}"
    depends_on:
      - check_subscription_status
      - activity_key: check_subscription_status
        conditions:
          - "{{check_subscription_status.active | last == true}}"  # Continue while active

  - key: check_subscription_status
    iteration_scoped: true
    parameters:
      subscriber_id: "{{INPUT.subscriber_id}}"
    depends_on:
      - process_subscription
```

**Pattern 3: Bounded Conditional (Recommended for Production)**
Use both iteration_limit and conditions for maximum safety. Provides a condition-based exit with a safety bound.

```yaml
# Example: Research loop with quality check and safety limit
activities:
  - key: perform_search
    iteration_scoped: true
    iteration_limit: 10          # Safety bound: max 10 iterations
    parameters:
      query: "{{INPUT.topic}}"
      context: "{{perform_search.results}}"
    depends_on:
      - initialize
      - activity_key: evaluate_results
        conditions:
          - "{{evaluate_results.sufficient | last == false}}"  # Continue if not sufficient

  - key: evaluate_results
    iteration_scoped: true
    parameters:
      findings: "{{perform_search.results}}"
    outputs:
      - name: sufficient
    depends_on:
      - perform_search
```

**Validation Rules**:
- ✅ Loop must have: (condition) OR (iteration_limit) OR (both)
- ❌ Loop with neither will fail validation
- ⚠️ Pattern 3 (both) recommended for production use

---

### 3. 🔲 Loop Detection in Workflow Validation

**File**: `core/src/workflow/definition.rs`

**When Validation Occurs**:
- ✅ **Once** when workflow definition is registered via `POST /api/v1/workflow_definitions`
- ✅ Computed metadata (`is_loop_activity`, `is_back_edge`) stored in database
- ❌ **NOT** re-validated when loading definition from database during orchestration
- 📌 **Workflow definitions are immutable** - changes create new definitions with new IDs
  - No migration needed for existing workflows
  - Each workflow instance references a specific immutable definition
  - Validation guarantees remain valid for the lifetime of the definition

**Code Path for Validation** (for reference):
```rust
// API Handler (api/src/handlers/workflow_definitions.rs)
async fn create_workflow_definition(
    request: CreateWorkflowDefinitionRequest,
) -> Result<WorkflowDefinitionResponse> {
    let mut definition = parse_yaml(&request.yaml)?;

    // VALIDATION HAPPENS HERE (once, at registration)
    definition.validate()?;  // ← Computes and caches all metadata

    // Store in database with computed metadata
    let stored = repository.store(definition).await?;  // ← is_loop_activity, is_back_edge saved to DB

    Ok(stored)
}

// Orchestrator (core/src/orchestrator/orchestrator.rs)
async fn handle_activity_completed(...) -> Result<()> {
    // NO VALIDATION HERE - just loads pre-validated definition
    let definition = load_workflow_definition(tx, workflow_id).await?;  // ← Metadata already cached

    // Use cached metadata (O(1) lookups)
    if activity_def.is_loop_activity { ... }
    if dep.is_back_edge { ... }
}
```

**Changes Needed**:

Modify validation to distinguish between circular dependencies (invalid) and loops (valid with back-edges), and **cache all computed metadata** for orchestrator performance:

```rust
impl WorkflowDefinition {
    /// Validate workflow structure and compute loop metadata
    pub fn validate(&mut self) -> Result<(), ValidationErrors> {
        let mut errors = ValidationErrors::new();

        // ... existing validation ...

        // Detect loops (back-edges) and mark activities
        let loops = self.detect_loops()?;

        // Mark activities and dependencies that participate in loops (cache for orchestrator hot path)
        for loop_edge in &loops {
            // Mark both activities in the loop
            if let Some(activity) = self.activities.iter_mut().find(|a| a.key == loop_edge.from) {
                activity.is_loop_activity = true;

                // Mark the specific dependency that is the back-edge
                if let Some(depends_on) = &mut activity.depends_on {
                    for dep in depends_on {
                        if dep.activity_key == loop_edge.to {
                            dep.is_back_edge = true;
                        }
                    }
                }
            }
            if let Some(activity) = self.activities.iter_mut().find(|a| a.key == loop_edge.to) {
                activity.is_loop_activity = true;
            }
        }

        // Validate that loops have proper configuration
        for loop_edge in loops {
            self.validate_loop_edge(&loop_edge, &mut errors)?;
        }

        // ... rest of validation ...
    }

    /// Detect back-edges (loops) in the workflow graph
    fn detect_loops(&self) -> Result<Vec<LoopEdge>, ValidationErrors> {
        let graph = self.build_dependency_graph();
        let mut loops = Vec::new();

        // Perform topological sort to identify back-edges
        let sorted = topological_sort(&graph)?;

        // Any edge from later to earlier in topo order is a back-edge
        for (idx, activity) in sorted.iter().enumerate() {
            if let Some(depends_on) = &activity.depends_on {
                for dep in depends_on {
                    let dep_idx = sorted.iter().position(|a| a.key == dep.activity_key);
                    if let Some(dep_idx) = dep_idx {
                        if dep_idx > idx {
                            // Back-edge found (loop)
                            loops.push(LoopEdge {
                                from: activity.key.clone(),
                                to: dep.activity_key.clone(),
                                condition: dep.conditions.clone(),
                            });
                        }
                    }
                }
            }
        }

        Ok(loops)
    }

    /// Validate that a loop edge has proper configuration
    fn validate_loop_edge(&self, loop_edge: &LoopEdge, errors: &mut ValidationErrors) -> Result<()> {
        let from_activity = self.get_activity(&loop_edge.from)?;
        let to_activity = self.get_activity(&loop_edge.to)?;

        // Check if loop has exit condition
        let has_condition = loop_edge.condition.as_ref()
            .map(|c| !c.is_empty())
            .unwrap_or(false);

        // Check if loop has iteration limit (check both activity-level and settings-level)
        let has_iteration_limit = from_activity.iteration_limit.is_some()
            || to_activity.iteration_limit.is_some()
            || from_activity.settings.as_ref().and_then(|s| s.iteration_limit).is_some()
            || to_activity.settings.as_ref().and_then(|s| s.iteration_limit).is_some();

        // Loop MUST have at least one exit mechanism: condition OR iteration_limit OR both
        if !has_condition && !has_iteration_limit {
            errors.add(
                "activities",
                &format!(
                    "Loop from '{}' to '{}' must have at least one exit mechanism:\n\
                    - Exit condition: conditions: [\"{{{{evaluate.done == true}}}}\"]\n\
                    - Iteration limit: iteration_limit: 10\n\
                    - Or both (recommended for production)\n\n\
                    Examples:\n\
                    1) Fixed iterations: iteration_limit: 12\n\
                    2) Conditional: conditions: [\"{{{{status.canceled == false}}}}\"]\n\
                    3) Bounded conditional: iteration_limit: 10 AND conditions",
                    loop_edge.from, loop_edge.to
                )
            );
        }

        // Recommend iteration_scoped for loop activities
        if !from_activity.iteration_scoped && !to_activity.iteration_scoped {
            // Just a warning in logs, not a hard error
            // Note: Iteration counter is still tracked for iteration_limit enforcement,
            // but outputs are not stored as arrays (only latest value retained)
            tracing::warn!(
                "Loop from '{}' to '{}' does not use iteration_scoped. \
                Iteration limits will still be enforced, but only the latest outputs will be available. \
                Consider setting iteration_scoped: true to track results per iteration.",
                loop_edge.from, loop_edge.to
            );
        }

        Ok(())
    }
}

#[derive(Debug, Clone)]
struct LoopEdge {
    from: String,
    to: String,
    condition: Option<Vec<String>>,
}
```

**Key Changes**:
1. Existing cycle detection becomes loop detection
2. Loops are allowed if they have proper configuration (condition, iteration_limit)
3. Validation ensures loop safety
4. **All computed metadata persisted to database** - no re-validation on load

**What Gets Cached** (MVP):
- ✅ `is_loop_activity` on each `ActivityDefinition`
- ✅ `is_back_edge` on each `DependencyRelationship`

**Future Optimization Opportunities** (Post-MVP):

1. **Topological Order** (`execution_order: Vec<String>` on `WorkflowDefinition`)
   - **What**: Pre-sorted list of activity keys in optimal execution order
   - **Current cost**: O(V+E) topological sort on every evaluation cycle to find ready activities
   - **Benefit**: O(1) lookup for "what comes next" - iterate through pre-sorted list instead of scanning all activities
   - **Use case**: Start-to-finish workflows, reduce scheduler overhead by 50-70%

2. **Dependency Depth** (`depth: u32` on `ActivityDefinition`)
   - **What**: Number of activities in the longest path from start to this activity
   - **Current cost**: Calculated ad-hoc if needed, or not available
   - **Benefits**:
     - **Priority scheduling**: Schedule deeper activities first (critical path optimization)
     - **Progress tracking**: Show workflow is "60% complete by depth"
     - **Latency prediction**: Estimate time to completion based on depth and average activity duration
     - **Resource allocation**: Prioritize workers for critical path activities
   - **Use case**: Workflows with mixed critical/non-critical paths, SLA-sensitive workloads

3. **Parallel Execution Groups** (`parallel_group: u32` on `ActivityDefinition`)
   - **What**: Group ID for activities that can execute in parallel (no dependencies between group members)
   - **Current cost**: Discovered dynamically by checking each activity's readiness individually
   - **Benefits**:
     - **Batch scheduling**: Schedule entire groups (5-10 activities) in single transaction vs 5-10 individual schedules
     - **Reduced orchestration overhead**: One evaluation cycle per group instead of per activity
     - **Worker allocation**: Reserve workers for entire group upfront
     - **Visualization**: Color-code parallel groups in workflow diagrams
     - **Cost estimation**: Calculate parallel cost (max of group) vs sequential cost (sum of group)
   - **Example**: Document processing workflow with 10 parallel file uploads - schedule all 10 at once instead of 10 separate evaluations
   - **Use case**: High-parallelism workflows (e.g., fan-out patterns with 100+ parallel activities)

4. **Critical Path** (`on_critical_path: bool` on `ActivityDefinition`)
   - **What**: Whether this activity is on the longest path through the workflow (determines minimum completion time)
   - **Benefits**:
     - **Priority scheduling**: Critical path activities get fastest workers
     - **Resource reservation**: Guarantee resources for critical path to minimize latency
     - **SLA optimization**: Focus optimization efforts on critical path activities
     - **Cost/speed tradeoff**: Use expensive/fast models for critical path, cheap/slow for non-critical
   - **Use case**: Time-sensitive workflows, SLA guarantees

**Example Workflow Analysis** (computed once at validation):

```yaml
# User writes YAML (no metadata):
activities:
  - key: fetch_data
    # ...no metadata fields...

  - key: process_1
    depends_on: [fetch_data]

  - key: process_2
    depends_on: [fetch_data]

  - key: aggregate
    depends_on: [process_1, process_2]
```

```rust
// Stored in database after validation (metadata added):
ActivityDefinition {
    key: "fetch_data",
    // User-specified fields...

    // COMPUTED METADATA (added by validation):
    is_loop_activity: false,
    depth: 0,                    // Start activity
    parallel_group: 0,
    on_critical_path: true,      // On critical path
}

ActivityDefinition {
    key: "process_1",
    depends_on: [
        DependencyRelationship {
            activity_key: "fetch_data",
            is_back_edge: false,  // Forward edge
        }
    ],

    // COMPUTED METADATA:
    is_loop_activity: false,
    depth: 1,                     // 1 activity deep
    parallel_group: 1,            // Parallel with process_2
    on_critical_path: true,       // Assuming process_1 is slower
}

ActivityDefinition {
    key: "process_2",
    depends_on: [
        DependencyRelationship {
            activity_key: "fetch_data",
            is_back_edge: false,
        }
    ],

    // COMPUTED METADATA:
    is_loop_activity: false,
    depth: 1,
    parallel_group: 1,            // Same group as process_1
    on_critical_path: false,      // Not on critical path
}

ActivityDefinition {
    key: "aggregate",
    depends_on: [/* ... */],

    // COMPUTED METADATA:
    is_loop_activity: false,
    depth: 2,                     // 2 activities deep
    parallel_group: 2,
    on_critical_path: true,       // Final activity
}
```

**Orchestrator uses metadata**:
- Schedule `parallel_group: 1` activities (`process_1`, `process_2`) in **one batch**
- Prioritize `on_critical_path: true` activities for fastest workers
- Track progress: "50% complete (depth 1 of 2)"
- No graph traversal needed - just check metadata flags

**Serialization Strategy**:
- Use `#[serde(default, skip_serializing_if = "is_false")]` for boolean metadata
- This allows:
  - Reading from YAML without the field (defaults to false)
  - Ignoring the field if user accidentally includes it in YAML
  - Storing computed value in database
  - Clean YAML output (metadata only shown when true)

---

### 4. 🔲 Update Dependency Evaluator for Loop Scheduling

**File**: `core/src/orchestrator/dependency_evaluator.rs`

**Changes Needed**:

Modify dependency evaluation to handle back-edges and iteration limits:

```rust
impl DependencyEvaluator {
    /// Check if activity is ready to execute (handles loops)
    pub fn is_activity_ready(
        &self,
        activity: &ActivityDefinition,
        state: &WorkflowState,
    ) -> Result<bool> {
        let activity_state = state.activities.get(&activity.key);

        // Check if already completed (but allow re-execution for loops)
        if let Some(state) = activity_state {
            match state.status {
                WorkflowActivityStatus::Completed => {
                    // Check if this is a loop back-edge scenario
                    if !self.should_loop_back(activity, state, state)? {
                        return Ok(false); // Already completed, don't re-execute
                    }
                    // Fall through to check loop conditions
                }
                WorkflowActivityStatus::Running | WorkflowActivityStatus::Pending => {
                    return Ok(false); // Already scheduled/running
                }
                WorkflowActivityStatus::Failed | WorkflowActivityStatus::Skipped => {
                    return Ok(false); // Terminal states
                }
                WorkflowActivityStatus::NotScheduled => {
                    // Continue to dependency check
                }
            }
        }

        // Check iteration limit
        if let Some(activity_state) = activity_state {
            if self.is_max_iterations_exceeded(activity, activity_state)? {
                return Ok(false);
            }
        }

        // Standard dependency check
        if let Some(depends_on) = &activity.depends_on {
            for dep in depends_on {
                let dep_state = state.activities.get(&dep.activity_key);

                // Check if this is a back-edge (loop) - uses precomputed metadata
                if dep.is_back_edge {
                    // For back-edges, evaluate loop condition
                    if !self.evaluate_loop_condition(dep, dep_state, state)? {
                        return Ok(false); // Loop condition not met
                    }
                } else {
                    // Standard forward dependency check
                    if !self.is_dependency_satisfied(dep, dep_state, state)? {
                        return Ok(false);
                    }
                }
            }
        }

        Ok(true)
    }

    /// Check if activity should loop back (re-execute)
    fn should_loop_back(
        &self,
        activity: &ActivityDefinition,
        activity_state: &ActivityState,
        state: &WorkflowState,
    ) -> Result<bool> {
        // Only loop if status is Completed and there's a back-edge dependency
        if activity_state.status != WorkflowActivityStatus::Completed {
            return Ok(false);
        }

        // Check if any downstream activity triggers a loop back
        // (This is detected when evaluating the depends_on of this activity)
        Ok(false) // Default: don't loop
    }

    /// Check if max iterations has been exceeded
    fn is_max_iterations_exceeded(
        &self,
        activity: &ActivityDefinition,
        activity_state: &ActivityState,
    ) -> Result<bool> {
        let iteration_limit = activity.iteration_limit
            .or_else(|| activity.settings.as_ref()?.iteration_limit)
            .unwrap_or(100); // Default limit

        Ok(activity_state.iteration >= iteration_limit)
    }

    // NOTE: is_back_edge() method removed - now using precomputed dep.is_back_edge flag

    /// Evaluate loop exit/continuation condition
    fn evaluate_loop_condition(
        &self,
        dep: &DependencyRelationship,
        dep_state: Option<&ActivityState>,
        state: &WorkflowState,
    ) -> Result<bool> {
        // Loop conditions should evaluate to true to continue looping
        // Exit conditions should evaluate to false to stop looping

        if let Some(conditions) = &dep.conditions {
            for condition in conditions {
                if !self.evaluate_condition(condition, dep_state, state)? {
                    return Ok(false); // Condition not met, don't loop
                }
            }
        }

        Ok(true) // All conditions met, continue loop
    }
}
```

**Key Logic**:
1. Use precomputed `dep.is_back_edge` flag (computed during validation, stored in database)
2. Evaluate loop conditions separately from forward dependencies
3. Enforce iteration limits
4. Allow re-execution of completed activities when loop conditions are met

**Performance Optimization**:
- ✅ Back-edge detection is O(1) lookup (was O(V+E) with old is_back_edge() method)
- ✅ No graph traversal in hot path

---

### 5. 🔲 Update Orchestrator for Iteration Management

**File**: `core/src/orchestrator/orchestrator.rs`

**Changes Needed**:

Handle activity completion differently for iteration-scoped activities:

```rust
impl Orchestrator {
    /// Handle ActivityCompleted event (with iteration support)
    async fn handle_activity_completed(
        &self,
        tx: &mut PgConnection,
        event: &WorkflowEvent,
        activity_key: &str,
        outputs: &[ActivityOutput],
        cost_usd: Decimal,
    ) -> Result<()> {
        let mut state = load_materialized_state(tx, event.workflow_id).await?;

        // Load pre-validated workflow definition from database
        // NOTE: Definition was validated once at registration (POST /api/v1/workflow_definitions)
        // All computed metadata (is_loop_activity, is_back_edge) already cached in DB
        // NO validation happens here - orchestrator uses pre-analyzed definition
        let definition = load_workflow_definition(tx, event.workflow_id).await?;

        let activity_def = definition.get_activity(activity_key)?;

        // Get or create activity state
        let activity_state = state.activities.entry(activity_key.to_string())
            .or_insert_with(|| ActivityState {
                key: activity_key.to_string(),
                status: WorkflowActivityStatus::NotScheduled,
                outputs: None,
                error: None,
                started_at: None,
                completed_at: None,
                attempt: 1,
                last_error: None,
                accumulated_cost_usd: Decimal::ZERO,
                iteration: 0,
                iteration_outputs: if activity_def.iteration_scoped {
                    Some(HashMap::new())
                } else {
                    None
                },
            });

        // Check if this activity is part of a loop (metadata computed during validation)
        if activity_def.is_loop_activity {
            // Increment iteration counter for ALL looping activities
            activity_state.increment_iteration();

            // Archive outputs only for iteration_scoped activities
            if activity_def.iteration_scoped {
                activity_state.archive_iteration_outputs(outputs.to_vec());
            }
        }

        // Update status and outputs
        activity_state.status = WorkflowActivityStatus::Completed;
        activity_state.completed_at = Some(Utc::now());
        activity_state.outputs = Some(outputs.to_vec());

        // Add cost to accumulated total
        activity_state.add_cost(cost_usd);

        // Save updated state
        save_materialized_state(tx, event.workflow_id, &state).await?;

        // Re-evaluate workflow to find next ready activities (may include loop-back)
        self.evaluate_and_schedule_ready_activities(tx, event.workflow_id, &state, &definition).await?;

        Ok(())
    }

    /// Schedule activity for execution (with iteration support)
    async fn schedule_activity(
        &self,
        tx: &mut PgConnection,
        workflow_id: Uuid,
        activity_def: &ActivityDefinition,
        state: &mut WorkflowState,
    ) -> Result<()> {
        let activity_state = state.activities.entry(activity_def.key.clone())
            .or_insert_with(|| ActivityState {
                key: activity_def.key.clone(),
                status: WorkflowActivityStatus::NotScheduled,
                outputs: None,
                error: None,
                started_at: None,
                completed_at: None,
                attempt: 1,
                last_error: None,
                accumulated_cost_usd: Decimal::ZERO,
                iteration: 0,
                iteration_outputs: if activity_def.iteration_scoped {
                    Some(HashMap::new())
                } else {
                    None
                },
            });

        // Check if this is a loop-back (re-execution)
        let is_loop_back = activity_state.status == WorkflowActivityStatus::Completed;

        if is_loop_back {
            // Reset status for next iteration (outputs are preserved in iteration_outputs)
            activity_state.status = WorkflowActivityStatus::Pending;
            activity_state.started_at = None;
            activity_state.completed_at = None;
            // iteration counter already incremented in handle_activity_completed
        } else {
            // First execution
            activity_state.status = WorkflowActivityStatus::Pending;
        }

        // Schedule to queue
        let resolved_params = self.template_resolver.resolve_activity_parameters(
            activity_def,
            &definition,
            state,
        )?;

        self.activity_queue.schedule(
            workflow_id,
            vec![QueueActivity {
                key: activity_def.key.clone(),
                worker: activity_def.worker.clone(),
                activity_name: activity_def.activity_name.clone(),
                parameters: resolved_params,
            }]
        ).await?;

        Ok(())
    }
}
```

**Key Changes**:
1. **Loop detection moved to validation time** - `is_loop_activity` computed once, stored in DB, never recomputed
2. **Orchestrator uses cached metadata** - simple boolean check `activity_def.is_loop_activity`
3. Iteration counter incremented for ALL activities with loop back-edges (not just iteration_scoped)
4. Outputs archived only for iteration_scoped activities
5. Separated iteration counting from output archiving
6. Allow re-scheduling of completed activities for loop-back
7. Preserve iteration state across loop cycles

**Performance Benefits**:
- Loop detection: O(1) lookup instead of O(V+E) graph traversal on every activity completion
- No validation overhead in orchestrator (validation happened once at registration)
- Workflow definitions loaded from DB already contain all computed metadata

---

### 6. 🔲 Extend Template Resolver for Iteration Access

**File**: `core/src/orchestrator/template_resolver.rs`

**Changes Needed**:

Serialize iteration-scoped outputs as arrays, allowing MiniJinja to handle array operations:

```rust
impl TemplateResolver {
    /// Resolve activity parameters with template substitution (iteration support)
    pub fn resolve_activity_parameters(
        &self,
        activity: &ActivityDefinition,
        definition: &WorkflowDefinition,
        state: &WorkflowState,
    ) -> Result<HashMap<String, Value>> {
        let mut resolved = HashMap::new();

        if let Some(params) = &activity.parameters {
            // Build template context
            let mut context = serde_json::Map::new();

            // Add INPUT variables
            context.insert("INPUT".to_string(), state.state_data.clone());

            // Add activity outputs (with iteration support)
            for (activity_key, activity_state) in &state.activities {
                context.insert(
                    activity_key.clone(),
                    self.serialize_activity_outputs(activity_state)?
                );
            }

            // Add ACTIVITY context (iteration, cost tracking)
            let activity_state = state.activities.get(&activity.key);
            context.insert("ACTIVITY".to_string(), json!({
                "iteration": activity_state.map(|s| s.iteration).unwrap_or(0),
                "accumulated_cost_usd": activity_state
                    .map(|s| s.accumulated_cost_usd.to_string())
                    .unwrap_or_else(|| "0.00".to_string()),
                "remaining_budget_usd": self.calculate_remaining_budget(activity, activity_state),
            }));

            // NOTE: For iteration-scoped activities, accumulated_cost_usd includes costs from ALL iterations
            // Budget limits are enforced against the total, not per-iteration

            // Resolve each parameter
            for (key, value) in params {
                let resolved_value = self.resolve_value(value, &context)?;
                resolved.insert(key.clone(), resolved_value);
            }
        }

        Ok(resolved)
    }

    /// Serialize activity outputs (iteration-scoped outputs as arrays)
    fn serialize_activity_outputs(&self, activity_state: &ActivityState) -> Result<Value> {
        // If iteration_outputs exists, return them directly (already grouped by name as arrays)
        if let Some(iteration_outputs) = &activity_state.iteration_outputs {
            // iteration_outputs is already HashMap<String, Vec<Value>>
            // Just convert to serde_json::Value
            Ok(serde_json::to_value(iteration_outputs)?)
        } else {
            // Non-iteration-scoped: standard output serialization (single values)
            let mut result = serde_json::Map::new();
            if let Some(outputs) = &activity_state.outputs {
                for output in outputs {
                    result.insert(output.name.clone(), output.value.clone());
                }
            }
            Ok(Value::Object(result))
        }
    }

    /// Calculate remaining budget for activity
    fn calculate_remaining_budget(
        &self,
        activity: &ActivityDefinition,
        activity_state: Option<&ActivityState>,
    ) -> String {
        if let Some(settings) = &activity.settings {
            if let Some(budget) = &settings.budget {
                let limit = budget.limit;
                let accumulated = activity_state
                    .map(|s| s.accumulated_cost_usd)
                    .unwrap_or(Decimal::ZERO);
                let remaining = limit - accumulated;
                return remaining.max(Decimal::ZERO).to_string();
            }
        }
        "0.00".to_string()
    }
}
```

**Template Syntax Support** (using MiniJinja array filters):

```yaml
# Access all iterations as array (iteration-scoped activities)
"{{perform_search.results}}"              # ["result1", "result2", "result3"]

# Access latest iteration value (use MiniJinja | last filter)
"{{perform_search.results | last}}"       # "result3"

# Access first iteration value
"{{perform_search.results | first}}"      # "result1"

# Get number of iterations
"{{perform_search.results | length}}"     # 3

# Access current iteration counter
"{{ACTIVITY.iteration}}"                  # 2

# Access accumulated cost (across all iterations)
"{{ACTIVITY.accumulated_cost_usd}}"       # "7.50"

# Access remaining budget (available but not typically used in loop conditions)
"{{ACTIVITY.remaining_budget_usd}}"       # "2.50"

# Check latest iteration result in condition (typical loop pattern)
conditions:
  - "{{evaluate_results.sufficient | last == false}}"
```

**Key Simplification**:
- Iteration-scoped activities store outputs grouped by name: `{ "output_name": [value0, value1, ...] }`
- This matches template access patterns: `{{activity.output_name}}` returns the array directly
- Users apply standard MiniJinja filters (`| last`, `| first`, `| length`, etc.) to access specific values
- No flattening or transformation needed - data is stored ready for template use

---

### 7. 🔲 Add Iteration Limit Enforcement

**File**: `core/src/orchestrator/dependency_evaluator.rs`

**Changes Needed** (Already covered in Task 4, but highlighting here):

```rust
/// Default maximum iterations if not specified
/// Provides safety bound for Pattern 2 (conditional-only) loops
const DEFAULT_MAX_ITERATIONS: u32 = 100;

impl DependencyEvaluator {
    /// Check if max iterations has been exceeded
    fn is_max_iterations_exceeded(
        &self,
        activity: &ActivityDefinition,
        activity_state: &ActivityState,
    ) -> Result<bool> {
        // Check activity-level iteration_limit
        let iteration_limit = activity.iteration_limit
            .or_else(|| {
                // Check settings-level iteration_limit
                activity.settings.as_ref()
                    .and_then(|s| s.iteration_limit)
            })
            .unwrap_or(DEFAULT_MAX_ITERATIONS);  // Safety bound for conditional-only loops

        // Note: To "disable" the limit for a specific activity, set iteration_limit
        // to a very high value like 999999 in the YAML. The default limit is
        // intentionally always applied as a safety mechanism.

        if activity_state.iteration >= iteration_limit {
            tracing::warn!(
                "Activity '{}' exceeded iteration_limit: {} >= {}",
                activity.key,
                activity_state.iteration,
                iteration_limit
            );
            return Ok(true);
        }

        Ok(false)
    }
}
```

**Configuration Examples**:

```yaml
# Example 1: Fixed iterations (Pattern 1)
activities:
  - key: send_newsletter
    iteration_scoped: true
    iteration_limit: 12  # Exactly 12 issues

# Example 2: Conditional with default safety bound (Pattern 2)
# Default limit of 100 applies automatically
activities:
  - key: poll_until_ready
    iteration_scoped: true
    # No iteration_limit specified → uses DEFAULT_MAX_ITERATIONS (100)
    depends_on:
      - activity_key: check_status
        conditions:
          - "{{check_status.ready | last == false}}"

# Example 3: Long-running subscription with high limit
activities:
  - key: monthly_subscription
    iteration_scoped: true
    iteration_limit: 999999  # Effectively "unlimited" but still bounded
    depends_on:
      - activity_key: check_subscription
        conditions:
          - "{{check_subscription.active | last == true}}"

# Example 4: Custom default via environment variable
# STREAMFLOW_DEFAULT_MAX_ITERATIONS=1000
# This changes the default for all Pattern 2 loops
```

**Note on "Disabling" the Limit**:
- There is no way to completely disable the iteration limit (by design - safety mechanism)
- To effectively disable it, set `iteration_limit: 999999` (or another very high value)
- For Pattern 2 (conditional-only), the default limit of 100 is intentionally conservative
- If you need more than 100 iterations, explicitly set `iteration_limit` to an appropriate value for your use case

---

## Testing Strategy

### Unit Tests

**File**: `core/src/workflow/definition.rs` (New tests)

1. **Loop Detection Tests**:
   - `test_detect_simple_loop` - Two activities with back-edge
   - `test_detect_multi_activity_loop` - Loop involving 3+ activities
   - `test_no_loop_in_sequential_workflow` - Verify sequential workflows don't trigger loop detection
   - `test_parallel_branches_not_loops` - Parallel execution should not be detected as loops

2. **Loop Validation Tests**:
   - `test_loop_requires_exit_mechanism` - Loop with neither condition nor iteration_limit should fail validation
   - `test_loop_with_iteration_limit_only` - Loop with only iteration_limit should pass (Pattern 1)
   - `test_loop_with_condition_only` - Loop with only condition should pass (Pattern 2)
   - `test_loop_with_both` - Loop with both condition and iteration_limit should pass (Pattern 3)

**File**: `core/src/orchestrator/workflow_state.rs` (New tests)

3. **Iteration State Tests**:
   - `test_increment_iteration` - Verify iteration counter increments and outputs grouped by name
   - `test_get_latest_output_value` - Access latest value for a specific output
   - `test_get_output_values` - Access all values for a specific output across iterations
   - `test_non_iteration_scoped_state` - Non-iteration activities don't create iteration_outputs

**File**: `core/src/orchestrator/template_resolver.rs` (New tests)

4. **Template Resolution Tests**:
   - `test_resolve_iteration_scoped_as_array` - `{{activity.output}}` returns array for iteration-scoped
   - `test_resolve_latest_with_filter` - `{{activity.output | last}}` returns latest value using MiniJinja filter
   - `test_resolve_iteration_counter` - `{{ACTIVITY.iteration}}` returns current iteration
   - `test_resolve_remaining_budget` - Budget calculation across iterations

### Integration Tests

**File**: `core/tests/orchestrator_loop_tests.rs` (New file)

5. **Simple Loop Test**:
   ```rust
   #[tokio::test]
   async fn test_simple_loop_workflow() {
       // Workflow: search -> evaluate -> (loop back if not sufficient)
       // - search: iteration_scoped, outputs "results"
       // - evaluate: iteration_scoped, outputs "sufficient" (bool)
       // - Loop condition: sufficient == false
       // - Iteration limit: 3

       // Expected behavior:
       // - Iteration 0: search -> evaluate (sufficient=false) -> loop back
       // - Iteration 1: search -> evaluate (sufficient=false) -> loop back
       // - Iteration 2: search -> evaluate (sufficient=true) -> exit loop
       // - Workflow completes with 3 iterations of search, 3 of evaluate
   }
   ```

6. **Iteration Limit Test**:
   ```rust
   #[tokio::test]
   async fn test_loop_max_iterations_enforced() {
       // Loop that never exits (condition always true)
       // Max iterations: 5
       // Expected: Loop stops after 5 iterations
   }
   ```

7. **Budget Accumulation Across Iterations Test**:
   ```rust
   #[tokio::test]
   async fn test_iteration_budget_accumulation() {
       // Iteration-scoped activity with budget limit: $10
       // Each iteration costs $3.50
       // Expected behavior:
       // - Iteration 0: $3.50 accumulated (passes)
       // - Iteration 1: $7.00 accumulated (passes)
       // - Iteration 2: $10.50 would exceed budget -> activity fails with budget exceeded
       // Verify:
       // - accumulated_cost_usd tracks total across ALL iterations
       // - Budget limit applies to sum, not per-iteration
       // - Activity fails when total would exceed limit (action: abort)
   }
   ```

8. **Iteration Array Access Test**:
   ```rust
   #[tokio::test]
   async fn test_iteration_array_access_in_template() {
       // Activity parameters reference {{search.results}} (returns array)
       // Verify array contains all iteration results
       // Test MiniJinja filters: {{search.results | last}}, {{search.results | length}}
   }
   ```

### End-to-End Tests

**File**: `examples/06-research-agent.yaml` (New example)

9. **Research Agent Workflow**:
   ```yaml
   name: agentic_research
   description: "Research assistant that iteratively searches and evaluates until sufficient information gathered"

   activities:
     - key: initialize
       worker: builtin
       activity_name: llm_call
       parameters:
         provider: anthropic
         model: claude-sonnet-4-5-20250929
         prompt: |
           Create a research plan for the topic: {{INPUT.topic}}

           Generate:
           1. A focused search strategy (what to look for)
           2. 3-5 key questions to answer
           3. Success criteria (what makes research "sufficient")

           Return JSON: {"strategy": "...", "questions": [...], "criteria": "..."}
         max_tokens: 500
       outputs:
         - name: results
         - name: cost_usd

     - key: perform_search
       worker: builtin
       activity_name: llm_call
       iteration_scoped: true
       iteration_limit: 5
       parameters:
         provider: anthropic
         model: claude-sonnet-4-5-20250929
         prompt: |
           Research topic: {{INPUT.topic}}
           Research plan: {{initialize.results.content}}

           Previous search results: {{perform_search.results}}

           Conduct research and return findings as JSON: {"findings": "...", "sources_found": [...]}
         max_tokens: 1000
       outputs:
         - name: results
         - name: cost_usd
       depends_on:
         - initialize
         - activity_key: evaluate_results
           conditions:
             # MVP limitation: Can't easily parse JSON fields from results.content
             # Workaround: Have LLM return simple "CONTINUE" or "SUFFICIENT" string
             # Future: Output field extraction (see enhancement note at end)
             - "{{evaluate_results.results | last | get(key='content') | contains(substring='CONTINUE')}}"

     - key: evaluate_results
       worker: builtin
       activity_name: llm_call
       iteration_scoped: true
       parameters:
         provider: anthropic
         model: claude-haiku-4-20250415
         prompt: |
           Topic: {{INPUT.topic}}
           Success criteria from plan: {{initialize.results.content}}
           All findings so far: {{perform_search.results}}
           Iteration: {{ACTIVITY.iteration}}

           Evaluate if we have sufficient information to write a comprehensive report.

           Respond with ONLY one word:
           - CONTINUE (if more research needed)
           - SUFFICIENT (if ready to compile report)
         max_tokens: 10
       outputs:
         - name: results
         - name: cost_usd
       depends_on:
         - perform_search

     - key: compile_report
       worker: builtin
       activity_name: llm_call
       parameters:
         provider: anthropic
         model: claude-sonnet-4-5-20250929
         prompt: |
           Topic: {{INPUT.topic}}
           Research plan: {{initialize.results.content}}
           All findings: {{perform_search.results}}

           Compile a comprehensive research report synthesizing all findings.
         max_tokens: 2000
       outputs:
         - name: results
         - name: cost_usd
       depends_on:
         - activity_key: evaluate_results
           conditions:
             - "{{evaluate_results.results | last | get(key='content') | contains(substring='SUFFICIENT')}}"
   ```

---

## Implementation Phases

### Phase 1: Data Model & Validation (Day 1-2)

**Focus**: Extend data structures and implement "validate once, cache forever" pattern

1. **Extend `ActivityState`** (workflow_state.rs):
   - Add `iteration: u32` field (tracked for ALL loop activities)
   - Add `iteration_outputs: Option<HashMap<String, Vec<Value>>>` (only for iteration_scoped)
   - Separate `increment_iteration()` from `archive_iteration_outputs()` methods
   - Unit tests for iteration state management

2. **Extend `ActivityDefinition`** (definition.rs):
   - Add `iteration_scoped: bool` field (user-specified)
   - Add `iteration_limit: Option<u32>` field (user-specified)
   - Add `is_loop_activity: bool` field (computed, cached)
   - Use `#[serde(default, skip_serializing_if = "is_false")]` pattern

3. **Add `DependencyRelationship` struct** (definition.rs):
   - Define struct with `activity_key`, `conditions`, `is_back_edge` fields
   - Add `is_back_edge: bool` field (computed, cached)
   - Use `#[serde(default, skip_serializing_if = "is_false")]` pattern

4. **Implement loop detection in validation** (definition.rs):
   - Detect back-edges via topological sort
   - Mark `is_loop_activity = true` on participating activities
   - Mark `is_back_edge = true` on back-edge dependencies
   - **Validate loop safety**: require (condition) OR (iteration_limit) OR (both)
   - Store all computed metadata in database (no re-validation)

5. **Unit tests for validation**:
   - `test_loop_requires_exit_mechanism` - neither → fail
   - `test_loop_with_iteration_limit_only` - Pattern 1 → pass
   - `test_loop_with_condition_only` - Pattern 2 → pass
   - `test_loop_with_both` - Pattern 3 → pass
   - `test_metadata_cached_in_database` - metadata persisted

### Phase 2: Orchestration Logic (Day 2-3)

**Focus**: Use cached metadata in hot path (no graph traversal)

1. **Update `DependencyEvaluator`** (dependency_evaluator.rs):
   - Use `dep.is_back_edge` flag instead of computing (O(1) vs O(V+E))
   - Remove `is_back_edge()` method (no longer needed)
   - Keep iteration limit enforcement logic
   - Integration tests for back-edge detection performance

2. **Update `Orchestrator`** (orchestrator.rs):
   - Check `activity_def.is_loop_activity` instead of computing (O(1) vs O(V+E))
   - Increment iteration counter for ALL loop activities (not just iteration_scoped)
   - Archive outputs only for iteration_scoped activities
   - Remove graph traversal helper methods
   - Integration tests for loop iteration management

3. **Iteration limit enforcement**:
   - DEFAULT_MAX_ITERATIONS = 100 (safety bound for Pattern 2)
   - Check activity-level, settings-level, then default
   - Integration tests for limit enforcement

4. **Integration tests for loop scheduling**:
   - Test all three patterns (iteration_limit only, condition only, both)
   - Test iteration counter increments for non-iteration_scoped loops
   - Test metadata is loaded from database (not recomputed)

### Phase 3: Template Resolution (Day 3-4)

**Focus**: Support iteration arrays and ACTIVITY context

1. **Extend `TemplateResolver`** (template_resolver.rs):
   - Serialize iteration-scoped outputs as arrays grouped by name
   - Serialize non-iteration-scoped outputs as single values
   - No transformation needed (data stored ready for templates)
   - Unit tests for array vs single-value serialization

2. **Add `ACTIVITY` context variables**:
   - `{{ACTIVITY.iteration}}` - current iteration number
   - `{{ACTIVITY.accumulated_cost_usd}}` - total cost across iterations
   - `{{ACTIVITY.remaining_budget_usd}}` - budget remaining
   - Unit tests for context variable resolution

3. **Template syntax support**:
   - Test MiniJinja filters on arrays (`| last`, `| first`, `| length`)
   - Test accessing all iterations: `{{activity.output}}`
   - Test accessing latest: `{{activity.output | last}}`
   - Unit tests for template edge cases

### Phase 4: End-to-End Testing & Performance Validation (Day 4-5)

**Focus**: Validate feature completeness and performance improvements

1. **Create Example 6** (Research Agent workflow):
   - Implement agentic research loop pattern
   - Demonstrate all three loop patterns
   - Test with mock LLM services
   - End-to-end workflow execution tests

2. **Performance testing**:
   - **Validation overhead**: Confirm validation happens once (not on every load)
   - **Loop detection overhead**: Measure O(1) metadata lookup vs O(V+E) traversal
   - **Back-edge detection overhead**: Measure O(1) flag check vs runtime analysis
   - **Benchmark**: 50+ iteration loop should not degrade orchestrator performance
   - **Compare**: Before/after metrics for hot path performance

3. **Integration testing**:
   - Test loop with budget accumulation across iterations
   - Test loop with max iterations enforced
   - Test loop with condition-only exit (Pattern 2)
   - Test loop with iteration_limit-only exit (Pattern 1)
   - Test non-iteration_scoped loop (counter tracked, no arrays)

4. **Documentation updates**:
   - Update architecture.md with loop patterns and performance optimizations
   - Update YAML syntax documentation with all three patterns
   - Document metadata caching strategy for future features
   - Add loops-guide.md with common patterns and pitfalls

---

## Success Criteria

### Core Loop Functionality

| Criterion                                                              | Status          | Evidence                                                 |
|------------------------------------------------------------------------|-----------------|----------------------------------------------------------|
| Loop detection distinguishes valid loops from circular dependencies   | 🔲 Not Started  | Unit test: `test_detect_simple_loop`                     |
| Pattern 1 (iteration_limit only) passes validation                    | 🔲 Not Started  | Unit test: `test_loop_with_iteration_limit_only`         |
| Pattern 2 (condition only) passes validation                          | 🔲 Not Started  | Unit test: `test_loop_with_condition_only`               |
| Pattern 3 (both) passes validation                                    | 🔲 Not Started  | Unit test: `test_loop_with_both`                         |
| Loop with neither condition nor limit fails validation                | 🔲 Not Started  | Unit test: `test_loop_requires_exit_mechanism`           |
| Iteration-scoped activities store outputs grouped by name as arrays   | 🔲 Not Started  | Unit test: `test_increment_iteration`                    |
| Non-iteration-scoped loops track iteration counter                    | 🔲 Not Started  | Unit test: `test_iteration_counter_without_scoping`      |
| Template resolution returns arrays for iteration-scoped activities    | 🔲 Not Started  | Unit test: `test_resolve_iteration_scoped_as_array`      |
| MiniJinja filters work on iteration arrays (`\| last`, `\| length`)   | 🔲 Not Started  | Unit test: `test_resolve_latest_with_filter`             |
| Iteration counter accessible via `{{ACTIVITY.iteration}}`             | 🔲 Not Started  | Unit test: `test_resolve_iteration_counter`              |
| Max iterations enforced to prevent infinite loops                     | 🔲 Not Started  | Integration test: `test_loop_max_iterations_enforced`    |
| Budget accumulates across all iterations (not per-iteration)          | 🔲 Not Started  | Integration test: `test_iteration_budget_accumulation`   |
| Loop exit conditions evaluated correctly                              | 🔲 Not Started  | Integration test: `test_simple_loop_workflow`            |
| Example 6 (Research Agent) executes end-to-end                        | 🔲 Not Started  | E2E test with Example 6 workflow                         |

### Performance & Tech Debt Improvements

| Criterion                                                              | Status          | Evidence                                                 |
|------------------------------------------------------------------------|-----------------|----------------------------------------------------------|
| Validation happens once at registration (not on every load)           | 🔲 Not Started  | Performance test: measure validation call count          |
| Loop metadata (`is_loop_activity`) cached in database                 | 🔲 Not Started  | Unit test: `test_metadata_cached_in_database`            |
| Back-edge metadata (`is_back_edge`) cached in database                | 🔲 Not Started  | Integration test: verify no graph traversal in hot path  |
| Orchestrator uses O(1) metadata lookups (not O(V+E) traversal)        | 🔲 Not Started  | Benchmark: compare before/after hot path performance     |
| 50+ iteration loop has no performance degradation                     | 🔲 Not Started  | Performance test: measure orchestration latency          |
| Metadata survives database round-trip (serialization/deserialization) | 🔲 Not Started  | Integration test: load workflow, verify metadata present |

**Overall US-3.4 Status**: 🔲 **Not Started** - Ready for implementation with comprehensive performance improvements

---

## Non-Goals (Post-MVP)

- ❌ Nested loops (loop within a loop)
- ❌ Dynamic loop targets (loop back to different activities based on runtime conditions)
- ❌ Parallel iterations (multiple iterations executing simultaneously)
- ❌ Visual loop representation in dashboard
- ❌ Loop replay/debugging features (step through iterations)
- ❌ Dynamic parallelism (`parallel_count` - see Epic 6.3)
- ❌ Conditional loop entry (skip loop entirely based on condition)
- ❌ Loop state snapshots (checkpoint/restore iteration state)

---

## Post-MVP Enhancement: Output Field Extraction

### Problem

Currently, built-in activities like `llm_call` return a standard structure:
```yaml
outputs:
  - name: results  # Full response object: {content: "...", usage: {...}, cost_usd: ...}
  - name: cost_usd
```

To access specific fields from JSON responses (e.g., `{"sufficient": true, "confidence": 0.9}`), users must:
1. Access the nested content: `{{activity.results.content}}`
2. Parse JSON in templates (limited MiniJinja support)
3. Or resort to workarounds like string matching

This is especially cumbersome for iteration-scoped workflows where you need to check loop conditions based on JSON fields.

### Proposed Solution

Add optional `value` field to output definitions that uses MiniJinja templates:

```yaml
outputs:
  - name: results        # Regular output: whatever the activity returns
  - name: cost_usd       # Regular output: whatever the activity returns
  - name: sufficient     # NEW: Computed from template expression
    value: "{{results.content.sufficient}}"
  - name: confidence     # NEW: Computed from template expression
    value: "{{results.content.confidence}}"
```

**How It Works**:
- Regular outputs (without `value`) are populated by the activity implementation
- Computed outputs (with `value`) are resolved via MiniJinja template after activity completes
- Template context includes all regular outputs from same activity
- Falls back to null if template evaluation fails or path doesn't exist

**Implementation**:
1. Activity executes and returns regular outputs: `{results: {...}, cost_usd: 0.05}`
2. For each output with `value`, resolve template using regular outputs as context
3. Store computed value as additional output
4. Save all outputs (regular + computed) to activity state

**Benefits**:
- Reuses existing MiniJinja template infrastructure
- Supports any expression, not just field extraction
- Can apply filters/transformations: `value: "{{results.content.confidence | round(2)}}"`
- Clean loop conditions: `{{evaluate_results.sufficient | last == false}}`
- No string matching hacks needed

**Full Example**:
```yaml
- key: evaluate_results
  activity_name: llm_call
  iteration_scoped: true
  parameters:
    prompt: |
      Evaluate if we have sufficient information.
      Return JSON: {"sufficient": true/false, "confidence": 0.0-1.0, "reasoning": "..."}
  outputs:
    - name: results       # Activity returns full response
    - name: cost_usd      # Activity returns cost
    - name: sufficient    # Computed: extract from JSON content
      value: "{{results.content.sufficient}}"
    - name: confidence    # Computed: extract and round
      value: "{{results.content.confidence | round(2)}}"
    - name: reasoning     # Computed: extract reasoning
      value: "{{results.content.reasoning}}"
  depends_on:
    - perform_search

# Later activity uses clean syntax:
- key: perform_search
  depends_on:
    - activity_key: evaluate_results
      conditions:
        - "{{evaluate_results.sufficient | last == false}}"  # Clean!
        - "{{evaluate_results.confidence | last > 0.7}}"     # Can check confidence too
```

**Advanced Use Cases**:
```yaml
# Transform output
- name: uppercase_status
  value: "{{results.content.status | upper}}"

# Conditional output
- name: needs_review
  value: "{{results.content.confidence < 0.8}}"

# Combine multiple fields
- name: summary
  value: "{{results.content.title}}: {{results.content.description}}"

# Parse nested JSON (if content itself is JSON string)
- name: inner_field
  value: "{{results.content | from_json | get(key='nested') | get(key='field')}}"
```

**Alternative Approaches Considered**:
1. **Custom `extract` syntax**: Less flexible than MiniJinja templates
2. **Auto-extract all JSON fields**: Implicit behavior, harder to debug
3. **JSONPath filter**: Requires new filter implementation
4. **Structured outputs API**: Depends on provider support (Anthropic beta feature)

**Recommendation**: Implement `value` with MiniJinja templates for MVP+1. It's the most flexible, leverages existing infrastructure, and provides powerful transformation capabilities.

---

## Risks and Mitigations

| Risk                                              | Impact | Mitigation                                                              |
|---------------------------------------------------|--------|-------------------------------------------------------------------------|
| Infinite loops despite iteration_limit            | High   | Enforce default limit of 100, add monitoring alerts                    |
| Complex loop conditions hard to debug             | Medium | Add detailed logging for loop evaluation, document common patterns     |
| Performance degradation with many iterations      | Medium | Test with 50+ iterations, optimize state serialization if needed       |
| Validation overhead on every workflow execution   | High   | ✅ **Eliminated**: Validate once at registration, cache all metadata in DB |
| Loop detection overhead in orchestrator hot path  | High   | ✅ **Eliminated**: `is_loop_activity` precomputed at validation time (O(1) vs O(V+E)) |
| Back-edge detection overhead in dependency eval   | High   | ✅ **Eliminated**: `is_back_edge` precomputed at validation time (O(1) vs O(V+E)) |
| Template array syntax confusing to users          | Medium | Clear documentation, examples, error messages                          |
| Budget tracking inaccurate across iterations      | High   | Comprehensive tests for cost accumulation                              |
| Race conditions with concurrent loop evaluations  | High   | PostgreSQL advisory locks already prevent this (existing safeguard)    |

---

## Dependencies

**Upstream** (Must be complete first):
- ✅ US-3.1: Sequential Workflows
- ✅ US-3.3: Parallel Execution

**Downstream** (Blocked by this work):
- 🔲 Example 6: Agentic Research Workflow
- 🔲 US-5.1: Multi-Provider LLM (will use loops for retry/fallback patterns)

**Parallel Work** (Can be developed independently):
- 🔲 US-3.5: Activity Settings (retry, timeout, budget) - budget tracking needed for budget-aware loops
- 🔲 US-5.3: Semantic Caching - can optimize repeated searches in loops

---

## Open Questions

1. **Should nested loops be supported in MVP?**
   - Decision: No - adds significant complexity, defer to post-MVP
   - Workaround: Users can flatten nested loops into sequential iterations

2. **How should we handle loop back-edges in workflow visualization?**
   - Decision: Document as post-MVP feature
   - For now, loops are defined in YAML but not visualized differently

3. **Should there be a global iteration_limit default?**
   - Decision: Yes - `DEFAULT_MAX_ITERATIONS = 100` (configurable via env var)
   - Prevents accidental infinite loops in production
   - Note: Default only applies when Pattern 2 (conditional-only) is used
   - **Cannot be completely disabled** (by design - safety first)
   - To effectively disable: set `iteration_limit: 999999` on specific activities
   - If default is too restrictive for your use case, explicitly set `iteration_limit` to appropriate value

4. **Must loops always have both condition AND iteration_limit?**
   - Decision: No - loops must have at least ONE exit mechanism (condition OR iteration_limit OR both)
   - Rationale:
     - Fixed iterations (Pattern 1): iteration_limit alone is cleaner (e.g., 12-issue newsletter)
     - Indefinite with exit (Pattern 2): condition alone is valid (e.g., subscription until cancellation)
     - Bounded conditional (Pattern 3): both together is safest for production
   - All three patterns prevent infinite loops while supporting different use cases

5. **How should cost accumulation work with iteration-scoped activities?**
   - Decision: `accumulated_cost_usd` tracks total across ALL iterations
   - Budget limits apply to total accumulated cost, not per-iteration
   - Budget action options: `abort` (fail activity, let orchestrator handle workflow) or `warn` (execute anyway)
   - No `skip` or `continue` action - not clearly meaningful for iteration-scoped activities
   - When budget exceeded with `action: abort`, activity fails and loop terminates

5. **Should we support dynamic loop counts (parallel_count-style)?**
   - Decision: No for MVP - only iteration-based loops (sequential)
   - Dynamic parallel loops are post-MVP

---

## Documentation Updates

**Files to Update**:
1. `docs/architecture.md` - Add section on "Iterative Workflows and Loops"
2. `docs/yaml-syntax.md` - Document `iteration_scoped`, `iteration_limit`, array template syntax
3. `docs/examples.md` - Add Example 6 with detailed explanation
4. `README.md` - Mention loop support in feature list

**New Documentation**:
1. `docs/loops-guide.md` - Comprehensive guide to loop patterns
   - Pattern 1: Fixed iterations (iteration_limit only)
   - Pattern 2: Conditional exit (condition only)
   - Pattern 3: Bounded conditional (both - recommended for production)
   - Budget-aware loops
   - Accessing iteration history
   - Common pitfalls and best practices

---

## Completion Checklist

### Phase 1: Data Model & Validation
- [ ] `ActivityState` extended with `iteration` and `iteration_outputs` fields
- [ ] `increment_iteration()` and `archive_iteration_outputs()` methods separated
- [ ] `ActivityDefinition` extended with `iteration_scoped`, `iteration_limit`, `is_loop_activity` fields
- [ ] `DependencyRelationship` struct defined with `is_back_edge` field
- [ ] Serialization uses `#[serde(default, skip_serializing_if = "is_false")]` pattern
- [ ] Loop detection implemented in `WorkflowDefinition.validate()` via topological sort
- [ ] Validation marks `is_loop_activity` and `is_back_edge` metadata
- [ ] Loop validation accepts Pattern 1 (iteration_limit only)
- [ ] Loop validation accepts Pattern 2 (condition only)
- [ ] Loop validation accepts Pattern 3 (both)
- [ ] Loop validation rejects loops with neither condition nor iteration_limit
- [ ] Metadata cached in database (survives serialization round-trip)

### Phase 2: Orchestration Logic
- [ ] `DependencyEvaluator` uses `dep.is_back_edge` flag (O(1) lookup)
- [ ] Removed `is_back_edge()` method (no graph traversal in hot path)
- [ ] `Orchestrator` uses `activity_def.is_loop_activity` flag (O(1) lookup)
- [ ] Iteration counter incremented for ALL loop activities (not just iteration_scoped)
- [ ] Outputs archived only for iteration_scoped activities
- [ ] Iteration limit enforcement with DEFAULT_MAX_ITERATIONS = 100
- [ ] Non-iteration_scoped loops work correctly (counter tracked, no arrays)

### Phase 3: Template Resolution
- [ ] `TemplateResolver` serializes iteration-scoped outputs as arrays grouped by name
- [ ] `TemplateResolver` serializes non-iteration-scoped outputs as single values
- [ ] `{{ACTIVITY.iteration}}` context variable works
- [ ] `{{ACTIVITY.accumulated_cost_usd}}` context variable works
- [ ] `{{ACTIVITY.remaining_budget_usd}}` context variable works
- [ ] MiniJinja filters (`| last`, `| first`, `| length`) work on iteration arrays

### Phase 4: Testing & Documentation
- [ ] Unit tests pass for all three loop patterns
- [ ] Unit tests pass for iteration state management
- [ ] Unit tests pass for template array resolution
- [ ] Integration tests pass for simple loop workflow
- [ ] Integration tests pass for iteration_limit enforcement
- [ ] Integration tests pass for budget accumulation across iterations
- [ ] Performance test confirms validation happens once (not on every load)
- [ ] Performance test confirms O(1) metadata lookups (no graph traversal)
- [ ] Benchmark shows 50+ iteration loop has no performance degradation
- [ ] Example 6 (Research Agent) workflow created and tested
- [ ] Documentation updated (architecture.md, YAML syntax, loops-guide.md)
- [ ] Code review complete
- [ ] Ready for production use

**US-3.4 Implementation Status**: 🔲 **Not Started** (0/38 items done)

---

## Example: Research Agent Workflow

Here's a complete example demonstrating all loop features:

```yaml
name: agentic_research_with_budget
description: "Research assistant with iteration tracking, budget limits, and conditional exit"

activities:
  # Step 1: Initialize research plan
  - key: initialize
    worker: builtin
    activity_name: llm_call
    parameters:
      provider: anthropic
      model: claude-sonnet-4-5-20250929
      prompt: |
        Create a detailed research plan for: {{INPUT.topic}}

        Generate:
        1. Focused search strategy
        2. 3-5 specific questions to answer
        3. Success criteria for "sufficient" research
        4. Key terms and concepts to track

        Return JSON: {
          "strategy": "...",
          "questions": [...],
          "criteria": "...",
          "key_terms": [...]
        }
      max_tokens: 500
    outputs:
      - name: results
      - name: cost_usd

  # Step 2: Perform iterative research (loops until sufficient)
  - key: perform_search
    worker: builtin
    activity_name: llm_call
    iteration_scoped: true
    iteration_limit: 5
    settings:
      budget:
        limit: 2.00  # $2 total across all iterations
        action: abort
    parameters:
      provider: anthropic
      model: claude-sonnet-4-5-20250929
      prompt: |
        Research topic: {{INPUT.topic}}
        Research plan: {{initialize.results.content}}

        Previous findings from past iterations: {{perform_search.results}}
        Iteration {{ACTIVITY.iteration}} - Build on previous research.

        Conduct research and return findings as JSON.
      max_tokens: 1000
    outputs:
      - name: results
      - name: cost_usd
    depends_on:
      - initialize
      - activity_key: evaluate_results
        conditions:
          - "{{evaluate_results.results | last | get(key='content') | contains(substring='CONTINUE')}}"

  # Step 3: Evaluate if we have enough information
  - key: evaluate_results
    worker: builtin
    activity_name: llm_call
    iteration_scoped: true
    settings:
      budget:
        limit: 1.00
        action: abort
    parameters:
      provider: anthropic
      model: claude-haiku-4-20250415
      prompt: |
        Topic: {{INPUT.topic}}
        All findings so far: {{perform_search.results}}
        Current iteration: {{ACTIVITY.iteration}}
        Remaining budget: ${{ACTIVITY.remaining_budget_usd}}

        Determine if we have sufficient information to write a comprehensive report.

        Respond with ONLY one word:
        - CONTINUE (if more research needed)
        - SUFFICIENT (if ready to compile report)
      max_tokens: 10
    outputs:
      - name: results
      - name: cost_usd
    depends_on:
      - perform_search

  # Step 4: Compile final report (only when sufficient)
  - key: compile_report
    worker: builtin
    activity_name: llm_call
    settings:
      budget:
        limit: 3.00
        action: abort
    parameters:
      provider: anthropic
      model: claude-sonnet-4-5-20250929
      prompt: |
        Topic: {{INPUT.topic}}
        Research plan: {{initialize.results.content}}
        All research findings: {{perform_search.results}}

        Compile a comprehensive report synthesizing all findings.
      max_tokens: 4000
    outputs:
      - name: results
      - name: cost_usd
    depends_on:
      - activity_key: evaluate_results
        conditions:
          - "{{evaluate_results.results | last | get(key='content') | contains(substring='SUFFICIENT')}}"
```

**Expected Execution Flow**:
1. `initialize` runs once, creates research plan with strategy, questions, and success criteria
2. `perform_search` iteration 0: First research iteration using the plan
3. `evaluate_results` iteration 0: Evaluates against criteria → not sufficient → loop back
4. `perform_search` iteration 1: Second iteration, builds on previous findings
5. `evaluate_results` iteration 1: Still not sufficient → loop back
6. `perform_search` iteration 2: Third iteration, synthesizes all prior research
7. `evaluate_results` iteration 2: Now sufficient → exit loop
8. `compile_report` runs once, synthesizing all findings from 3 iterations into final report

**Key Features Demonstrated**:
- ✅ Iteration-scoped storage (`perform_search` and `evaluate_results`)
- ✅ Array access to all iterations (`{{perform_search.results}}` returns array of result objects)
- ✅ Latest iteration access via MiniJinja filter (`{{evaluate_results.results | last}}`)
- ✅ Accessing nested fields (`{{activity.results.content}}` for LLM response text)
- ✅ Iteration counter (`{{ACTIVITY.iteration}}`)
- ✅ Budget accumulation across all iterations (not per-iteration reset)
- ✅ Conditional loop back (checking content for "CONTINUE")
- ✅ Conditional loop exit (checking content for "SUFFICIENT")
- ✅ Maximum iterations limit (`iteration_limit: 5`)

**MVP Limitation Note**:
The examples use simple string matching (`contains(substring='CONTINUE')`) to check loop conditions because parsing JSON fields from `results.content` is cumbersome in MVP. For production workflows, consider:
1. Having LLMs return simple strings for control flow ("CONTINUE" vs "SUFFICIENT")
2. Or use the proposed **Output Field Extraction** enhancement (see Post-MVP section below)

---

## Summary: Tech Debt Cleanup & Performance Improvements

As part of implementing US-3.4, we're addressing significant architectural tech debt that improves performance for **all** workflows, not just iterative ones:

### Problems Identified
1. **Validation overhead**: Original plan had `load_workflow_definition()` on every activity completion, potentially re-validating
2. **Loop detection overhead**: Computing `has_loop_back_edge()` with O(V+E) graph traversal on every activity completion
3. **Back-edge detection overhead**: Computing `is_back_edge()` with runtime state checks on every dependency evaluation
4. **Lost metadata**: Computed analysis discarded after validation (using `#[serde(skip)]`)

### Solutions Implemented
1. **Validate once, cache forever**:
   - Validation happens ONLY at workflow registration (`POST /api/v1/workflow_definitions`)
   - All computed metadata stored in database alongside activities JSON
   - Orchestrator loads pre-analyzed definitions (no validation in hot path)
   - **Workflow definitions are immutable** - changes create new definitions with new IDs
   - No migration complexity (each workflow instance references immutable definition)

2. **Precomputed loop metadata** (O(1) lookups):
   - `is_loop_activity: bool` on `ActivityDefinition`
   - `is_back_edge: bool` on `DependencyRelationship`
   - Both computed during validation, persisted to DB

3. **Smart serialization**:
   - `#[serde(default, skip_serializing_if = "is_false")]` pattern
   - Allows reading YAML without computed fields (defaults to false)
   - Ignores user-provided values (metadata is always computed)
   - Clean output (metadata only shown when true)
   - Backward compatible when deserializing (missing fields default to false)

### Performance Impact
| Operation          | Before                                  | After                     | Improvement                        |
|--------------------|-----------------------------------------|---------------------------|------------------------------------|
| Validation         | Every load (~100/sec for busy workflow) | Once at registration      | 100x fewer validations             |
| Loop detection     | O(V+E) per activity completion          | O(1) lookup               | Constant time                      |
| Back-edge check    | O(V+E) per dependency                   | O(1) lookup               | Constant time                      |
| **Total hot path** | **O(V+E)** graph traversals             | **O(1)** metadata lookups | Eliminates graph analysis overhead |

### Future Opportunities

With this "validate once, cache forever" pattern established, we can precompute additional graph analysis metrics (see lines 565-602 for detailed benefits):

1. **Topological Order** - 50-70% scheduler overhead reduction
   - Pre-sort activities in execution order
   - Eliminates O(V+E) topological sort on every evaluation cycle
   - Simple iteration through pre-sorted list

2. **Dependency Depth** - Priority scheduling & progress tracking
   - Enables critical path optimization
   - Powers "60% complete" style progress bars
   - Helps predict workflow completion time

3. **Parallel Execution Groups** - Batch scheduling optimization
   - Schedule 10+ parallel activities in single transaction
   - Massive overhead reduction for fan-out patterns
   - Example: 100 parallel file uploads → 1 scheduling operation instead of 100

4. **Critical Path** - SLA optimization
   - Identify minimum-latency path through workflow
   - Prioritize fast workers for critical path activities
   - Enable cost/speed tradeoffs (fast models on critical path, slow on non-critical)

**Implementation Pattern** (same for all):
- Compute during validation (one-time cost)
- Store in database with activities JSON
- Use `#[serde(default)]` for backward compatibility
- O(1) lookup in orchestrator hot path

**Estimated Impact** (based on typical workflows):
- Scheduler CPU: 50-70% reduction for sequential workflows
- Orchestration latency: 40-60% reduction for high-parallelism workflows
- Database load: 80-90% reduction for fan-out patterns (batch scheduling)

**Concrete Example** - Document processing workflow with 100 files:
```
Input → Split[100] → Process[100] → Validate[100] → Aggregate → Output
```

Without optimizations:
- 300+ orchestration cycles (check each activity individually)
- 300+ database queries to find ready activities
- 100+ separate scheduling operations for parallel activities

With precomputed metadata:
- 5 orchestration cycles (one per stage: Split, Process, Validate, Aggregate, Output)
- 5 database transactions (batch schedule each parallel group)
- **60x fewer database operations**
- **60x fewer scheduling decisions**

**US-3.4 Status**: 🔲 **Not Started** - Ready for implementation with comprehensive performance improvements
