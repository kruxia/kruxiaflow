2025-10-21

# Activity Settings

The StreamFlow codebase supports these settings for each activity step:

1. retry_policy (Basic Retry)

  - Default: 3 attempts with exponential backoff
  - Simple retry with the same parameters
  - Fields:
    - max_attempts: Number of retry attempts
    - backoff: Backoff strategy between retries

2. retry_strategy (Advanced Retry)

  - Intelligent retry with parameter modification
  - Can switch providers, models, or adjust parameters on retry
  - Types:
    - Fixed: Same parameters for all retries
    - Exponential: Parameter adjustments with exponential backoff
    - Adaptive: Fallback chains with different providers/models
  - Includes budget limits for retry costs

3. timeout (Timeout Settings)

  - Default: 60 seconds with Fail strategy
  - Fields:
    - timeout: Duration before timeout
    - on_timeout: Strategy (Fail, Retry, or Fallback)
    - enable_heartbeat: For long-running activities (>60s)
    - heartbeat_interval: How often to send heartbeats
    - warning_threshold: When to emit warning (default 0.8 = 80%)

4. budget (Cost Limits)

  - Optional spending limit for the activity
  - Fields:
    - limit: Maximum cost in USD
    - on_exceeded: Action when exceeded (Abort, Continue, Alert)


```yaml
settings:
  retry_policy:  # Basic retry (if not using retry_strategy)
    max_attempts: 3
    backoff: exponential  # or fixed, linear
  timeout:  # Optional timeout settings
    timeout: 10
    on_timeout: fail  # or retry, fallback
    enable_heartbeat: false  # Not needed for short activities
  budget:  # Optional budget limit
    limit: 0.10  # $0.10 USD
    on_exceeded: abort  # or continue, alert
---
settings:
  retry_strategy:  # Advanced retry with modifications
    type: adaptive
    attempts:
      - provider: primary
        timeout: 10
        parameters:
          processing_mode: fast
      - provider: fallback
        timeout: 15
        parameters:
          processing_mode: robust
    max_total_cost: 1.0  # Retry budget limit
    backoff:
      type: exponential
      initial_ms: 1000
      multiplier: 2.0
      max_ms: 10000
  timeout:
    timeout: 10
    on_timeout: retry
    max_attempts: 2
    backoff: 5
```

Key Points:

1. retry_policy and retry_strategy are mutually exclusive - use one or the other
2. timeout is optional and only needed for activities that might timeout
3. budget is optional and used for cost-constrained workflows
4. scheduled_at is optional for delayed activity execution
5. The settings block itself is optional - if omitted, defaults are used

Minimal Example (Using Defaults):
```yaml
activities:
  - key: simple_activity
    worker: example
    workflow_id: workflow_{{uuid}}
    parameters:
      input: "data"
    outputs:
      - result
    # No settings block - uses all defaults:
    # - 3 retry attempts with exponential backoff
    # - 60 second timeout with immediate failure
    # - No budget limits
    following:
      - activity_key: next_activity
```