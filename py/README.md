# Kruxia Flow Python SDK

Python SDK for Kruxia Flow workflow orchestration.

## Installation

```bash
pip install kruxiaflow
```

## Quick Start

```python
from kruxiaflow import Activity, Workflow, Input, KruxiaFlow

# Define workflow inputs
webhook_url = Input("webhook_url", type=str, required=True)

# Define activities with fluent API
fetch_data = (
    Activity(key="fetch_data")
    .with_worker("std", "http_request")
    .with_params(method="GET", url="https://api.example.com/data")
)

process_data = (
    Activity(key="process_data")
    .with_worker("std", "transform")
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

# Or deploy directly
client = KruxiaFlow(api_url="http://localhost:8080")
client.deploy(workflow)
```

## Features

- **Fluent API**: Build workflows with chainable methods
- **Type-Safe**: Full type hints and Pydantic validation
- **Expression System**: SQLAlchemy-style operators for conditions
- **YAML Export**: Generate workflow definitions for deployment
- **Standard Workers**: Pre-built Docker images with common packages
- **Worker SDK**: Build custom Python activity workers

## Documentation

- [Quick Start Guide](../docs/python-sdk/quickstart.md)
- [Workflow Definitions](../docs/python-sdk/workflow-definitions.md)
- [Standard Workers](../docs/python-sdk/standard-workers.md)
- [Custom Workers](../docs/python-sdk/custom-workers.md)

## Standard Workers

Pre-built Docker images for common use cases:

| Worker | Packages | Use Case |
|--------|----------|----------|
| `py-std` | httpx, pydantic, orjson | API calls, JSON processing |
| `py-data` | pandas, polars, duckdb | ETL, data transformation |
| `py-ml` | sklearn, torch, numpy | ML training/inference |
| `py-nlp` | transformers, spacy | Text processing, embeddings |

```yaml
# Use in workflows (either YAML or Python SDK)
activities:
  transform:
    worker: py-data
    activity_name: script
    parameters:
      script: |
        import pandas as pd
        df = pd.DataFrame(INPUT["data"])
        OUTPUT = {"result": df.describe().to_dict()}
```

## Development

```bash
# Install dev dependencies
uv sync --dev

# Run tests
uv run pytest

# Lint and format
uv run ruff check
uv run ruff format

# Type check
uv run ty check
```

## License

MIT
