# Python SDK Quick Start

Get started with the Kruxia Flow Python SDK in minutes.

## Installation

```bash
pip install kruxiaflow
```

For worker development, no additional installation is required - the worker SDK is included.

## Architecture Overview

The Python SDK has two distinct contexts:

```
┌─────────────────────────────────────────────────────────────────┐
│  WORKFLOW DEFINITION (my_workflow.py)                           │
│  ─────────────────────────────────────                          │
│  Describes WHAT to run. Compiles to YAML and deploys to API.    │
│                                                                 │
│  from kruxiaflow import Activity, Workflow                      │
│  activity = Activity(key="process")                             │
│      .with_worker("my-worker", "analyze")  ← references worker  │
└─────────────────────────────────────────────────────────────────┘
                              │
                              │ deploys to
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                     KRUXIA FLOW API SERVER                      │
└─────────────────────────────────────────────────────────────────┘
                              ▲
                              │ polls for work
                              │
┌─────────────────────────────────────────────────────────────────┐
│  WORKER PROCESS (my_worker.py)                                  │
│  ─────────────────────────────                                  │
│  Contains the actual CODE that executes activities.             │
│                                                                 │
│  from kruxiaflow.worker import activity, ActivityResult         │
│  @activity()                                                    │
│  async def analyze(params, ctx):  ← actual implementation       │
│      return ActivityResult.value("result", ...)                 │
└─────────────────────────────────────────────────────────────────┘
```

**Key distinction:**
- **`kruxiaflow.Activity`** - Workflow definition (describes what to run)
- **`kruxiaflow.worker.Activity`** - Worker implementation (the code that runs)

## Your First Workflow

Workflow definitions describe the structure and flow of activities.

**File: `my_workflow.py`** (Workflow Definition)

```python
from kruxiaflow import Activity, Input, Workflow, KruxiaFlow

# Define workflow inputs
webhook_url = Input("webhook_url", type=str, required=True)

# Step 1: Fetch data from an API
fetch_data = (
    Activity(key="fetch_data")
    .with_worker("std", "http_request")
    .with_params(
        method="GET",
        url="https://api.example.com/data",
    )
)

# Step 2: Send notification with the fetched data
# Uses fetch_data["response"] to reference the output
send_notification = (
    Activity(key="send_notification")
    .with_worker("std", "http_request")
    .with_params(
        method="POST",
        url=webhook_url,
        headers={"Content-Type": "application/json"},
        body={"data": fetch_data["response.json"]},
    )
    .with_dependencies(fetch_data)
)

# Build the workflow
workflow = (
    Workflow(name="my_first_workflow")
    .with_inputs(webhook_url)
    .with_activities(fetch_data, send_notification)
)

# Print the compiled YAML
print(workflow.to_yaml())
```

Run it:

```bash
python my_workflow.py
```

This prints the compiled YAML workflow definition.

## Deploy to Kruxia Flow

**File: `my_workflow.py`** (Workflow Definition)

```python
# Deploy to a running Kruxia Flow server
client = KruxiaFlow(
    api_url="http://localhost:8080",
    api_token="${KRUXIAFLOW_TOKEN}",  # Or set KRUXIAFLOW_API_TOKEN env var
)

# Deploy the workflow definition
result = client.deploy(workflow)
print(f"Deployed: {result['name']} v{result['version']}")

# Start a workflow instance
instance = client.start_workflow(
    "my_first_workflow",
    inputs={"webhook_url": "https://example.com/webhook"},
)
print(f"Started workflow: {instance['workflow_id']}")
```

## Key Concepts

### Activities (Workflow Definition)

In workflow definitions, activities describe WHAT to run:

**File: `my_workflow.py`** (Workflow Definition)

```python
from kruxiaflow import Activity

activity = (
    Activity(key="unique_key")
    .with_worker("std", "http_request")  # worker type, activity name
    .with_params(url="https://api.example.com")
    .with_timeout(60)  # seconds
    .with_retry(max_attempts=3)
)
```

### Dependencies (Workflow Definition)

Control execution order with dependencies:

**File: `my_workflow.py`** (Workflow Definition)

```python
step2 = (
    Activity(key="step2")
    .with_worker("std", "transform")
    .with_dependencies(step1)  # Waits for step1 to complete
)
```

### Output References (Workflow Definition)

Reference outputs from previous activities:

**File: `my_workflow.py`** (Workflow Definition)

```python
# Access nested JSON paths
save = (
    Activity(key="save")
    .with_worker("std", "postgres_query")
    .with_params(
        query="INSERT INTO results (value) VALUES ($1)",
        params=[fetch["response.json.data.value"]],
    )
    .with_dependencies(fetch)
)
```

### Expressions (Workflow Definition)

Use expressions for dynamic values:

**File: `my_workflow.py`** (Workflow Definition)

```python
from kruxiaflow import Input, SecretRef, EnvRef

# Workflow inputs
user_id = Input("user_id", type=str, required=True)

# Secrets (loaded from Kruxia Flow secrets store)
api_key = SecretRef("OPENAI_API_KEY")

# Environment variables
db_url = EnvRef("DATABASE_URL")
```

## Next Steps

- [Workflow Definitions Guide](workflow-definitions.md) - Complete API reference for workflow definitions
- [Custom Workers Guide](custom-workers.md) - Build your own Python activity implementations
- [Standard Workers Guide](standard-workers.md) - Use pre-built Python workers
