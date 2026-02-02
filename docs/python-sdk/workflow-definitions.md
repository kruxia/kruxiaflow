# Workflow Definitions Guide

Build type-safe, fluent workflow definitions with the Python SDK.

## Overview

The Python SDK compiles workflow definitions to YAML at deployment time. This means:

- **No runtime Python dependency** - workflows execute the same as hand-written YAML
- **Full IDE support** - autocomplete, type checking, refactoring
- **Dynamic generation** - use loops, conditionals, and functions to build workflows

> **Context**: All code in this guide is for **workflow definitions** - Python code that describes WHAT activities to run and in what order. This code compiles to YAML and is deployed to the API server. The actual activity implementations run in [worker processes](custom-workers.md). See the [Quick Start](quickstart.md#architecture-overview) for an architecture diagram.

## Activities

Activities are the units of work in a workflow. In workflow definitions, you specify:
- **Which worker** should execute the activity
- **Which activity type** on that worker to call
- **Parameters** to pass to the activity

### Basic Activity

**File: `my_workflow.py`** (Workflow Definition)

```python
from kruxiaflow import Activity

fetch = (
    Activity(key="fetch_data")
    .with_worker("std", "http_request")
    .with_params(
        method="GET",
        url="https://api.example.com/data",
    )
)
```

### Fluent Configuration Methods

All methods return `self` for chaining:

```python
activity = (
    Activity(key="process")
    .with_worker("std", "transform")
    .with_params(data="${INPUT.data}")
    .with_timeout(300)                    # 5 minute timeout
    .with_retry(max_attempts=3, backoff="exponential")
    .with_cache(ttl=3600)                 # Cache for 1 hour
    .with_budget(limit_usd=1.00, action="fail")
    .with_dependencies(fetch)
)
```

### Activity Settings

| Method | Description |
|--------|-------------|
| `.with_worker(worker, activity_name)` | Set the worker and activity type |
| `.with_params(**kwargs)` | Set activity parameters |
| `.with_timeout(seconds)` | Set execution timeout |
| `.with_retry(max_attempts, backoff)` | Configure retry policy |
| `.with_cache(ttl, key)` | Enable result caching |
| `.with_budget(limit_usd, action)` | Set cost budget |
| `.with_dependencies(*activities)` | Set activity dependencies |

## Dependencies

### Simple Dependencies

```python
step1 = Activity(key="step1").with_worker("std", "http_request")
step2 = Activity(key="step2").with_worker("std", "transform")
step3 = Activity(key="step3").with_worker("std", "http_request")

# step2 waits for step1
step2 = step2.with_dependencies(step1)

# step3 waits for both step1 and step2 (fan-in)
step3 = step3.with_dependencies(step1, step2)
```

### Conditional Dependencies

Execute activities based on conditions:

```python
from kruxiaflow import Dependency

# Only run if confidence > 0.8
notify = (
    Activity(key="notify")
    .with_worker("std", "http_request")
    .with_dependencies(
        Dependency.on(analyze, analyze["confidence"] > 0.8)
    )
)

# Run on dependency failure
retry = (
    Activity(key="retry")
    .with_worker("std", "llm_prompt")
    .with_dependencies(
        Dependency.on(analyze, analyze["status"] == "failed")
    )
)

# Combine conditions with & (AND) and | (OR)
save = (
    Activity(key="save")
    .with_worker("std", "postgres_query")
    .with_dependencies(
        Dependency.on(
            analyze,
            (analyze["confidence"] > 0.8) & (analyze["status"] == "success")
        )
    )
)
```

## Expressions

### Input References

```python
from kruxiaflow import Input

# Define typed inputs
user_id = Input("user_id", type=str, required=True)
count = Input("count", type=int, default=10)
options = Input("options", type=dict, required=False)

# Use in activity parameters
activity = (
    Activity(key="fetch")
    .with_worker("std", "http_request")
    .with_params(
        url=f"https://api.example.com/users/{user_id}",
        limit=count,
    )
)
```

### Output References

Access outputs from previous activities:

```python
fetch = Activity(key="fetch").with_worker("std", "http_request")

# Simple output access
process = (
    Activity(key="process")
    .with_params(data=fetch["response"])
)

# Nested path access
save = (
    Activity(key="save")
    .with_params(
        user_name=fetch["response.json.user.name"],
        items=fetch["response.json.data.items"],
        first_item=fetch["response.json.data.items[0]"],
    )
)
```

### Comparison Operators

Output references support comparison operators:

```python
# Returns Comparison expressions
condition1 = analyze["confidence"] > 0.8
condition2 = analyze["status"] == "success"
condition3 = analyze["error"] != None
condition4 = analyze["count"] >= 10

# Combine with logical operators
combined = (condition1 & condition2) | condition3
negated = ~condition1  # NOT
```

### Secret References

```python
from kruxiaflow import SecretRef

api_key = SecretRef("OPENAI_API_KEY")
db_password = SecretRef("DATABASE_PASSWORD")

activity = (
    Activity(key="call_api")
    .with_params(
        api_key=api_key,
        auth_header=f"Bearer {api_key}",
    )
)
```

### Environment Variables

```python
from kruxiaflow import EnvRef

db_url = EnvRef("DATABASE_URL")
api_base = EnvRef("API_BASE_URL")

activity = (
    Activity(key="connect")
    .with_params(
        connection_string=db_url,
        url=f"{api_base}/endpoint",
    )
)
```

### Workflow Metadata

```python
from kruxiaflow import workflow

activity = (
    Activity(key="log")
    .with_params(
        workflow_id=workflow.id,
        workflow_name=workflow.name,
    )
)
```

### Helper Functions

```python
from kruxiaflow import and_, or_, not_, is_null, is_not_null, contains, in_

# Null checks
is_null(activity["optional_field"])
is_not_null(activity["required_field"])

# Contains (substring)
contains(activity["text"], "error")

# In (list membership)
in_(activity["status"], ["success", "completed"])

# Combine expressions
and_(condition1, condition2, condition3)
or_(condition1, condition2)
not_(condition1)
```

## Workflows

### Basic Workflow

```python
from kruxiaflow import Workflow, Input

user_id = Input("user_id", type=str, required=True)

workflow = (
    Workflow(name="my_workflow")
    .with_version("1.0.0")
    .with_namespace("production")
    .with_inputs(user_id)
    .with_activities(fetch, process, save)
)
```

### Workflow Methods

| Method | Description |
|--------|-------------|
| `.with_version(version)` | Set semantic version |
| `.with_namespace(namespace)` | Set namespace |
| `.with_inputs(*inputs)` | Add input definitions |
| `.with_activities(*activities)` | Add activities |
| `.to_yaml()` | Compile to YAML string |
| `.to_dict()` | Convert to dictionary |

## Dynamic Workflow Generation

### Parallel Activities with Loops

```python
queries = ["AI workflows", "ML pipelines", "LLM orchestration"]

# Generate parallel search activities
searches = [
    Activity(key=f"search_{i}")
    .with_worker("std", "http_request")
    .with_params(url=f"https://api.search.com/q={query}")
    for i, query in enumerate(queries)
]

# Fan-in: aggregate depends on all searches
aggregate = (
    Activity(key="aggregate")
    .with_worker("py-std", "script")
    .with_params(
        inputs={"results": [s["response"] for s in searches]},
        script="OUTPUT = {'combined': INPUT['results']}",
    )
    .with_dependencies(*searches)
)

workflow = (
    Workflow(name="parallel_search")
    .with_activities(*searches, aggregate)
)
```

### Reusable Components

```python
def create_llm_with_fallback(name: str, prompt: str, budget: float) -> list[Activity]:
    """Create activity group with Claude -> GPT-4 fallback."""
    primary = (
        Activity(key=f"{name}_claude")
        .with_worker("std", "llm_prompt")
        .with_params(provider="anthropic", model="claude-3-haiku", prompt=prompt)
        .with_budget(limit_usd=budget * 0.7)
    )

    fallback = (
        Activity(key=f"{name}_gpt4")
        .with_worker("std", "llm_prompt")
        .with_params(provider="openai", model="gpt-4o-mini", prompt=prompt)
        .with_budget(limit_usd=budget * 0.3)
        .with_dependencies(Dependency.on(primary, primary["status"] == "failed"))
    )

    return [primary, fallback]

# Use the reusable component
activities = create_llm_with_fallback("summarize", "Summarize: ${INPUT.text}", 1.00)
workflow = Workflow(name="summarization").with_activities(*activities)
```

## API Client

### Synchronous Client

```python
from kruxiaflow import KruxiaFlow

client = KruxiaFlow(
    api_url="http://localhost:8080",
    api_token="your_token",  # Or set KRUXIAFLOW_API_TOKEN env var
)

# Deploy workflow
result = client.deploy(workflow)

# Start workflow instance
instance = client.start_workflow("my_workflow", inputs={"user_id": "123"})

# Check status
status = client.get_workflow(instance["workflow_id"])

# Get output
output = client.get_workflow_output(instance["workflow_id"])

# Cancel
client.cancel_workflow(instance["workflow_id"])
```

### Async Client

```python
from kruxiaflow import AsyncKruxiaFlow

async with AsyncKruxiaFlow(api_url="http://localhost:8080") as client:
    result = await client.deploy(workflow)
    instance = await client.start_workflow("my_workflow", inputs={"user_id": "123"})
```

## Complete Example

```python
from kruxiaflow import (
    Activity, Workflow, Input, Dependency, KruxiaFlow, workflow
)

# Define inputs
document_url = Input("document_url", type=str, required=True)
notify_email = Input("notify_email", type=str, required=True)

# Fetch document
fetch = (
    Activity(key="fetch_document")
    .with_worker("std", "http_request")
    .with_params(method="GET", url=document_url)
    .with_timeout(30)
    .with_retry(max_attempts=3)
)

# Analyze with LLM
analyze = (
    Activity(key="analyze")
    .with_worker("std", "llm_prompt")
    .with_params(
        provider="anthropic",
        model="claude-3-haiku-20240307",
        prompt=f"Analyze this document:\n\n{fetch['response.text']}",
    )
    .with_dependencies(fetch)
    .with_budget(limit_usd=0.50)
    .with_cache(ttl=3600)
)

# Save results (only if confidence > 0.7)
save = (
    Activity(key="save_results")
    .with_worker("std", "postgres_query")
    .with_params(
        query="INSERT INTO analyses (doc_url, summary, confidence) VALUES ($1, $2, $3)",
        params=[document_url, analyze["summary"], analyze["confidence"]],
    )
    .with_dependencies(
        Dependency.on(analyze, analyze["confidence"] > 0.7)
    )
)

# Send notification
notify = (
    Activity(key="notify")
    .with_worker("std", "email_send")
    .with_params(
        to=notify_email,
        subject=f"Analysis Complete: {workflow.name}",
        body=f"Document analyzed. Summary: {analyze['summary']}",
    )
    .with_dependencies(save)
)

# Build workflow
wf = (
    Workflow(name="document_analysis")
    .with_version("1.0.0")
    .with_inputs(document_url, notify_email)
    .with_activities(fetch, analyze, save, notify)
)

# Deploy
if __name__ == "__main__":
    client = KruxiaFlow(api_url="http://localhost:8080")
    client.deploy(wf)
    print(f"Deployed {wf.name}")
```
