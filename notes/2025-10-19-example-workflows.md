2025-10-19

# StreamFlow Example Workflows

The StreamFlow examples are located in the `streamflow-examples` crate, which is a separate binary and library that keeps the core StreamFlow libraries clean and lightweight. All example workflows can be explored via the CLI:

```bash
# List all examples
cargo run --bin streamflow-examples list

# Get info about a specific example
cargo run --bin streamflow-examples info autonomous_research

# Run an example (demonstration mode)
cargo run --bin streamflow-examples run autonomous_research
```

## Comprehensive Example Workflows

### 1. Autonomous Research Agent
- **Code**: `streamflow-examples/src/workflows/autonomous_research.rs`
- **Tests**: `streamflow-examples/src/workflows/autonomous_research_comprehensive_tests.rs`
- **Documentation**: `book/src/examples/autonomous-research.md`
- **Description**: Dynamic parallel searches, iterative loops, context accumulation, and budget tracking for autonomous AI research.
- **Key Patterns**: Parallel Activities, Iterative Loops, Context Accumulation, Budget Tracking

### 2. Adaptive Content Generator
- **Code**: `streamflow-examples/src/workflows/adaptive_content.rs`
- **Documentation**: `book/src/examples/adaptive-content.md`
- **Description**: Quality gates, budget limits, retry with strategy modification, and parallel variant generation.
- **Key Patterns**: Budget Tracking, Retry with Strategy Modification

### 3. Code Review & Fix Agent
- **Code**: `streamflow-examples/src/workflows/code_review_fix_agent.rs`
- **Documentation**: `docs/implementation-architecture/phase-10.1-code-review-fix-agent.md`
- **Description**: Parallel analysis, human approval gates, retry with strategy modification, auto-fix minor issues, and comprehensive reporting.
- **Key Patterns**: Parallel Activities, Retry with Strategy Modification, External Signals

### 4. Multi-Agent Debate System
- **Code**: `streamflow-examples/src/workflows/multi_agent_debate.rs`
- **Documentation**:
    - `book/src/examples/multi-agent-debate.md`
    - `docs/implementation-architecture/phase-10.2-multi-agent-debate.md`
- **Description**: Fixed parallel agents, dual exit conditions, progress tracking, context accumulation, and consensus-based convergence.
- **Key Patterns**: Parallel Activities, Iterative Loops, Progress Tracking

### 5. Dynamic Task Decomposition Agent
- **Code**: `streamflow-examples/src/workflows/dynamic_task_decomposition.rs`
- **Documentation**:
    - `book/src/examples/dynamic-task-decomposition.md`
    - `docs/implementation-architecture/phase-10.3-dynamic-task-decomposition.md`
- **Description**: Fully dynamic activity scheduling, dependency-aware parallel execution, nested task decomposition, adaptive replanning on failure.
- **Key Patterns**: Parallel Activities, Conditional Routing, Retry with Strategy Modification

### 6. Human-Supervised AI Code Generator
- **Code**: `streamflow-examples/src/workflows/human_supervised_code_gen.rs`
- **Documentation**:
    - `book/src/examples/human-supervised-code-generation.md`
    - `docs/implementation-architecture/phase-10.4-human-supervised-ai-code-generator.md`
- **Description**: LLM code generation with parallel testing, mandatory human approval, timeout handling, rejection retry with feedback incorporation.
- **Key Patterns**: Parallel Activities, Conditional Routing, Non-Deterministic Activities, External Signals

### 7. Multi-Stage Approval Pipeline
- **Code**: `streamflow-examples/src/workflows/multi_stage_approval.rs`
- **Documentation**:
    - `book/src/examples/multi-stage-approval.md`
    - `docs/implementation-architecture/phase-10.5-multi-stage-approval-pipeline.md`
- **Description**: Sequential approval gates with different timeout strategies, signal payload processing, parallel variant generation, conditional routing.
- **Key Patterns**: External Signals, Conditional Routing, Parallel Activities

### 8. Data Pipeline
- **Code**: `streamflow-examples/src/workflows/data_pipeline.rs`
- **Documentation**:
    - `book/src/examples/data-pipeline.md`
    - `docs/implementation-architecture/phase-10.6-data-pipeline-example.md`
- **Description**: Production-ready ETL pattern with sequential stages, parallel chunk processing, progress tracking, comprehensive error handling.
- **Key Patterns**: Parallel Activities, Progress Tracking, Error Handling

### 9. Order Fulfillment Saga
- **Code**: `streamflow-examples/src/workflows/order_fulfillment_saga.rs`
- **Documentation**:
    - `book/src/examples/order-fulfillment-saga.md`
    - `docs/implementation-architecture/phase-10.7-order-fulfillment-saga.md`
- **Description**: Saga pattern with compensation logic for distributed transactions, sequential saga steps with rollback on failure, external signal integration.
- **Key Patterns**: Saga Pattern, Compensation Logic, External Signals

## Additional Example Workflows

### 10. Cost-Constrained Research Pipeline
- **Code**: `streamflow-examples/src/workflows/cost_constrained_research.rs`
- **Documentation**: `book/src/examples/cost-constrained-research.md`
- **Description**: Strict budget constraints, per-activity budgets, and provider fallback chains.
- **Key Patterns**: Budget Tracking, Retry with Strategy Modification

### 11. Non-Deterministic Creative Workflow
- **Code**: `streamflow-examples/src/workflows/creative_recovery.rs`
- **Documentation**: `book/src/examples/creative-recovery.md`
- **Description**: Proper handling of non-deterministic activities with crash recovery and result replay.
- **Key Patterns**: Non-Deterministic Activities

### 12. Payment Processing
- **Code**: `streamflow-examples/src/workflows/payment_processing.rs`
- **Documentation**: `book/src/examples/payment.md`
- **Description**: Sequential activities, error handling, retry logic, and compensation on failure.
- **Key Patterns**: Basic workflow patterns, Error Handling

### 13. AI Customer Support
- **Code**: `streamflow-examples/src/workflows/ai_customer_support.rs`
- **Documentation**: `book/src/examples/ai-customer-support.md`
- **Description**: Streaming responses, context management, cost tracking, and conditional routing.
- **Key Patterns**: Streaming, Conditional Routing

## Pattern Example Workflows

These workflows demonstrate specific StreamFlow patterns in isolation:

### 14. Parallel Research (Pattern Example)
- **Code**: `streamflow-examples/src/workflows/parallel_research.rs`
- **Documentation**: `book/src/guide/parallel-activities.md`
- **Description**: Dynamic parallel activity execution with flexible wait strategies (All, Any, FirstN).
- **Key Patterns**: Parallel Activities

### 15. Iterative Loops (Pattern Example)
- **Code**: `streamflow-examples/src/workflows/iterative_loops.rs`
- **Documentation**: `book/src/guide/iterative-loops.md`
- **Description**: Loop patterns with max iteration guards, LLM-based exit conditions.
- **Key Patterns**: Iterative Loops

### 16. Context Accumulation (Pattern Example)
- **Code**: `streamflow-examples/src/workflows/context_accumulation.rs`
- **Documentation**: `book/src/guide/context-accumulation.md`
- **Description**: Context management, state passing, filtering, and summarization for multi-turn conversations.
- **Key Patterns**: Context Accumulation

### 17. Conditional Routing (Pattern Example)
- **Code**: `streamflow-examples/src/workflows/conditional_routing.rs`
- **Documentation**: `book/src/guide/conditional-routing.md`
- **Description**: If/else branching, switch/match patterns, dynamic activity selection, LLM-driven routing.
- **Key Patterns**: Conditional Routing

### 18. Determinism (Pattern Example)
- **Code**: `streamflow-examples/src/workflows/determinism.rs`
- **Documentation**:
    - `book/src/guide/determinism.md`
    - `book/src/examples/determinism.md`
- **Description**: Deterministic vs non-deterministic activity handling with proper crash recovery semantics.
- **Key Patterns**: Non-Deterministic Activities

### 19. Retry Strategies (Pattern Example)
- **Code**: `streamflow-examples/src/workflows/retry_strategies.rs`
- **Documentation**:
    - `book/src/guide/errors.md`
    - `book/src/examples/retry-strategies.md`
- **Description**: Retry with strategy modification - fallback chains, parameter adjustment, provider switching.
- **Key Patterns**: Retry with Strategy Modification

### 20. Streaming Activity (Pattern Example)
- **Code**: `streamflow-examples/src/workflows/streaming_activity.rs`
- **Documentation**: `book/src/guide/streaming.md`
- **Description**: Real-time token streaming for AI workloads via WebSocket.
- **Key Patterns**: Streaming

### 21. Timeout Patterns (Pattern Example)
- **Code**: `streamflow-examples/src/workflows/timeout_patterns.rs`
- **Documentation**: Referenced in implementation docs for extended timeouts
- **Description**: Extended timeouts, heartbeat mechanism, timeout strategies, manual progress reporting for long-running activities.
- **Key Patterns**: Extended Timeouts & Heartbeats

## Benchmark Workflows

### 22. Benchmark Pipeline
- **Code**: `streamflow-examples/src/workflows/benchmark_pipeline.rs`
- **Documentation**: `benchmark-suite/docs/phase-2-status.md`
- **Description**: Performance benchmarking workflow for testing StreamFlow throughput and latency.
- **Key Patterns**: Performance Testing

```yaml
# Benchmark Pipeline Workflow Definition
# Sequential ETL pipeline for benchmarking StreamFlow performance
activities:
  # Step 1: Extract data from source
  - name: extract_data
    namespace: streamflow_examples
    parameters:
      source: "{{ARG.source}}"  # Optional: source system identifier
      data_size_kb: "{{ARG.data_size_kb}}"  # Optional: KB to generate (default: 5)
      duration_ms: "{{ARG.extract_duration_ms}}"  # Optional: simulated delay (default: 200)
    outputs:
      - activity
      - status
      - duration_ms
      - data_size_kb
      - record_count
      - timestamp

  # Step 2: Transform the extracted data
  - name: transform_data
    namespace: streamflow_examples
    parameters:
      extract_result: "{{extract_data}}"  # Full output from extract_data
      operations: "{{ARG.transform_operations}}"  # Optional: ops list (default: ["normalize", "enrich"])
      duration_ms: "{{ARG.transform_duration_ms}}"  # Optional: simulated delay (default: 300)
    outputs:
      - activity
      - status
      - duration_ms
      - operations
      - input_records
      - output_records
      - timestamp
    preceding:
      - activity_key: extract_data
        conditions:
          - preceding.succeeded: true

  # Step 3: Validate the transformed data
  - name: validate_data
    namespace: streamflow_examples
    parameters:
      transform_result: "{{transform_data}}"  # Full output from transform_data
      rules: "{{ARG.validation_rules}}"  # Optional: rules list (default: ["schema_check", "data_quality"])
      duration_ms: "{{ARG.validate_duration_ms}}"  # Optional: simulated delay (default: 150)
    outputs:
      - activity
      - status
      - duration_ms
      - checks
      - valid_records
      - timestamp
    preceding:
      - activity_key: transform_data
        conditions:
          - preceding.succeeded: true

  # Step 4: Load data to destination
  - name: load_data
    namespace: streamflow_examples
    parameters:
      validate_result: "{{validate_data}}"  # Full output from validate_data
      destination: "{{ARG.destination}}"  # Optional: target system (default: "benchmark_output")
      duration_ms: "{{ARG.load_duration_ms}}"  # Optional: simulated delay (default: 250)
    outputs:
      - activity
      - status
      - duration_ms
      - destination
      - records_loaded
      - timestamp
    preceding:
      - activity_key: validate_data
        conditions:
          - preceding.succeeded: true

  # Step 5: Complete pipeline and summarize results
  - name: complete_pipeline
    namespace: streamflow_examples
    parameters:
      results:  # Map of all previous activity results
        extract: "{{extract_data}}"
        transform: "{{transform_data}}"
        validate: "{{validate_data}}"
        load: "{{load_data}}"
      duration_ms: "{{ARG.complete_duration_ms}}"  # Optional: simulated delay (default: 100)
    outputs:
      - activity
      - status
      - duration_ms
      - total_pipeline_duration_ms
      - activities_completed
      - completion_time
    preceding:
      - activity_key: load_data
        conditions:
          - preceding.succeeded: true

# Notes:
# - Settings block omitted as defaults are sufficient (3 retries, 60s timeout)
# - {{ARG.name}} indicates parameters provided at workflow start
# - {{activity_name}} references output from a previous activity
# - All duration_ms parameters are optional with activity-specific defaults
# - Total expected duration: ~1000ms (200+300+150+250+100) plus orchestration overhead
```

### 23. Benchmark Pipeline Workflow
- **Code**: `streamflow-examples/src/workflows/benchmark_pipeline_workflow.rs`
- **Description**: Extended benchmark pipeline workflow implementation.
- **Key Patterns**: Performance Testing

## Supporting Files

- **Module Definition**: `streamflow-examples/src/workflows/mod.rs` - Module organization and public exports
- **Main Entry Point**: `streamflow-examples/src/main.rs` - CLI for exploring and running examples
- **Library Entry**: `streamflow-examples/src/lib.rs` - Library interface for the examples crate
- **Streaming Demo**: `streamflow-examples/examples/streaming_demo.rs` - Standalone streaming demonstration

## Documentation References

### User Guide Documentation
- `book/src/getting-started/first-workflow.md` - Tutorial on creating your first workflow
- `book/src/guide/workflows.md` - General workflow concepts
- `book/src/guide/activities.md` - Creating and using activities
- `book/src/examples/custom.md` - Guide for creating custom workflows

### Development Reports
Multiple development reports document the implementation of these example workflows:

- `docs/development-reports/2025-10-08-phase-11-example-workflows.md`
- `docs/development-reports/2025-10-09-phase-10.1-code-review-fix-agent.md`
- `docs/development-reports/2025-10-09-phase-10.2-multi-agent-debate.md`
- `docs/development-reports/2025-10-09-phase-10.3-dynamic-task-decomposition.md`
- `docs/development-reports/2025-10-10-phase-10.4-human-supervised-ai-code-generator.md`
- `docs/development-reports/2025-10-10-phase-10.5-multi-stage-approval-pipeline.md`
- `docs/development-reports/2025-10-10-phase-10.6-data-pipeline-example.md`
- `docs/development-reports/2025-10-10-phase-10.7-order-fulfillment-saga.md`

### Test Reports
Various test reports validate the example implementations:

- `docs/test-reports/2025-10-09T00-00-00Z-code-review-fix-agent-test-status.md`
- `docs/test-reports/2025-10-09-dynamic-task-decomposition.md`
- `docs/test-reports/2025-10-10T03-05-53Z-multi-agent-debate-test-report.md`

## Usage Notes

1. **Running Examples**: All examples require the StreamFlow server to be running (`cargo run --bin streamflow serve`)
2. **Book Documentation**: The mdBook (`book/src/examples/`) contains user-facing documentation for most production examples
3. **Implementation Architecture**: Technical implementation details are in `docs/implementation-architecture/`
4. **Pattern Learning Path**: Start with pattern examples to understand individual concepts before exploring comprehensive workflows
5. **Test Coverage**: Most workflows have comprehensive test coverage, with tests embedded in the source files