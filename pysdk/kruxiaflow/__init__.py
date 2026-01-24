"""Kruxia Flow Python SDK.

A fluent Python API for building and deploying workflows to Kruxia Flow.

Example:
    from kruxiaflow import KruxiaFlow, Workflow, Activity, Input, Dependency

    # Define inputs
    user_text = Input("text", type=str, required=True)

    # Build activity definitions with fluent API
    analyze = (
        Activity(key="analyze_sentiment")
        .with_worker("builtin", "llm_prompt")
        .with_params(
            provider="anthropic",
            model="claude-3-haiku-20240307",
            prompt=f"Analyze sentiment: {user_text}",
        )
        .with_cache(ttl=3600)
    )

    save = (
        Activity(key="save_results")
        .with_worker("builtin", "postgres_query")
        .with_params(
            query="INSERT INTO results VALUES ($1, $2)",
            params=[user_text, analyze["sentiment"]],
        )
        .with_dependencies(analyze)
    )

    # Build workflow
    wf = (
        Workflow(name="sentiment_analysis")
        .with_version("1.0.0")
        .with_inputs(user_text)
        .with_activities(analyze, save)
    )

    # Deploy to server
    client = KruxiaFlow(api_url="http://localhost:8080")
    client.deploy(wf)
"""

from importlib.metadata import PackageNotFoundError, version

from .client import (
    AsyncKruxiaFlow,
    AuthenticationError,
    DeploymentError,
    KruxiaFlow,
    KruxiaFlowError,
    WorkflowNotFoundError,
)
from .expressions import EnvRef, Input, OutputComparison, OutputRef, SecretRef
from .models import (
    Activity,
    ActivitySettings,
    BudgetSettings,
    CacheSettings,
    Dependency,
    InputSchema,
    RetrySettings,
    Workflow,
)

try:
    __version__ = version("kruxiaflow")
except PackageNotFoundError:
    __version__ = "0.0.0"

__all__ = [
    "Activity",
    "ActivitySettings",
    "AsyncKruxiaFlow",
    "AuthenticationError",
    "BudgetSettings",
    "CacheSettings",
    "Dependency",
    "DeploymentError",
    "EnvRef",
    "Input",
    "InputSchema",
    "KruxiaFlow",
    "KruxiaFlowError",
    "OutputComparison",
    "OutputRef",
    "RetrySettings",
    "SecretRef",
    "Workflow",
    "WorkflowNotFoundError",
    "__version__",
]
