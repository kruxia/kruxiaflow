# Standard Python Workers Guide

Pre-built Python workers for zero-setup script execution with domain-specific packages.

## Overview

Standard Python workers provide ready-to-use Docker images with pre-installed packages for common use cases. Each worker includes a `script` activity that executes arbitrary Python code.

> **Context**: This guide covers two aspects:
> 1. **Workflow definitions** - How to reference standard workers and write scripts (runs at deploy time)
> 2. **Worker deployment** - How to run the Docker containers (ops/infrastructure)
>
> See the [Quick Start](quickstart.md#architecture-overview) for an architecture diagram.

## Available Workers

| Worker | Image | Size (raw/compressed) | Focus |
|--------|-------|----------------------|-------|
| `py-std` | `ghcr.io/kruxia/kruxiaflow-worker-py-std` | 240MB / 50MB | Universal utilities |
| `py-data` | `ghcr.io/kruxia/kruxiaflow-worker-py-data` | 1.3GB / 310MB | ETL/transformation |
| `py-ml` | `ghcr.io/kruxia/kruxiaflow-worker-py-ml` | 2.0GB / 435MB | Machine learning |
| `py-nlp` | `ghcr.io/kruxia/kruxiaflow-worker-py-nlp` | 2.4GB / 515MB | NLP/text processing |

## Pre-installed Packages

### py-std (Universal Utilities)

```
pydantic>=2.0      # Data validation
httpx>=0.24        # HTTP client
pyyaml>=6.0        # YAML parsing
orjson>=3.9        # Fast JSON
python-dateutil>=2.8  # Date utilities
```

### py-data (ETL/Transformation)

Includes all `py-std` packages plus:

```
pandas>=2.2        # DataFrames
polars>=0.20       # Fast DataFrames
pyarrow>=15.0      # Parquet/Arrow
duckdb>=0.10       # In-process SQL
sqlalchemy>=2.0    # Database interface
```

### py-ml (Machine Learning)

Includes all `py-std` packages plus:

```
numpy>=1.26        # Numerical computing
pandas>=2.2        # DataFrames
scikit-learn>=1.4  # ML algorithms
scipy>=1.12        # Scientific computing
torch>=2.2         # Deep learning (CPU)
```

### py-nlp (Natural Language Processing)

Includes all `py-std` packages plus:

```
numpy>=1.26                 # Numerical computing
transformers>=4.38          # Hugging Face models
sentence-transformers>=2.3  # Embeddings
tiktoken>=0.6               # Token counting
spacy>=3.7                  # NLP pipeline
```

The `en_core_web_sm` spacy model is pre-downloaded. Below ("[Model Caching](#model-caching)") are instructions for caching other NLP models in infrastructure for use in activities.

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
from kruxiaflow import Activity, Workflow

transform = (
    Activity(key="transform")
    .with_worker("py-data", "script")  # ← References the py-data worker
    .with_params(
        inputs={"records": fetch["response.json"]},
        # The script string runs ON THE WORKER at execution time
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

### Data Transformation (py-data)

```yaml
- key: transform_data
  worker: py-data
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

### ML Inference (py-ml)

```yaml
- key: predict
  worker: py-ml
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

### Text Embeddings (py-nlp)

```yaml
- key: embed_texts
  worker: py-nlp
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

### Sentiment Analysis (py-nlp)

```yaml
- key: analyze_sentiment
  worker: py-nlp
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
  py-nlp-worker:
    image: ghcr.io/kruxia/kruxiaflow-worker-py-nlp:latest
    environment:
      KRUXIAFLOW_API_URL: http://api:8080
      KRUXIAFLOW_CLIENT_ID: nlp-worker
      KRUXIAFLOW_CLIENT_SECRET: ${WORKER_SECRET}
      # Point all caches to /cache
      HF_HOME: /cache/huggingface
      TRANSFORMERS_CACHE: /cache/huggingface
      SENTENCE_TRANSFORMERS_HOME: /cache/sentence-transformers
      TIKTOKEN_CACHE_DIR: /cache/tiktoken
    volumes:
      - model-cache:/cache
    deploy:
      replicas: 2

  py-ml-worker:
    image: ghcr.io/kruxia/kruxiaflow-worker-py-ml:latest
    environment:
      KRUXIAFLOW_API_URL: http://api:8080
      KRUXIAFLOW_CLIENT_ID: ml-worker
      KRUXIAFLOW_CLIENT_SECRET: ${WORKER_SECRET}
    volumes:
      - model-cache:/cache
    deploy:
      replicas: 2

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
  name: py-nlp-worker
spec:
  replicas: 3
  selector:
    matchLabels:
      app: py-nlp-worker
  template:
    metadata:
      labels:
        app: py-nlp-worker
    spec:
      containers:
        - name: worker
          image: ghcr.io/kruxia/kruxiaflow-worker-py-nlp:latest
          env:
            - name: KRUXIAFLOW_API_URL
              value: "http://kruxiaflow-api:8080"
            - name: KRUXIAFLOW_CLIENT_ID
              value: "nlp-worker"
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
          image: ghcr.io/kruxia/kruxiaflow-worker-py-nlp:latest
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

## Choosing the Right Worker

| Use Case | Recommended Worker |
|----------|-------------------|
| API calls, JSON processing, simple transforms | `py-std` |
| DataFrames, SQL, ETL pipelines, Parquet files | `py-data` |
| Model training, inference, numerical computing | `py-ml` |
| Text embeddings, sentiment analysis, NLP pipelines | `py-nlp` |

## When to Use Standard vs. Custom Workers

Use standard workers for:
- Ad-hoc data transformations
- Scripts using only pre-installed packages
- Prototyping and quick iterations
- Simple scripts

Use [custom workers](custom-workers.md) when you need:
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
  -e KRUXIAFLOW_CLIENT_ID=py-data-worker \
  -e KRUXIAFLOW_CLIENT_SECRET=your_secret \
  -v model-cache:/cache \
  ghcr.io/kruxia/kruxiaflow-worker-py-data:latest
```

### Scaling

Scale workers based on workload:

```bash
# Docker Compose
docker compose up -d --scale py-data-worker=5

# Kubernetes
kubectl scale deployment py-data-worker --replicas=5
```

Workers automatically:
- Poll for available activities
- Execute up to `KRUXIAFLOW_WORKER_MAX_ACTIVITIES` concurrently (default: 16)
- Handle graceful shutdown on SIGTERM
