# Kruxia Flow Python SDK

Python SDK for Kruxia Flow workflow orchestration.

## Installation

```bash
pip install kruxiaflow
```

## Quick Start

```python
from kruxiaflow import Activity, Workflow, Input

# Define workflow inputs
webhook_url = Input("webhook_url", type=str, required=True)

# Define activities
fetch_data = (
    Activity(key="fetch_data")
    .with_worker("builtin", "http_request")
    .with_params(method="GET", url="https://api.example.com/data")
)

process_data = (
    Activity(key="process_data")
    .with_worker("builtin", "transform")
    .with_params(data=fetch_data["response"])
    .with_dependencies(fetch_data)
)

# Create workflow
workflow = (
    Workflow(name="data_pipeline")
    .with_inputs(webhook_url)
    .with_activities(fetch_data, process_data)
)

# Export to YAML
print(workflow.to_yaml())
```

## Features

- **Fluent API**: Build workflows with chainable methods
- **Type-Safe**: Full type hints and Pydantic validation
- **YAML Export**: Generate workflow definitions for deployment
- **Expression Support**: Reference inputs, outputs, secrets, and environment variables

## Documentation

For full documentation, visit [kruxiaflow.dev/docs/python-sdk](https://kruxiaflow.dev/docs/python-sdk).

## License

MIT
