# Python Script Activities from Functions

**Status:** ✅ Implemented
**Since:** 2026-02-04
**Updated:** 2026-02-04

## Overview

Kruxia Flow supports defining Python script activities using actual Python functions via the `ScriptActivity.from_function()` decorator. This enables IDE features like syntax highlighting, code validation, linting, and auto-formatting, making workflow development more productive and less error-prone.

The decorator creates Activity **instances** for workflows (not activity implementations), with the activity key defaulting to the function name for consistency with the custom worker `@activity` decorator.

## Distinction: Activity Implementations vs Activity Instances

Understanding the difference is crucial:

| Feature                | Custom Worker `@activity`                       | `@ScriptActivity.from_function`        |
|------------------------|-------------------------------------------------|----------------------------------------------|
| **Purpose**            | Define activity implementation                  | Create activity instance                     |
| **Module**             | `kruxiaflow.worker.activity`                    | `kruxiaflow.models.ScriptActivity`     |
| **Creates**            | Activity type that workers execute              | Specific activity in a workflow              |
| **Parameter**          | `name=` (defaults to function name)             | `key=` (defaults to function name)           |
| **Function Signature** | `(params: dict, ctx: ActivityContext)`          | `(INPUT)` with implicit variables            |
| **Return**             | `ActivityResult`                                | Set `OUTPUT` dict                            |
| **Use Case**           | Building reusable activity types                | Quick inline scripts in workflows            |
| **Registration**       | Registered with worker                          | Added to workflow activities list            |

### Custom Worker Example (Activity Implementation)

```python
from kruxiaflow.worker import activity, ActivityContext, ActivityResult

@activity(name="transform_data")  # name defaults to function name
async def transform_data(params: dict, ctx: ActivityContext) -> ActivityResult:
    """Defines WHAT the worker executes."""
    result = process(params["input"])
    return ActivityResult.value("result", result)
```

### Script Activity Example (Activity Instance)

```python
from kruxiaflow import ScriptActivity

@ScriptActivity.from_function(worker="py-std")  # key defaults to function name
async def transform_data(INPUT):
    """Defines a SPECIFIC activity in a workflow."""
    result = process(INPUT["input"])
    OUTPUT = {"result": result}
```

## Motivation

Previously, script activities required embedding Python code as strings:

```python
Activity(
    key="process",
    worker="py-std",
    activity_name="script",
    parameters={
        "script": """
import pandas as pd
df = pd.DataFrame(INPUT["data"])
OUTPUT = {"result": df.sum().to_dict()}
"""
    }
)
```

**Problems with string-based scripts:**
- ❌ No syntax highlighting
- ❌ No code validation or linting
- ❌ No auto-formatting
- ❌ Difficult to refactor
- ❌ No IDE autocomplete
- ❌ Syntax errors only caught at runtime

## Solution

Two approaches enable function-based script definitions:

### 1. `@ScriptActivity.from_function()` Decorator (Recommended)

Most explicit and clear approach - creates Activity instances from functions:

```python
from kruxiaflow import ScriptActivity

@ScriptActivity.from_function(  # Key defaults to function name, worker defaults to "py-std"
    inputs={"data": previous["output"]},
    depends_on=["previous"],
)
async def process_data(data):
    import pandas as pd
    df = pd.DataFrame(data)
    return {"result": df.sum().to_dict()}

# process_data is now an Activity instance with key="process_data"
```

**Why this is best:**
- ✅ Explicit: `ScriptActivity` makes it clear you're creating a script activity
- ✅ Distinct: Clearly different from `worker.activity` (implementations vs instances)
- ✅ Consistent: Key defaults to function name (like `worker.activity(name=...)`)
- ✅ Discoverable: IDE shows all methods when exploring the class

### 2. `script()` Helper Function

For manual Activity construction when you need more control:

```python
from kruxiaflow import Activity, script

async def my_logic(INPUT):
    import pandas as pd
    df = pd.DataFrame(INPUT["data"])
    OUTPUT = {"result": df.sum().to_dict()}

activity = Activity(
    key="process",
    worker="py-std",
    activity_name="script",
    parameters={
        "inputs": {"data": previous["output"]},
        "script": script(my_logic),
    },
)
```

## Features

### ✅ Async/Await Support

Functions should be defined as `async def` to support async operations:

```python
@ScriptActivity.from_function(key="fetch", worker="py-std")
async def fetch_data(INPUT):
    import httpx

    async with httpx.AsyncClient() as client:
        response = await client.get(INPUT["url"])
        data = response.json()

    OUTPUT = {"data": data}
```

### ✅ IDE Benefits

- **Syntax highlighting**: Code is colored and formatted by your editor
- **Linting**: Tools like Ruff, Pylint, Mypy work on your code
- **Auto-formatting**: Black, autopep8, ruff format apply automatically
- **Autocomplete**: IDE suggests imports, functions, methods
- **Refactoring**: Rename variables, extract functions with IDE support
- **Type checking**: Optional type hints are validated

### ✅ Standard Script Variables

Functions have access to standard script activity variables:

- `INPUT`: Dictionary of input data
- `OUTPUT`: Dictionary to set (must be assigned in function)
- `ctx`: ActivityContext for heartbeat, file operations, logging
- `logger`: Logging instance (shortcut for ctx.logger)
- `workflow_id`: UUID of the current workflow
- `activity_key`: Key of this activity

### ✅ All Workers Supported

Works with all Python workers:

```python
@ScriptActivity.from_function(key="validate", worker="py-std")
async def validate_input(INPUT):
    from pydantic import BaseModel
    # ... validation logic

@ScriptActivity.from_function(key="transform", worker="py-std")
async def transform_data(INPUT):
    import pandas as pd
    import duckdb
    # ... ETL logic

@ScriptActivity.from_function(key="train", worker="py-ml")
async def train_model(INPUT):
    from sklearn.ensemble import RandomForestClassifier
    # ... ML logic

@ScriptActivity.from_function(key="analyze", worker="py-nlp")
async def analyze_text(INPUT):
    from transformers import pipeline
    # ... NLP logic
```

## Complete Example

```python
"""Data Processing Pipeline with ScriptActivity."""

from kruxiaflow import ScriptActivity, Workflow

# Key defaults to "fetch_data" (function name)
@ScriptActivity.from_function(
    worker="py-std",
    inputs={"url": "https://api.example.com/data"},
)
async def fetch_data(INPUT):
    import httpx

    async with httpx.AsyncClient() as client:
        response = await client.get(INPUT["url"])
        data = response.json()

    OUTPUT = {"records": data, "count": len(data)}


@ScriptActivity.from_function(
    key="transform_data",
    worker="py-std",
    inputs={"records": fetch_data["records"]},
    depends_on=["fetch_data"],
)
async def transform_data(INPUT):
    import pandas as pd
    import duckdb

    df = pd.DataFrame(INPUT["records"])

    result = duckdb.sql("""
        SELECT category, COUNT(*) as count, AVG(value) as avg
        FROM df
        GROUP BY category
    """).df()

    OUTPUT = {"summary": result.to_dict(orient="records")}


@ScriptActivity.from_function(
    key="predict",
    worker="py-ml",
    inputs={"summary": transform_data["summary"]},
    depends_on=["transform_data"],
)
async def predict(INPUT):
    import numpy as np
    from sklearn.ensemble import RandomForestClassifier

    # Feature engineering
    features = np.array([[s["count"], s["avg"]] for s in INPUT["summary"]])

    # Predict (using pre-trained model logic)
    predictions = [1 if f[1] > 50 else 0 for f in features]

    OUTPUT = {"predictions": predictions}


workflow = Workflow(
    name="data_pipeline",
    activities=[fetch_data, transform_data, predict],
)

if __name__ == "__main__":
    print(workflow)
```

## Generated YAML

The decorator generates standard script activity YAML:

```yaml
name: data_pipeline
activities:
- key: fetch_data
  worker: py-std
  activity_name: script
  parameters:
    script: |-
      import httpx

      async with httpx.AsyncClient() as client:
          response = await client.get(INPUT["url"])
          data = response.json()

      OUTPUT = {"records": data, "count": len(data)}
    inputs:
      url: https://api.example.com/data

- key: transform_data
  worker: py-data
  activity_name: script
  parameters:
    script: |-
      import pandas as pd
      import duckdb

      df = pd.DataFrame(INPUT["records"])

      result = duckdb.sql("""
          SELECT category, COUNT(*) as count, AVG(value) as avg
          FROM df
          GROUP BY category
      """).df()

      OUTPUT = {"summary": result.to_dict(orient="records")}
    inputs:
      records: '{{fetch_data.records}}'
  depends_on:
  - fetch_data

- key: predict
  worker: py-ml
  activity_name: script
  parameters:
    script: |-
      import numpy as np
      from sklearn.ensemble import RandomForestClassifier

      # Feature engineering
      features = np.array([[s["count"], s["avg"]] for s in INPUT["summary"]])

      # Predict (using pre-trained model logic)
      predictions = [1 if f[1] > 50 else 0 for f in features]

      OUTPUT = {"predictions": predictions}
    inputs:
      summary: '{{transform_data.summary}}'
  depends_on:
  - transform_data
```

## API Reference

### `ScriptActivity.from_function(key=None, worker="py-std", inputs=None, depends_on=None, **kwargs)`

Classmethod decorator to create a script Activity instance from a Python function.

This is the recommended way to define Python script activities. It creates Activity
**instances** for workflows (not activity implementations), with the activity key
defaulting to the function's `__name__`.

**Parameters:**
- `key` (str | None): Activity key. Defaults to function's `__name__`.
- `worker` (str): Worker name (default: "py-std"). Options: py-std, py-data, py-ml, py-nlp
- `inputs` (dict | None): Input data mapping (OutputRef expressions supported)
- `depends_on` (list[str] | None): List of activity keys this depends on
- `**kwargs`: Additional Activity parameters (settings, outputs, etc.)

**Returns:**
- Decorator that returns an `Activity` instance (not `ScriptActivity`)

**Raises:**
- `ValueError`: If key cannot be determined from function name

**Examples:**
```python
from kruxiaflow import ScriptActivity

# Default key (from function name)
@ScriptActivity.from_function(worker="py-std")
async def process_data(INPUT):  # key = "process_data"
    OUTPUT = {"result": ...}

# Explicit key
@ScriptActivity.from_function(
    key="process",
    worker="py-std",
    inputs={"data": prev["output"]},
    depends_on=["prev"],
)
async def process_and_validate(INPUT):  # key = "process" (overrides function name)
    OUTPUT = {"result": ...}

# Optional short alias
from kruxiaflow import ScriptActivity as Py

@Py.from_function(worker="py-std")
async def transform(INPUT):
    OUTPUT = {"result": ...}
```

---

### `script(func: callable) -> str`

Extracts the body of a Python function as a script string.

**Parameters:**
- `func`: Python function (preferably async) containing script logic

**Returns:**
- String containing the function body with dedented indentation

**Example:**
```python
async def my_func(INPUT):
    OUTPUT = {"result": INPUT["value"] * 2}

script_code = script(my_func)
# Returns: 'OUTPUT = {"result": INPUT["value"] * 2}'
```

## Best Practices

### ✅ Use async functions

Always define functions as `async def` for async/await support:

```python
@ScriptActivity.from_function(key="fetch", worker="py-std")
async def fetch(INPUT):  # ✅ async def
    async with httpx.AsyncClient() as client:
        await client.get(...)
```

### ✅ Import inside functions

Import modules inside the function body (not at module level):

```python
@ScriptActivity.from_function(key="process", worker="py-std")
async def process(INPUT):
    import pandas as pd  # ✅ Import inside function
    import duckdb
    # ... use imports
```

### ✅ Set OUTPUT explicitly

Always set `OUTPUT` as a dictionary:

```python
async def my_func(INPUT):
    result = compute(INPUT["data"])
    OUTPUT = {"result": result}  # ✅ Explicit OUTPUT
```

### ✅ Use descriptive function names

Function names help with code organization and refactoring:

```python
@ScriptActivity.from_function(key="validate_email", worker="py-std")
async def validate_email_format(INPUT):  # ✅ Descriptive name
    # ... validation logic
```

### ❌ Don't use external variables

The function is never executed; only its source is extracted:

```python
external_var = 42

@ScriptActivity.from_function(key="process", worker="py-std")
async def process(INPUT):
    OUTPUT = {"value": external_var}  # ❌ Won't work - external_var not available at runtime
```

### ❌ Don't call the function

The decorator handles Activity creation; don't call the function:

```python
@ScriptActivity.from_function(key="process", worker="py-std")
async def process(INPUT):
    OUTPUT = {"result": 42}

# ✅ Use directly
workflow = Workflow(activities=[process])

# ❌ Don't call
workflow = Workflow(activities=[process()])  # Wrong!
```

## Migration Guide

### Before (String-Based)

```python
Activity(
    key="analyze",
    worker="py-nlp",
    activity_name="script",
    parameters={
        "inputs": {"text": input_text},
        "script": """
from transformers import pipeline

analyzer = pipeline("sentiment-analysis")
result = analyzer(INPUT["text"])

OUTPUT = {
    "sentiment": result[0]["label"],
    "score": result[0]["score"],
}
""",
    },
    depends_on=["fetch_text"],
)
```

### After (Function-Based with Explicit Key)

```python
@ScriptActivity.from_function(
    key="analyze",  # Explicit key
    worker="py-nlp",
    inputs={"text": input_text},
    depends_on=["fetch_text"],
)
async def analyze_sentiment(INPUT):  # Function name differs from key
    from transformers import pipeline

    analyzer = pipeline("sentiment-analysis")
    result = analyzer(INPUT["text"])

    OUTPUT = {
        "sentiment": result[0]["label"],
        "score": result[0]["score"],
    }
```

### After (Function-Based with Default Key - Recommended)

```python
# Key defaults to function name: "analyze_sentiment"
@ScriptActivity.from_function(
    worker="py-nlp",
    inputs={"text": input_text},
    depends_on=["fetch_text"],
)
async def analyze_sentiment(INPUT):
    from transformers import pipeline

    analyzer = pipeline("sentiment-analysis")
    result = analyzer(INPUT["text"])

    OUTPUT = {
        "sentiment": result[0]["label"],
        "score": result[0]["score"],
    }
```

**Benefits:**
- ✅ Syntax highlighting on `pipeline`, `analyzer`, etc.
- ✅ Auto-complete for transformers API
- ✅ Linting catches typos like `pipline` → `pipeline`
- ✅ Auto-formatting applies (PEP 8 compliance)
- ✅ Easier to refactor and test logic separately
- ✅ Less boilerplate (key defaults to function name)

## Implementation Details

### Source Code Extraction

The `script()` function uses Python's `inspect.getsource()` to extract function source code, then:

1. Finds the function definition line (handles decorators)
2. Extracts the function body (everything after `def` line)
3. Removes leading indentation with `textwrap.dedent()`
4. Returns the cleaned body as a string

### Function Signature Ignored

Only the function **body** is extracted. The signature is for documentation:

```python
async def my_func(INPUT, ctx, workflow_id):  # Signature ignored
    # Only this body is extracted
    OUTPUT = {"result": 42}
```

Parameters like `INPUT`, `ctx`, `workflow_id` are available at runtime regardless of signature.

## See Also

- [Standard Workers](./standard-workers.md) - Pre-built Python workers
- [Custom Workers](./custom-workers.md) - Building custom activity types
- [Python SDK Guide](./README.md) - Complete SDK documentation
