# Script Function Syntax

## Overview

ScriptActivity functions use Pythonic conventions where:
- **Function parameters** are automatically extracted from the INPUT dict
- **Return values** become the OUTPUT dict

This provides full IDE support with syntax highlighting, linting, and type hints.

## Basic Pattern

```python
from kruxiaflow import ScriptActivity

@ScriptActivity.from_function()  # worker defaults to "py-std"
async def process_data(records, threshold=0.5):
    """Process data with parameters.

    Args:
        records: List of records to process
        threshold: Minimum threshold value (default: 0.5)

    Returns:
        dict with processing results
    """
    import pandas as pd

    df = pd.DataFrame(records)
    filtered = df[df['value'] > threshold]

    return {
        "count": len(filtered),
        "total": filtered['value'].sum(),
    }
```

## Generated Script

The above function generates a script that:

1. Defines the function
2. Extracts parameters from INPUT dict
3. Calls the function with extracted parameters
4. Assigns the return value to OUTPUT

```python
async def process_data(records, threshold=0.5):
    import pandas as pd

    df = pd.DataFrame(records)
    filtered = df[df['value'] > threshold]

    return {
        "count": len(filtered),
        "total": filtered['value'].sum(),
    }

# Extract parameters from INPUT
records = INPUT.get('records')
threshold = INPUT.get('threshold')

# Call function and assign result to OUTPUT
OUTPUT = await process_data(records, threshold)
```

## Passing Inputs

Use the `inputs` parameter to map upstream outputs to function parameters:

```python
@ScriptActivity.from_function(
    worker="py-std",
    inputs={"records": load_activity["data"]},
    depends_on=["load_activity"],
)
async def transform(records):
    return {"transformed": [r * 2 for r in records]}
```

## No Parameters

Functions without parameters work too:

```python
@ScriptActivity.from_function(worker="py-std")
async def get_timestamp():
    from datetime import datetime
    return {"timestamp": datetime.now().isoformat()}
```

## Comparison with Old Pattern

### ❌ Old Pattern (Deprecated)
```python
async def my_function(INPUT):
    value = INPUT["key"]
    result = process(value)
    OUTPUT = {"result": result}
```

### ✅ New Pattern
```python
async def my_function(key):
    result = process(key)
    return {"result": result}
```

## Benefits

1. **Type Hints**: Function parameters can have type annotations
2. **IDE Support**: Full autocomplete and linting
3. **Testability**: Functions can be unit tested directly
4. **Readability**: Clear function signatures
5. **Pythonic**: Follows standard Python conventions
