# Python Workers - Multi-stage Dockerfile
#
# Build targets (uncompressed / compressed):
#   - py-std:  Universal utilities (~240MB / ~50MB)
#   - py-data: ETL/transformation (~1.3GB / ~310MB)
#   - py-ml:   Machine learning (~2.0GB / ~435MB)
#   - py-nlp:  Natural language processing (~2.4GB / ~515MB)
#
# Usage (from repo root):
#   docker build -f py/Dockerfile --target py-std -t ghcr.io/kruxia/kruxiaflow-worker-py-std py/
#   docker build -f py/Dockerfile --target py-data -t ghcr.io/kruxia/kruxiaflow-worker-py-data py/
#   docker build -f py/Dockerfile --target py-ml -t ghcr.io/kruxia/kruxiaflow-worker-py-ml py/
#   docker build -f py/Dockerfile --target py-nlp -t ghcr.io/kruxia/kruxiaflow-worker-py-nlp py/

# =============================================================================
# Base stage - common setup for all Python workers
# =============================================================================
FROM python:3.12-slim AS base

WORKDIR /app

# Common environment
ENV PYTHONUNBUFFERED=1
ENV PIP_NO_CACHE_DIR=1
ENV PIP_DISABLE_PIP_VERSION_CHECK=1

# Copy package source
COPY pyproject.toml README.md ./
COPY kruxiaflow/ ./kruxiaflow/

# =============================================================================
# py-std: Universal utilities
# Packages: pydantic, httpx, pyyaml, orjson, python-dateutil
# =============================================================================
FROM base AS py-std

RUN pip install ".[std]"

ENV WORKER_TYPE=py-std
CMD ["kruxiaflow-worker"]

# =============================================================================
# Base build stage - adds build dependencies for compiled packages
# =============================================================================
FROM base AS base-build

RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential \
    && rm -rf /var/lib/apt/lists/*

# =============================================================================
# py-data: ETL and transformation
# Packages: pandas, polars, pyarrow, duckdb, sqlalchemy + std
# =============================================================================
FROM base-build AS py-data

RUN pip install ".[data]"

ENV WORKER_TYPE=py-data
CMD ["kruxiaflow-worker"]

# =============================================================================
# py-ml: Machine learning
# Packages: numpy, pandas, scikit-learn, scipy, torch (CPU) + std
# =============================================================================
FROM base-build AS py-ml

RUN pip install ".[ml]"

ENV WORKER_TYPE=py-ml
CMD ["kruxiaflow-worker"]

# =============================================================================
# py-nlp: Natural language processing
# Packages: numpy, transformers, sentence-transformers, tiktoken, spacy + std
# =============================================================================
FROM base-build AS py-nlp

RUN pip install ".[nlp]"

# Download small spacy model
RUN python -m spacy download en_core_web_sm

ENV WORKER_TYPE=py-nlp
CMD ["kruxiaflow-worker"]
