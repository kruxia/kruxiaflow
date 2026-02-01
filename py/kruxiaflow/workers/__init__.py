"""Standard Python workers for Kruxia Flow.

This module provides pre-built workers with domain-specific packages
for executing Python scripts in Kruxia Flow workflows.

Workers (all use "py-" prefix):
    - py-std:  Universal utilities (httpx, orjson, pydantic)
    - py-data: ETL/transformation (pandas, polars, duckdb)
    - py-ml:   Training/inference (sklearn, torch, numpy)
    - py-nlp:  Text processing (transformers, spacy)

All workers share the same `script` activity - they differ only in
which packages are pre-installed.
"""

from .script_activity import script_activity

__all__ = ["script_activity"]
