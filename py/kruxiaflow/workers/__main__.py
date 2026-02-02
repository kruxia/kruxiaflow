"""Standard Python worker entry point.

Usage:
    python -m kruxiaflow.workers

    Or via the console script:
    kruxiaflow-worker

Environment variables:
    KRUXIAFLOW_API_URL: API server URL (required)
    KRUXIAFLOW_CLIENT_ID: OAuth client ID (required)
    KRUXIAFLOW_CLIENT_SECRET: OAuth client secret (required)
    KRUXIAFLOW_WORKER: Worker type (default: from WORKER_TYPE or "python")
    WORKER_TYPE: Alternative env var for worker type (used in Docker images)

Worker types use "py-" prefix to indicate Python-based workers:
    - py-std:  Universal utilities (httpx, orjson, pydantic)
    - py-data: ETL/transformation (pandas, polars, duckdb)
    - py-ml:   Training/inference (sklearn, torch, numpy)
    - py-nlp:  Text processing (transformers, spacy)

The worker type determines which activities are advertised to the orchestrator.
All standard workers use the same `script` activity - they differ only in
which packages are available at runtime.
"""

import asyncio
import logging
import os
import sys

from kruxiaflow.worker import ActivityRegistry, WorkerConfig, WorkerManager

from .script_activity import script_activity

logger = logging.getLogger(__name__)


def main() -> None:
    """Run the standard Python worker."""
    # Configure logging
    logging.basicConfig(
        level=logging.INFO,
        format="%(asctime)s [%(levelname)s] %(name)s: %(message)s",
        stream=sys.stderr,
    )

    # Suppress httpx request logging (polls every 100ms at INFO level)
    logging.getLogger("httpx").setLevel(logging.WARNING)
    logging.getLogger("httpcore").setLevel(logging.WARNING)

    # Load config from environment
    try:
        config = WorkerConfig()  # type: ignore[call-arg]  # pydantic-settings loads from env
    except Exception as e:
        logger.error(f"Failed to load worker config: {e}")
        logger.error(
            "Required environment variables: "
            "KRUXIAFLOW_API_URL, KRUXIAFLOW_CLIENT_ID, KRUXIAFLOW_CLIENT_SECRET"
        )
        sys.exit(1)

    # Worker type can be overridden by WORKER_TYPE env var (set in Docker images)
    # This allows the same code to run as py-std, py-data, py-ml, or py-nlp worker
    worker_type = os.environ.get("WORKER_TYPE") or config.worker or "py-std"
    config.worker = worker_type

    logger.info(f"Starting {worker_type} worker")

    # Create registry and register script activity
    registry = ActivityRegistry()
    registry.register(script_activity, worker_type)

    # Run worker
    manager = WorkerManager(config, registry)

    try:
        asyncio.run(manager.run_until_shutdown())
    except KeyboardInterrupt:
        logger.info("Worker interrupted")
    except Exception as e:
        logger.error(f"Worker failed: {e}")
        sys.exit(1)

    logger.info("Worker stopped")


if __name__ == "__main__":
    main()
