# Standard Python Worker Guide

Pre-built comprehensive Python worker for zero-setup script execution covering 80-90% of data engineering, ML, and NLP use cases.

## Overview

The standard Python worker (`py-std`) provides a ready-to-use Docker image with pre-installed packages for common data processing, machine learning, and NLP tasks. It includes a `script` activity that executes arbitrary Python code.

> **Context**: This guide covers two aspects:
> 1. **Workflow definitions** - How to reference the worker and write scripts (runs at deploy time)
> 2. **Worker deployment** - How to run the Docker container (ops/infrastructure)
>
> See the [Quick Start](quickstart.md#architecture-overview) for an architecture diagram.

## The py-std Worker

| Worker | Image | Size (raw/compressed) | Coverage |
|--------|-------|----------------------|----------|
| `py-std` | `kruxia/kruxiaflow-py-std` | 1.25GB / 320-350MB | Data engineering, ML, NLP |

## Pre-installed Packages

The `py-std` worker includes a comprehensive set of packages covering most common use cases:

### Core Utilities
```
pydantic>=2.0           # Data validation
httpx>=0.24             # HTTP client
pyyaml>=6.0             # YAML parsing
orjson>=3.9             # Fast JSON
python-dateutil>=2.8    # Date utilities
```

### Data Processing
```
pandas>=2.2             # DataFrames (standard library)
polars>=0.20            # Fast DataFrames (alternative)
pyarrow>=15.0           # Parquet/Arrow format
duckdb>=0.10            # In-process SQL analytics
numpy>=1.26             # Numerical computing
```

### Machine Learning
```
scikit-learn>=1.4       # Traditional ML algorithms
```

### Natural Language Processing
```
transformers>=4.38          # Hugging Face models
sentence-transformers>=2.3  # Semantic embeddings
tiktoken>=0.6               # Token counting for LLMs
```

## Custom Workers for Specialized Needs

The `py-std` worker covers 80-90% of use cases. For specialized needs, the community can contribute custom worker definitions:

- **PyTorch Training**: Add `torch>=2.2` for deep learning model training
- **Spacy NER**: Add `spacy>=3.7` with language models for advanced NLP
- **Scientific Computing**: Add `scipy>=1.12` for specialized algorithms
- **Geospatial**: Add `geopandas`, `shapely` for GIS processing

See [custom workers](custom-workers.md) for details on building specialized workers.

## Using the Script Activity

The `script` activity executes Python code with access to pre-installed packages.

### Basic Usage

The workflow definition specifies which worker to use and provides the script:

**Workflow Definition** (YAML)

```yaml
activities:
  process:
    worker: py-std          # ← Which worker container runs this
    activity_name: script   # ← The script activity type
    parameters:
      inputs:
        data: "{{INPUT.data}}"
      script: |             # ← Python code that runs ON THE WORKER
        import orjson

        # INPUT contains the inputs dict
        data = INPUT["data"]

        # Process data
        result = {"processed": len(data)}

        # Set OUTPUT to return results
        OUTPUT = result
```

The `script` parameter contains Python code that executes **on the worker container** at runtime, not at workflow definition time.

### Available Variables

Inside the script, these variables are available:

| Variable | Description |
|----------|-------------|
| `INPUT` | Dict containing activity inputs |
| `OUTPUT` | Dict to set activity outputs (assign to this) |
| `ctx` | ActivityContext for heartbeat, file ops |
| `logger` | Logger instance |
| `workflow_id` | Current workflow ID |
| `activity_key` | Current activity key |

### Python SDK

**File: `my_workflow.py`** (Workflow Definition)

```python
from kruxiaflow import ScriptActivity, Workflow

# Using ScriptActivity.from_function (recommended)
@ScriptActivity.from_function(
    inputs={"records": fetch["response.json"]},
    depends_on=["fetch"],
)
async def transform(records):
    import pandas as pd
    import duckdb

    df = pd.DataFrame(records)
    result = duckdb.sql("SELECT * FROM df WHERE value > 100").df()

    return {"rows": len(result), "data": result.to_dict()}

# Or using Activity with script parameter (if needed)
from kruxiaflow import Activity

transform_alt = (
    Activity(key="transform_alt")
    .with_worker("py-std", "script")  # ← worker defaults to py-std
    .with_params(
        inputs={"records": fetch["response.json"]},
        script="""
import pandas as pd
import duckdb

df = pd.DataFrame(INPUT["records"])
result = duckdb.sql("SELECT * FROM df WHERE value > 100").df()

OUTPUT = {"rows": len(result), "data": result.to_dict()}
""",
    )
    .with_dependencies(fetch)
)
```

Note: The outer Python code (importing `Activity`, calling `.with_worker()`) runs at **definition time** to build the workflow. The `script` string runs on the **worker container** at execution time.

## Use Case Examples

### Data Transformation

```yaml
- key: transform_data
  worker: py-std
  activity_name: script
  parameters:
    inputs:
      records: "{{fetch_data.response.json}}"
    script: |
      import pandas as pd
      import duckdb

      df = pd.DataFrame(INPUT["records"])

      # Clean data
      df_clean = df.dropna().drop_duplicates()

      # SQL transformation
      result = duckdb.sql("""
          SELECT category, SUM(amount) as total
          FROM df_clean
          GROUP BY category
          ORDER BY total DESC
      """).df()

      OUTPUT = {
          "summary": result.to_dict(orient="records"),
          "row_count": len(result),
      }
```

### ML Inference

```yaml
- key: predict
  worker: py-std
  activity_name: script
  parameters:
    inputs:
      features: "{{transform.result.features}}"
    script: |
      import numpy as np
      from sklearn.ensemble import RandomForestClassifier
      import pickle

      # Load pre-trained model (from previous activity or storage)
      # model = pickle.loads(...)

      # For demo, create simple model
      X = np.array(INPUT["features"])

      # Make predictions
      # predictions = model.predict(X)

      OUTPUT = {
          "predictions": X.tolist(),  # placeholder
          "count": len(X),
      }
```

### Text Embeddings

```yaml
- key: embed_texts
  worker: py-std
  activity_name: script
  parameters:
    inputs:
      texts: "{{INPUT.texts}}"
    script: |
      from sentence_transformers import SentenceTransformer

      model = SentenceTransformer("all-MiniLM-L6-v2")
      embeddings = model.encode(INPUT["texts"])

      OUTPUT = {
          "embeddings": embeddings.tolist(),
          "dimensions": embeddings.shape[1],
      }
```

### Sentiment Analysis

```yaml
- key: analyze_sentiment
  worker: py-std
  activity_name: script
  parameters:
    inputs:
      texts: "{{INPUT.texts}}"
    script: |
      from transformers import pipeline

      classifier = pipeline("sentiment-analysis")
      results = classifier(INPUT["texts"])

      OUTPUT = {"results": results}
```

## Model Caching

ML/NLP workers download models on first use, which can be slow. Use volume mounts to cache models across container restarts.

### Cache Directories

| Library | Environment Variable | Default Path |
|---------|---------------------|--------------|
| Hugging Face | `HF_HOME` | `~/.cache/huggingface` |
| Transformers | `TRANSFORMERS_CACHE` | `~/.cache/huggingface` |
| Sentence Transformers | `SENTENCE_TRANSFORMERS_HOME` | `~/.cache/torch/sentence_transformers` |
| spacy | `SPACY_DATA` | Platform-specific |
| tiktoken | `TIKTOKEN_CACHE_DIR` | `~/.cache/tiktoken` |

### Docker Compose

```yaml
version: "3.8"

services:
  py-std-worker:
    image: kruxia/kruxiaflow-py-std:latest
    environment:
      KRUXIAFLOW_API_URL: http://api:8080
      KRUXIAFLOW_CLIENT_ID: py-std-worker
      KRUXIAFLOW_CLIENT_SECRET: ${WORKER_SECRET}
      # Point all caches to /cache
      HF_HOME: /cache/huggingface
      TRANSFORMERS_CACHE: /cache/huggingface
      SENTENCE_TRANSFORMERS_HOME: /cache/sentence-transformers
      TIKTOKEN_CACHE_DIR: /cache/tiktoken
    volumes:
      - model-cache:/cache
    deploy:
      replicas: 3

volumes:
  model-cache:
    driver: local
```

### Kubernetes

```yaml
apiVersion: v1
kind: PersistentVolumeClaim
metadata:
  name: model-cache-pvc
spec:
  accessModes:
    - ReadWriteMany  # Multiple pods can share
  resources:
    requests:
      storage: 50Gi
  storageClassName: fast-storage  # Use appropriate storage class
---
apiVersion: apps/v1
kind: Deployment
metadata:
  name: py-std-worker
spec:
  replicas: 3
  selector:
    matchLabels:
      app: py-std-worker
  template:
    metadata:
      labels:
        app: py-std-worker
    spec:
      containers:
        - name: worker
          image: kruxia/kruxiaflow-py-std:latest
          env:
            - name: KRUXIAFLOW_API_URL
              value: "http://kruxiaflow-api:8080"
            - name: KRUXIAFLOW_CLIENT_ID
              value: "py-std-worker"
            - name: KRUXIAFLOW_CLIENT_SECRET
              valueFrom:
                secretKeyRef:
                  name: worker-secrets
                  key: client-secret
            # Cache configuration
            - name: HF_HOME
              value: "/cache/huggingface"
            - name: TRANSFORMERS_CACHE
              value: "/cache/huggingface"
            - name: SENTENCE_TRANSFORMERS_HOME
              value: "/cache/sentence-transformers"
            - name: TIKTOKEN_CACHE_DIR
              value: "/cache/tiktoken"
          volumeMounts:
            - name: model-cache
              mountPath: /cache
          resources:
            requests:
              memory: "2Gi"
              cpu: "500m"
            limits:
              memory: "8Gi"
              cpu: "4"
      volumes:
        - name: model-cache
          persistentVolumeClaim:
            claimName: model-cache-pvc
```

### Pre-warming the Cache

To download models before workflow execution, run a one-time job:

```yaml
# kubernetes pre-warm job
apiVersion: batch/v1
kind: Job
metadata:
  name: prewarm-models
spec:
  template:
    spec:
      containers:
        - name: prewarm
          image: kruxia/kruxiaflow-py-std:latest
          command: ["python", "-c"]
          args:
            - |
              from sentence_transformers import SentenceTransformer
              from transformers import pipeline

              print("Downloading sentence-transformers models...")
              SentenceTransformer("all-MiniLM-L6-v2")
              SentenceTransformer("all-mpnet-base-v2")

              print("Downloading transformers models...")
              pipeline("sentiment-analysis")
              pipeline("summarization", model="facebook/bart-large-cnn")

              print("Done!")
          env:
            - name: HF_HOME
              value: "/cache/huggingface"
            - name: SENTENCE_TRANSFORMERS_HOME
              value: "/cache/sentence-transformers"
          volumeMounts:
            - name: model-cache
              mountPath: /cache
      restartPolicy: Never
      volumes:
        - name: model-cache
          persistentVolumeClaim:
            claimName: model-cache-pvc
  backoffLimit: 2
```

## Use Cases

The `py-std` worker covers:

| Use Case | Capabilities |
|----------|-------------|
| **Data Engineering** | DataFrames (pandas/polars), SQL (DuckDB), Parquet (PyArrow) |
| **Machine Learning** | scikit-learn for classification, regression, clustering |
| **NLP** | Text embeddings, sentiment analysis, transformers |
| **API Integration** | HTTP clients, JSON processing, data validation |

## When to Use Standard vs. Custom Workers

Use the `py-std` worker for:
- Data transformation and ETL pipelines
- Traditional machine learning (scikit-learn)
- Text embeddings and sentiment analysis
- Scripts using pre-installed packages
- Prototyping and quick iterations

Build [custom workers](custom-workers.md) when you need:
- PyTorch for deep learning training
- Spacy for advanced NER
- Custom or proprietary packages
- Type-safe parameters with validation
- Model caching across activity calls
- Fine-grained heartbeat control
- Versioned activity releases
- Automated testing and software engineering lifecycle

## Deployment

### Running Locally

```bash
docker run -d \
  -e KRUXIAFLOW_API_URL=http://host.docker.internal:8080 \
  -e KRUXIAFLOW_CLIENT_ID=py-std-worker \
  -e KRUXIAFLOW_CLIENT_SECRET=your_secret \
  -v model-cache:/cache \
  kruxia/kruxiaflow-py-std:latest
```

### Scaling

Scale workers based on workload:

```bash
# Docker Compose
docker compose up -d --scale py-std-worker=5

# Kubernetes
kubectl scale deployment py-std-worker --replicas=5
```

Workers automatically:
- Poll for available activities
- Execute up to `KRUXIAFLOW_WORKER_MAX_ACTIVITIES` concurrently (default: 16)
- Handle graceful shutdown on SIGTERM
