"""Kruxia Flow Python SDK.

A fluent Python API for building and deploying workflows to Kruxia Flow.

Example:
    from kruxiaflow import (
        KruxiaFlow, Workflow, Activity, Input, Dependency, workflow
    )

    # Define inputs
    user_text = Input("text", type=str, required=True)

    # Build activity definitions with fluent API
    analyze = (
        Activity(key="analyze_sentiment")
        .with_worker("std", "llm_prompt")
        .with_params(
            provider="anthropic",
            model="claude-3-haiku-20240307",
            prompt=f"Analyze sentiment: {user_text}",
        )
        .with_cache(ttl=3600)
    )

    # Use expression conditions for dependencies
    save = (
        Activity(key="save_results")
        .with_worker("std", "postgres_query")
        .with_params(
            query="INSERT INTO results VALUES ($1, $2)",
            params=[user_text, analyze["sentiment"]],
        )
        .with_dependencies(
            Dependency.on(analyze,
                (analyze["confidence"] > 0.8) & (analyze["status"] == "success")
            )
        )
    )

    notify = (
        Activity(key="notify")
        .with_worker("std", "http_request")
        .with_params(
            method="POST",
            body={"workflow_id": workflow.id, "result": analyze["sentiment"]},
        )
        .with_dependencies(save)
    )

    # Build workflow
    wf = (
        Workflow(name="sentiment_analysis")
        .with_version("1.0.0")
        .with_inputs(user_text)
        .with_activities(analyze, save, notify)
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
from .expressions import (
    # Logical types
    And,
    # Core expression types
    Comparison,
    # Utility types
    Contains,
    EnvRef,
    # Comparison types
    Eq,
    Expression,
    Ge,
    Gt,
    In,
    Input,
    IsNotNull,
    IsNull,
    Le,
    Literal,
    Lt,
    Ne,
    Not,
    Or,
    # Backward compatibility
    OutputComparison,
    OutputRef,
    SecretRef,
    WorkflowRef,
    # Helper functions
    and_,
    contains,
    in_,
    is_not_null,
    is_null,
    not_,
    or_,
    # Workflow metadata accessor
    workflow,
)
from .models import (
    Activity,
    ActivityOutputDefinition,
    ActivitySettings,
    BackoffStrategy,
    BudgetAction,
    BudgetSettings,
    CacheSettings,
    Dependency,
    OutputType,
    RetrySettings,
    ScriptActivity,
    Workflow,
    script,
)

try:
    __version__ = version("kruxiaflow")
except PackageNotFoundError:
    __version__ = "0.0.0"

__all__ = [
    # Models
    "Activity",
    "ActivityOutputDefinition",
    "ActivitySettings",
    # Expression types
    "And",
    # Client
    "AsyncKruxiaFlow",
    "AuthenticationError",
    "BackoffStrategy",
    "BudgetAction",
    "BudgetSettings",
    "CacheSettings",
    "Comparison",
    "Contains",
    "Dependency",
    "DeploymentError",
    "EnvRef",
    "Eq",
    "Expression",
    "Ge",
    "Gt",
    "In",
    "Input",
    "IsNotNull",
    "IsNull",
    "KruxiaFlow",
    "KruxiaFlowError",
    "Le",
    "Literal",
    "Lt",
    "Ne",
    "Not",
    "Or",
    "OutputComparison",  # Backward compatibility
    "OutputRef",
    "OutputType",
    "RetrySettings",
    "ScriptActivity",
    "SecretRef",
    "Workflow",
    "WorkflowNotFoundError",
    "WorkflowRef",
    # Version
    "__version__",
    # Helper functions
    "and_",
    "contains",
    "in_",
    "is_not_null",
    "is_null",
    "not_",
    "or_",
    # Script utilities
    "script",
    # Workflow metadata accessor
    "workflow",
]
