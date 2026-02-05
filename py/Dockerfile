# Python Worker - Single comprehensive worker for 80-90% of use cases
#
# Includes: pandas, polars, numpy, scikit-learn, duckdb, pyarrow,
#           sentence-transformers, tiktoken, transformers, and core utilities
#
# Image size (uncompressed / compressed): ~1.25GB / ~320-350MB
#
# Usage (from repo root):
#   docker build -f py/Dockerfile -t kruxia/kruxiaflow-py-std py/
#
# For specialized needs (PyTorch training, spacy NER, etc.),
# community can contribute custom worker definitions.

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
# py-std: Comprehensive Python worker
# Includes: pandas, polars, numpy, scikit-learn, duckdb, pyarrow,
#           sentence-transformers, tiktoken, transformers, and core utilities
# =============================================================================
FROM base AS base-build

# Add build dependencies for compiled packages (numpy, scikit-learn, etc.)
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential \
    && rm -rf /var/lib/apt/lists/*

FROM base-build AS py-std

RUN pip install ".[std]"

ENV WORKER_TYPE=py-std
CMD ["kruxiaflow-worker"]
