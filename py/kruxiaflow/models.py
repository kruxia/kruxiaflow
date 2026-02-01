"""Pydantic models for workflow definitions with fluent builder methods.

This module provides the primary interface for constructing workflows programmatically.
Models can be constructed declaratively (by passing arguments) or using fluent method chaining.
"""

from __future__ import annotations

import sys
from enum import Enum
from typing import TYPE_CHECKING, Any

import yaml
from pydantic import BaseModel, ConfigDict, Field

from .types import ActivityKey, ActivityNameOptional, WorkerName


class BackoffStrategy(str, Enum):
    """Retry backoff strategy."""

    EXPONENTIAL = "exponential"
    FIXED = "fixed"


class BudgetAction(str, Enum):
    """Action to take when budget is exceeded."""

    ABORT = "abort"
    CONTINUE = "continue"


if sys.version_info >= (3, 11):
    from typing import Self
else:
    from typing_extensions import Self

if TYPE_CHECKING:
    from .expressions import Input, OutputComparison, OutputRef


# =============================================================================
# Settings Models
# =============================================================================


class RetrySettings(BaseModel):
    """Retry policy configuration."""

    model_config = ConfigDict(validate_assignment=True)

    max_attempts: int = 3
    strategy: BackoffStrategy = BackoffStrategy.EXPONENTIAL
    base_seconds: float | None = None
    factor: float | None = None
    max_seconds: float | None = None


class CacheSettings(BaseModel):
    """Cache configuration."""

    model_config = ConfigDict(validate_assignment=True)

    enabled: bool = True
    ttl: int
    key: str | None = None


class BudgetSettings(BaseModel):
    """Cost budget configuration."""

    model_config = ConfigDict(validate_assignment=True)

    limit_usd: float
    action: BudgetAction = BudgetAction.ABORT


class ActivitySettings(BaseModel):
    """Activity execution settings."""

    model_config = ConfigDict(validate_assignment=True)

    timeout_seconds: int | None = None
    retry: RetrySettings | None = None
    cache: CacheSettings | None = None
    budget: BudgetSettings | None = None
    delay: str | None = None
    scheduled_for: str | None = None
    streaming: bool | None = None
    iteration_scoped: bool | None = None


# =============================================================================
# Dependency Model
# =============================================================================


class Dependency(BaseModel):
    """A dependency on another activity with optional conditions.

    Use this to specify conditional dependencies where the dependency
    is only considered satisfied when the conditions evaluate to true.

    Example:
        # Simple dependency (just use the activity key string)
        activity_def.with_dependencies("other_activity")

        # Dependency with condition
        Dependency(
            activity_key="other_activity",
            conditions=["{{ other_activity.success }} == true"]
        )
    """

    model_config = ConfigDict(validate_assignment=True)

    activity_key: ActivityKey
    conditions: list[str] = Field(default_factory=list)

    @classmethod
    def on(
        cls,
        activity: "Activity | str",
        *conditions: "str | OutputComparison",
    ) -> "Dependency":
        """Create a dependency with optional conditions.

        Args:
            activity: The activity this depends on (Activity instance or key string)
            *conditions: Condition expressions (all must be true)

        Returns:
            Dependency instance

        Example:
            Dependency.on(analyze_activity, analyze_activity["confidence"] > 0.8)
        """
        key = activity.key if isinstance(activity, Activity) else activity
        return cls(
            activity_key=key,
            conditions=[str(c) for c in conditions],
        )


# =============================================================================
# Activity Model
# =============================================================================


class Activity(BaseModel):
    """Activity within a workflow definition.

    Specifies which activity to run on which worker with what parameters.
    Can be constructed declaratively or using method chaining.

    Declarative example:
        activity = Activity(
            key="fetch_data",
            worker="std",
            activity_name="http_request",
            parameters={"url": "https://api.example.com"},
            settings=ActivitySettings(timeout_seconds=300),
        )

    Fluent example:
        activity = (
            Activity(key="fetch_data")
            .with_worker("std", "http_request")
            .with_params(url="https://api.example.com")
            .with_timeout(300)
        )
    """

    model_config = ConfigDict(validate_assignment=True)

    key: ActivityKey
    worker: WorkerName
    # activity_name allows empty during construction for fluent API,
    # validated non-empty at serialization in to_dict()
    activity_name: ActivityNameOptional
    parameters: dict[str, Any] = Field(default_factory=dict)
    settings: ActivitySettings = Field(default_factory=ActivitySettings)
    depends_on: list[str | Dependency] = Field(default_factory=list)

    # -------------------------------------------------------------------------
    # Fluent Builder Methods
    # -------------------------------------------------------------------------

    def with_worker(self, worker: str, activity_name: str) -> Self:
        """Set the worker and activity name.

        Args:
            worker: Worker identifier (e.g., "std", "python")
            activity_name: Activity name within the worker (e.g., "http_request")

        Returns:
            self for chaining
        """
        self.worker = worker
        self.activity_name = activity_name
        return self

    def with_params(self, **parameters: Any) -> Self:
        """Set activity parameters.

        Parameters can include template expressions (Input, OutputRef, etc.)
        which will be serialized appropriately when exported to YAML.

        Args:
            **parameters: Keyword arguments for activity parameters

        Returns:
            self for chaining
        """
        self.parameters.update(_serialize_parameters(parameters))
        return self

    def with_timeout(self, seconds: int) -> Self:
        """Set activity timeout in seconds.

        Args:
            seconds: Maximum execution time before timeout

        Returns:
            self for chaining
        """
        self.settings.timeout_seconds = seconds
        return self

    def with_retry(
        self,
        max_attempts: int = 3,
        strategy: BackoffStrategy = BackoffStrategy.EXPONENTIAL,
        base_seconds: float | None = None,
        factor: float | None = None,
        max_seconds: float | None = None,
    ) -> Self:
        """Set retry policy.

        Args:
            max_attempts: Maximum number of retry attempts
            strategy: Backoff strategy (exponential or fixed)
            base_seconds: Initial backoff duration
            factor: Multiplier for exponential backoff
            max_seconds: Maximum backoff duration

        Returns:
            self for chaining
        """
        self.settings.retry = RetrySettings(
            max_attempts=max_attempts,
            strategy=strategy,
            base_seconds=base_seconds,
            factor=factor,
            max_seconds=max_seconds,
        )
        return self

    def with_cache(self, ttl: int, key: str | None = None) -> Self:
        """Enable result caching.

        Args:
            ttl: Cache time-to-live in seconds
            key: Optional custom cache key template

        Returns:
            self for chaining
        """
        self.settings.cache = CacheSettings(ttl=ttl, key=key)
        return self

    def with_budget(
        self, limit_usd: float, action: BudgetAction = BudgetAction.ABORT
    ) -> Self:
        """Set cost budget limit.

        Args:
            limit_usd: Maximum cost in USD
            action: Action when budget exceeded (abort or continue)

        Returns:
            self for chaining
        """
        self.settings.budget = BudgetSettings(limit_usd=limit_usd, action=action)
        return self

    def with_delay(self, delay: str) -> Self:
        """Set execution delay.

        Args:
            delay: Delay duration (e.g., "5s", "1m", "30s")

        Returns:
            self for chaining
        """
        self.settings.delay = delay
        return self

    def with_streaming(self, enabled: bool = True) -> Self:
        """Enable token streaming for LLM activities.

        Args:
            enabled: Whether to enable streaming

        Returns:
            self for chaining
        """
        self.settings.streaming = enabled
        return self

    def with_dependencies(self, *dependencies: "Activity | Dependency | str") -> Self:
        """Add activity dependencies.

        Activities listed here must complete before this activity runs.
        Use Dependency.on() to add conditions to dependencies.

        Args:
            *dependencies: Activity instances, Dependency objects, or activity key strings

        Returns:
            self for chaining

        Example:
            # Simple dependencies
            activity.with_dependencies(step1, step2)
            activity.with_dependencies("step1", "step2")

            # Dependency with conditions
            activity.with_dependencies(
                Dependency.on(step1, step1["success"] == True)
            )

            # Mix of simple and conditional
            activity.with_dependencies(
                step1,
                Dependency.on(step2, step2["valid"] == True),
            )
        """
        for dep in dependencies:
            if isinstance(dep, Dependency):
                self.depends_on.append(dep)
            elif isinstance(dep, Activity):
                self.depends_on.append(dep.key)
            else:
                # String key
                self.depends_on.append(dep)
        return self

    # -------------------------------------------------------------------------
    # Output Reference Support
    # -------------------------------------------------------------------------

    def __getitem__(self, key: str) -> OutputRef:
        """Access activity output by key.

        Enables activity_def["field"] syntax for referencing outputs.

        Args:
            key: Output field key (supports dot notation for nested paths)

        Returns:
            OutputRef that can be used in parameters or dependency conditions

        Example:
            # Use in parameters
            save.with_params(data=process["result"])

            # Use in dependency conditions
            notify.with_dependencies(
                Dependency.on(analyze, analyze["confidence"] > 0.8)
            )
        """
        from .expressions import OutputRef

        return OutputRef(self, key)

    @property
    def failed(self) -> str:
        """Condition expression for activity failure.

        Use in Dependency to run an activity only if this one failed.

        Example:
            handle_error.with_dependencies(Dependency.on(process, process.failed))
        """
        return f"{{{{ {self.key}.status == 'failed' }}}}"

    @property
    def succeeded(self) -> str:
        """Condition expression for activity success.

        Use in Dependency to run an activity only if this one succeeded.

        Example:
            next_step.with_dependencies(Dependency.on(process, process.succeeded))
        """
        return f"{{{{ {self.key}.status == 'succeeded' }}}}"

    # -------------------------------------------------------------------------
    # Serialization
    # -------------------------------------------------------------------------

    def to_dict(self) -> dict[str, Any]:
        """Convert to YAML-compatible dictionary format.

        Raises:
            ValueError: If activity_name is empty (required for serialization)
        """
        if not self.activity_name:
            raise ValueError(
                f"Activity '{self.key}' has no activity_name. "
                "Use .with_worker(worker, activity_name) or set activity_name directly."
            )

        result: dict[str, Any] = {
            "key": self.key,
            "worker": self.worker,
            "activity_name": self.activity_name,
        }

        if self.parameters:
            result["parameters"] = self.parameters

        # Add settings if any are set
        settings_dict: dict[str, Any] = {}
        if self.settings.timeout_seconds is not None:
            settings_dict["timeout_seconds"] = self.settings.timeout_seconds
        if self.settings.retry is not None:
            retry_dict: dict[str, Any] = {
                "max_attempts": self.settings.retry.max_attempts,
                "strategy": self.settings.retry.strategy.value,
            }
            if self.settings.retry.base_seconds is not None:
                retry_dict["base_seconds"] = self.settings.retry.base_seconds
            if self.settings.retry.factor is not None:
                retry_dict["factor"] = self.settings.retry.factor
            if self.settings.retry.max_seconds is not None:
                retry_dict["max_seconds"] = self.settings.retry.max_seconds
            settings_dict["retry"] = retry_dict
        if self.settings.cache is not None:
            cache_dict: dict[str, Any] = {
                "enabled": self.settings.cache.enabled,
                "ttl": self.settings.cache.ttl,
            }
            if self.settings.cache.key is not None:
                cache_dict["key"] = self.settings.cache.key
            settings_dict["cache"] = cache_dict
        if self.settings.budget is not None:
            settings_dict["budget"] = {
                "limit_usd": self.settings.budget.limit_usd,
                "action": self.settings.budget.action.value,
            }
        if self.settings.delay is not None:
            settings_dict["delay"] = self.settings.delay
        if self.settings.scheduled_for is not None:
            settings_dict["scheduled_for"] = self.settings.scheduled_for
        if self.settings.streaming is not None:
            settings_dict["streaming"] = self.settings.streaming
        if self.settings.iteration_scoped is not None:
            settings_dict["iteration_scoped"] = self.settings.iteration_scoped

        if settings_dict:
            result["settings"] = settings_dict

        # Add dependencies
        if self.depends_on:
            deps_list: list[str | dict[str, Any]] = []
            for dep in self.depends_on:
                if isinstance(dep, str):
                    deps_list.append(dep)
                elif dep.conditions:
                    # Dependency with conditions
                    deps_list.append(
                        {
                            "activity_key": dep.activity_key,
                            "conditions": dep.conditions,
                        }
                    )
                else:
                    # Dependency without conditions - just use string
                    deps_list.append(dep.activity_key)
            result["depends_on"] = deps_list

        return result


# =============================================================================
# Input Schema Model
# =============================================================================


class InputSchema(BaseModel):
    """Schema definition for a workflow input."""

    model_config = ConfigDict(validate_assignment=True)

    type: str = "string"
    required: bool = True
    default: Any = None
    description: str | None = None


# =============================================================================
# Workflow Model
# =============================================================================


class Workflow(BaseModel):
    """Workflow definition with fluent builder methods.

    Can be constructed declaratively or using method chaining.

    Declarative example:
        workflow = Workflow(
            name="my_workflow",
            version="1.0.0",
            activities=[activity1, activity2],
        )

    Fluent example:
        workflow = (
            Workflow(name="my_workflow")
            .with_inputs(text_input, url_input)
            .with_activities(activity1, activity2, activity3)
        )
    """

    model_config = ConfigDict(validate_assignment=True)

    name: str
    version: str = "1.0.0"
    namespace: str = "default"
    description: str | None = None
    inputs: dict[str, InputSchema] = Field(default_factory=dict)
    activities: list[Activity] = Field(default_factory=list)

    # -------------------------------------------------------------------------
    # Fluent Builder Methods
    # -------------------------------------------------------------------------

    def with_version(self, version: str) -> Self:
        """Set the workflow version.

        Args:
            version: Version string (e.g., "1.0.0")

        Returns:
            self for chaining
        """
        self.version = version
        return self

    def with_namespace(self, namespace: str) -> Self:
        """Set the workflow namespace.

        Args:
            namespace: Namespace for organization

        Returns:
            self for chaining
        """
        self.namespace = namespace
        return self

    def with_description(self, description: str) -> Self:
        """Set the workflow description.

        Args:
            description: Human-readable description

        Returns:
            self for chaining
        """
        self.description = description
        return self

    def with_inputs(self, *inputs: Input) -> Self:
        """Declare workflow inputs.

        Args:
            *inputs: Input definitions

        Returns:
            self for chaining
        """
        for inp in inputs:
            self.inputs[inp.name] = InputSchema(
                type=_python_type_to_schema_type(inp._type),
                required=inp.required,
                default=inp.default,
                description=inp._description,
            )
        return self

    def with_activities(self, *activities: Activity) -> Self:
        """Add activity definitions to the workflow.

        Args:
            *activities: Activity instances to add

        Returns:
            self for chaining
        """
        self.activities.extend(activities)
        return self

    # -------------------------------------------------------------------------
    # Serialization
    # -------------------------------------------------------------------------

    def to_dict(self) -> dict[str, Any]:
        """Convert to YAML-compatible dictionary format."""
        result: dict[str, Any] = {
            "name": self.name,
            "version": self.version,
        }

        if self.namespace != "default":
            result["namespace"] = self.namespace

        if self.description is not None:
            result["description"] = self.description

        if self.inputs:
            inputs_dict: dict[str, Any] = {}
            for name, schema in self.inputs.items():
                input_schema: dict[str, Any] = {
                    "type": schema.type,
                    "required": schema.required,
                }
                if schema.default is not None:
                    input_schema["default"] = schema.default
                if schema.description is not None:
                    input_schema["description"] = schema.description
                inputs_dict[name] = input_schema
            result["inputs"] = inputs_dict

        if self.activities:
            result["activities"] = [activity.to_dict() for activity in self.activities]

        return result

    def to_yaml(self) -> str:
        """Serialize to YAML format."""
        return yaml.dump(self.to_dict(), sort_keys=False, default_flow_style=False)

    def to_json(self) -> dict[str, Any]:
        """Serialize to JSON-compatible dictionary (for API deployment)."""
        return self.model_dump(mode="json", exclude_none=True)


# =============================================================================
# Helper Functions
# =============================================================================


def _serialize_parameters(params: dict[str, Any]) -> dict[str, Any]:
    """Serialize parameters, converting expression objects to strings."""
    result: dict[str, Any] = {}
    for key, value in params.items():
        result[key] = _serialize_value(value)
    return result


def _serialize_value(value: Any) -> Any:
    """Serialize a single value, handling nested structures."""
    from .expressions import Expression

    if isinstance(value, Expression):
        return str(value)
    if isinstance(value, dict):
        return {k: _serialize_value(v) for k, v in value.items()}
    if isinstance(value, list):
        return [_serialize_value(v) for v in value]
    return value


def _python_type_to_schema_type(python_type: type | None) -> str:
    """Convert Python type to JSON schema type string."""
    if python_type is None:
        return "string"
    type_map = {
        str: "string",
        int: "integer",
        float: "number",
        bool: "boolean",
        list: "array",
        dict: "object",
    }
    return type_map.get(python_type, "string")
