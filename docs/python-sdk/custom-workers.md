# Custom Workers Guide

Build custom Python workers to execute your own activity implementations.

## Overview

Custom workers allow you to:

- Implement domain-specific activities in Python
- Use any Python libraries and dependencies
- Integrate with internal systems and APIs
- Deploy workers alongside your infrastructure

> **Context**: All code in this guide runs in a **worker process** - a separate Python application that connects to the Kruxia Flow API server and executes activities. This is distinct from workflow definitions which describe WHAT to run. See the [Quick Start](quickstart.md#architecture-overview) for an architecture diagram.

## Quick Start

**File: `my_worker.py`** (Worker Process)

```python
import asyncio
from kruxiaflow.worker import (
    Activity,
    ActivityContext,
    ActivityRegistry,
    ActivityResult,
    WorkerConfig,
    WorkerManager,
    activity,
)

# Define an activity using the decorator
@activity()
async def my_activity(params: dict, ctx: ActivityContext) -> ActivityResult:
    result = params["input"] * 2
    return ActivityResult.value("result", result)

async def main():
    # Load config from environment
    config = WorkerConfig()

    # Register activities
    registry = ActivityRegistry()
    registry.register(my_activity, config.worker)

    # Run worker
    manager = WorkerManager(config, registry)
    await manager.run_until_shutdown()

if __name__ == "__main__":
    asyncio.run(main())
```

Run with:

```bash
export KRUXIAFLOW_API_URL=http://localhost:8080
export KRUXIAFLOW_CLIENT_SECRET=your_secret
export KRUXIAFLOW_WORKER=my-python-worker
python my_worker.py
```

## Defining Activities

### Using the `@activity` Decorator

The simplest way to define activities:

```python
from kruxiaflow.worker import activity, ActivityContext, ActivityResult

@activity()  # Name defaults to function name
async def process_data(params: dict, ctx: ActivityContext) -> ActivityResult:
    data = params["data"]
    processed = [item.upper() for item in data]
    return ActivityResult.value("result", processed)

@activity(name="custom_name")  # Custom activity name
async def my_func(params: dict, ctx: ActivityContext) -> ActivityResult:
    return ActivityResult.value("output", "done")
```

### Using Class-Based Activities

For complex activities that need state or configuration: Class-based activities are instantiated with configuration before registration:

```python
import asyncio
from kruxiaflow.worker import (
    Activity,
    ActivityContext,
    ActivityRegistry,
    ActivityResult,
    WorkerConfig,
    WorkerManager,
)


class TextProcessor(Activity):
    """Process text with configurable settings."""

    def __init__(self, max_length: int = 1000, language: str = "en"):
        # Configuration set at instantiation time
        self.max_length = max_length
        self.language = language

    @property
    def name(self) -> str:
        return "process_text"

    async def execute(self, params: dict, ctx: ActivityContext) -> ActivityResult:
        text = params["text"]

        # Use instance configuration
        if len(text) > self.max_length:
            text = text[:self.max_length]

        ctx.logger.info(f"Processing {len(text)} chars in {self.language}")

        return ActivityResult.value("result", {
            "processed": text.upper(),
            "language": self.language,
        })


async def main():
    config = WorkerConfig()
    registry = ActivityRegistry()

    # Instantiate with custom configuration, then register
    processor = TextProcessor(max_length=500, language="es")
    registry.register(processor, config.worker)

    manager = WorkerManager(config, registry)
    await manager.run_until_shutdown()


if __name__ == "__main__":
    asyncio.run(main())
```

The key pattern:
1. **Instantiate** the activity class with configuration: `TextProcessor(max_length=500)`
2. **Register** the instance: `registry.register(processor, config.worker)`

This differs from decorator-based activities which are already instances when decorated.

## ActivityResult

### Returning Values

```python
# Single value
return ActivityResult.value("result", {"key": "value"})

# Multiple outputs
from kruxiaflow.worker import ActivityOutput, OutputType

return ActivityResult.values([
    ActivityOutput(name="summary", output_type=OutputType.VALUE, value="..."),
    ActivityOutput(name="count", output_type=OutputType.VALUE, value=42),
])
```

### Cost Tracking

Track costs for activities that incur charges (API calls, compute, etc.):

```python
from decimal import Decimal

result = ActivityResult.value("result", response)
return result.with_cost(Decimal("0.05"))
```

### Error Handling

```python
# Return an error result (activity marked as failed)
return ActivityResult.error(
    message="Invalid input format",
    code="VALIDATION_ERROR",
    retryable=False,  # Won't be retried
)

# Or raise an exception (will be caught and reported as retryable error)
raise ValueError("Something went wrong")
```

## ActivityContext

The context provides utilities for activity execution:

### Logging

```python
async def my_activity(params: dict, ctx: ActivityContext) -> ActivityResult:
    ctx.logger.info("Starting processing")
    ctx.logger.debug(f"Parameters: {params}")
    ctx.logger.warning("Rate limit approaching")
    ctx.logger.error("Failed to connect")
    # ...
```

### Heartbeats

For long-running activities, send heartbeats to prevent timeout:

```python
async def long_running(params: dict, ctx: ActivityContext) -> ActivityResult:
    items = params["items"]

    for i, item in enumerate(items):
        # Send heartbeat every 10 items
        if i % 10 == 0:
            await ctx.heartbeat()
            ctx.logger.info(f"Processed {i}/{len(items)}")

        process(item)

    return ActivityResult.value("result", {"processed": len(items)})
```

### File Operations

Download and upload files from workflow storage:

```python
async def process_file(params: dict, ctx: ActivityContext) -> ActivityResult:
    # Download file to local temp directory
    file_url = params["file_url"]
    local_path = await ctx.download_file(file_url)

    # Process the file
    with open(local_path) as f:
        content = f.read()

    result = transform(content)

    # Write result to temp file
    output_path = f"{local_path}.out"
    with open(output_path, "w") as f:
        f.write(result)

    # Upload back to storage
    result_url = await ctx.upload_file(output_path, "result.txt")

    return ActivityResult.value("result", {"url": result_url})
```

### Context Properties

```python
async def my_activity(params: dict, ctx: ActivityContext) -> ActivityResult:
    # Access workflow and activity IDs
    print(f"Workflow ID: {ctx.workflow_id}")
    print(f"Activity ID: {ctx.activity_id}")
    print(f"Activity Key: {ctx.activity_key}")
    # ...
```

## Worker Configuration

Configuration is loaded from environment variables:

| Environment Variable | Description | Default |
|---------------------|-------------|---------|
| `KRUXIAFLOW_API_URL` | API server URL | `http://localhost:8080` |
| `KRUXIAFLOW_CLIENT_ID` | OAuth client ID | (required) |
| `KRUXIAFLOW_CLIENT_SECRET` | OAuth client secret | (required) |
| `KRUXIAFLOW_WORKER` | Worker type name | `python` |
| `KRUXIAFLOW_WORKER_ID` | Unique worker instance ID | Auto-generated |
| `KRUXIAFLOW_WORKER_POLL_INTERVAL` | Poll interval (seconds) | `0.1` |
| `KRUXIAFLOW_WORKER_MAX_ACTIVITIES` | Max concurrent activities | `16` |
| `KRUXIAFLOW_WORKER_ACTIVITY_TIMEOUT` | Default timeout (seconds) | `300` |
| `KRUXIAFLOW_WORKER_HEARTBEAT_INTERVAL` | Heartbeat interval (seconds) | `30` |

> **Note**: The Rust worker uses slightly different variable names for some settings. See [SDK Discrepancies](../implementation/sdk-discrepancies.md) for details on alignment planned before v1.0.

### Programmatic Configuration

```python
from kruxiaflow.worker import WorkerConfig

# The parameters of WorkerConfig default to the value in the corresponding env variable. 
# For example, WorkerConfig.api_url defaults to the value of KRUXIAFLOW_WORKER_API_URL.
config = WorkerConfig(
    api_url="http://localhost:8080",
    client_id="my-client",
    client_secret="secret",
    worker="my-worker",
    max_concurrent_activities=32,
    activity_timeout=600,
)
```

## Registering Activities

```python
from kruxiaflow.worker import ActivityRegistry

registry = ActivityRegistry()

# Register decorator-based activities
registry.register(my_activity, "my-worker")
registry.register(another_activity, "my-worker")

# Register class-based activities
registry.register(ProcessFileActivity(), "my-worker")
registry.register(ProcessFileActivity(chunk_size=500), "my-worker")  # Custom config

# List registered activities
print(registry.activity_types())
# ['my-worker.my_activity', 'my-worker.another_activity', 'my-worker.process_file']
```

## Running Workers

### Basic Execution

```python
async def main():
    config = WorkerConfig()
    registry = ActivityRegistry()
    registry.register(my_activity, config.worker)

    manager = WorkerManager(config, registry)
    await manager.run_until_shutdown()  # Runs until SIGINT/SIGTERM

asyncio.run(main())
```

### Graceful Shutdown

The worker handles `SIGINT` and `SIGTERM` for graceful shutdown:

1. Stops accepting new activities
2. Waits for in-progress activities to complete
3. Closes connections

### Docker Deployment

```dockerfile
FROM python:3.12-slim

WORKDIR /app

# Install dependencies
COPY requirements.txt .
RUN pip install -r requirements.txt

# Copy worker code
COPY worker.py .

# Run worker
CMD ["python", "worker.py"]
```

```yaml
# docker-compose.yml
services:
  my-worker:
    build: .
    environment:
      KRUXIAFLOW_API_URL: http://api:8080
      KRUXIAFLOW_CLIENT_ID: my-worker
      KRUXIAFLOW_CLIENT_SECRET: ${WORKER_SECRET}
      KRUXIAFLOW_WORKER: my-python-worker
      KRUXIAFLOW_WORKER_MAX_ACTIVITIES: 16
    deploy:
      replicas: 3
```

## Using Custom Workers in Workflows

Reference your custom worker in workflow definitions:

```python
from kruxiaflow import Activity, Workflow

# Use your custom worker
process = (
    Activity(key="process")
    .with_worker("my-python-worker", "process_data")  # worker name, activity name
    .with_params(data=["item1", "item2", "item3"])
)

workflow = Workflow(name="my_workflow").with_activities(process)
```

## Complete Example

```python
#!/usr/bin/env python3
"""Custom Python worker with multiple activities."""

import asyncio
from decimal import Decimal

from kruxiaflow.worker import (
    Activity,
    ActivityContext,
    ActivityRegistry,
    ActivityResult,
    WorkerConfig,
    WorkerManager,
    activity,
)


@activity()
async def analyze_text(params: dict, ctx: ActivityContext) -> ActivityResult:
    """Analyze text sentiment."""
    text = params["text"]

    # Simple keyword-based sentiment analysis
    positive = {"good", "great", "excellent", "happy", "love"}
    negative = {"bad", "terrible", "awful", "sad", "hate"}

    words = set(text.lower().split())
    pos_count = len(words & positive)
    neg_count = len(words & negative)

    if pos_count > neg_count:
        sentiment, confidence = "positive", pos_count / (pos_count + neg_count + 1)
    elif neg_count > pos_count:
        sentiment, confidence = "negative", neg_count / (pos_count + neg_count + 1)
    else:
        sentiment, confidence = "neutral", 0.5

    return ActivityResult.value("result", {
        "sentiment": sentiment,
        "confidence": confidence,
    })


@activity()
async def call_external_api(params: dict, ctx: ActivityContext) -> ActivityResult:
    """Call an external API with cost tracking."""
    import httpx

    url = params["url"]
    method = params.get("method", "GET")

    async with httpx.AsyncClient() as client:
        response = await client.request(method, url)
        response.raise_for_status()

    # Track API cost
    cost = Decimal("0.001")  # $0.001 per request

    return ActivityResult.value("result", {
        "status": response.status_code,
        "body": response.json(),
    }).with_cost(cost)


class BatchProcessor(Activity):
    """Process items in batches with heartbeating."""

    def __init__(self, batch_size: int = 100):
        self.batch_size = batch_size

    @property
    def name(self) -> str:
        return "batch_process"

    async def execute(self, params: dict, ctx: ActivityContext) -> ActivityResult:
        items = params["items"]
        results = []

        for i in range(0, len(items), self.batch_size):
            batch = items[i:i + self.batch_size]
            await ctx.heartbeat()
            ctx.logger.info(f"Processing batch {i // self.batch_size + 1}")

            # Process batch
            for item in batch:
                results.append(item.upper())

        return ActivityResult.value("result", {
            "processed": len(results),
            "items": results,
        })


async def main():
    config = WorkerConfig()

    registry = ActivityRegistry()
    registry.register(analyze_text, config.worker)
    registry.register(call_external_api, config.worker)
    registry.register(BatchProcessor(batch_size=50), config.worker)

    print(f"Worker: {config.worker}")
    print(f"Activities: {registry.activity_types()}")

    manager = WorkerManager(config, registry)
    await manager.run_until_shutdown()


if __name__ == "__main__":
    asyncio.run(main())
```
