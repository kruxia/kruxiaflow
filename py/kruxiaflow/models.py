"""Pydantic models for workflow definitions with fluent builder methods.

This module provides the primary interface for constructing workflows programmatically.
Models can be constructed declaratively (by passing arguments) or using fluent method chaining.
"""

from __future__ import annotations

import inspect
import sys
from collections.abc import Callable
from enum import Enum
from textwrap import dedent
from typing import TYPE_CHECKING, Any, cast

import yaml
from pydantic import BaseModel, ConfigDict, Field, field_validator

from .types import ActivityKey, WorkerName

# =============================================================================
# YAML Configuration
# =============================================================================


class LiteralString(str):
    """String subclass to force YAML literal block scalar representation."""


def str_representer(dumper: yaml.Dumper, data: str) -> yaml.Node:
    """Custom string representer that uses block literal style for multiline strings.

    Automatically applies dedent and uses trimming block literal style (|-).

    Args:
        dumper: YAML dumper instance
        data: String to represent

    Returns:
        YAML scalar node
    """
    if "\n" in data:
        # Apply dedent to remove common leading whitespace and strip leading/trailing whitespace
        dedented = dedent(data).strip()
        # Use literal block scalar (|) for multiline strings
        return dumper.represent_scalar("tag:yaml.org,2002:str", dedented, style="|")
    # Use default representation for single-line strings
    return dumper.represent_scalar("tag:yaml.org,2002:str", data)


def expression_representer(dumper: yaml.Dumper, data: Any) -> yaml.Node:
    """Custom representer for Expression objects.

    Converts Expression objects to their string representation.

    Args:
        dumper: YAML dumper instance
        data: Expression object to represent

    Returns:
        YAML scalar node
    """
    return dumper.represent_scalar("tag:yaml.org,2002:str", str(data))


# Create custom YAML dumper with block literal style for multiline strings
class BlockLiteralDumper(yaml.SafeDumper):
    """YAML dumper that uses block literal style for multiline strings."""


BlockLiteralDumper.add_representer(str, str_representer)

# Add representer for Expression objects (OutputRef, Input, etc.)
# This is imported later to avoid circular imports, so we'll add it lazily


# =============================================================================
# Enum Classes
# =============================================================================


class BackoffStrategy(str, Enum):
    """Retry backoff strategy."""

    EXPONENTIAL = "exponential"
    FIXED = "fixed"


class BudgetAction(str, Enum):
    """Action to take when budget is exceeded."""

    ABORT = "abort"
    CONTINUE = "continue"


class OutputType(str, Enum):
    """Activity output type."""

    VALUE = "value"
    FILE = "file"
    FOLDER = "folder"


if sys.version_info >= (3, 11):
    pass
else:
    pass

if TYPE_CHECKING:
    from .expressions import OutputComparison, OutputRef


# =============================================================================
# Output Models
# =============================================================================


class ActivityOutputDefinition(BaseModel):
    """Activity output definition.

    Specifies the name and type of an activity output.
    Supports shorthand string format or explicit object format.

    Examples:
        # Shorthand (string) - defaults to type "value"
        outputs=["response", "result"]

        # Explicit object format
        outputs=[
            ActivityOutputDefinition(name="response", output_type=OutputType.VALUE),
            ActivityOutputDefinition(name="data_file", output_type=OutputType.FILE),
        ]
    """

    model_config = ConfigDict(validate_assignment=True)

    name: str
    output_type: OutputType = OutputType.VALUE

    @classmethod
    def from_string(cls, name: str) -> ActivityOutputDefinition:
        """Create output definition from shorthand string.

        Args:
            name: Output name (defaults to type "value")

        Returns:
            ActivityOutputDefinition with type "value"
        """
        return cls(name=name, output_type=OutputType.VALUE)


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

    limit: float
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
        activity: Activity | str,
        *conditions: str | OutputComparison,
    ) -> Dependency:
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
    Activities are constructed using declarative Pydantic-style parameters.

    Example:
        activity = Activity(
            key="fetch_data",
            worker="std",
            activity_name="http_request",
            parameters={"url": "https://api.example.com"},
            settings=ActivitySettings(timeout_seconds=300),
            outputs=["response"],
        )
    """

    model_config = ConfigDict(validate_assignment=True)

    key: ActivityKey
    worker: WorkerName = "std"
    activity_name: str
    parameters: dict[str, Any] = Field(default_factory=dict)
    settings: ActivitySettings = Field(default_factory=ActivitySettings)
    depends_on: list[str | Dependency] = Field(default_factory=list)
    outputs: list[ActivityOutputDefinition] = Field(default_factory=list)

    @field_validator("parameters", mode="before")
    @classmethod
    def convert_expression_parameters(cls, v: dict[str, Any]) -> dict[str, Any]:
        """Convert Expression objects in parameters to their string representation.

        Allows users to pass Expression objects (Input, OutputRef, etc.) directly
        in parameters, which are automatically converted to strings.

        Args:
            v: Dictionary of parameters

        Returns:
            Dictionary with Expression objects converted to strings
        """
        if not isinstance(v, dict):
            return v
        return _serialize_parameters(v)

    @field_validator("outputs", mode="before")
    @classmethod
    def convert_output_strings(
        cls, v: list[str | ActivityOutputDefinition | dict[str, Any]]
    ) -> list[ActivityOutputDefinition]:
        """Convert string outputs to ActivityOutputDefinition objects.

        Allows users to specify outputs as simple strings which are automatically
        converted to ActivityOutputDefinition with type "value".

        Args:
            v: List of outputs (strings, dicts, or ActivityOutputDefinition objects)

        Returns:
            List of ActivityOutputDefinition objects
        """
        if not isinstance(v, list):
            return v

        result = []
        for item in v:
            if isinstance(item, str):
                # Convert string to ActivityOutputDefinition
                result.append(ActivityOutputDefinition(name=item))
            elif isinstance(item, dict):
                # Convert dict to ActivityOutputDefinition
                result.append(ActivityOutputDefinition(**item))
            else:
                # Already an ActivityOutputDefinition
                result.append(item)
        return result

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
            save_activity = Activity(
                key="save",
                worker="std",
                activity_name="postgres_query",
                parameters={"data": process["result"]},
            )

            # Use in dependency conditions
            notify = Activity(
                key="notify",
                worker="std",
                activity_name="http_request",
                depends_on=[
                    Dependency.on(analyze, analyze["confidence"] > 0.8)
                ],
            )
        """
        from .expressions import OutputRef

        return OutputRef(self, key)

    @property
    def failed(self) -> str:
        """Condition expression for activity failure.

        Use in Dependency to run an activity only if this one failed.

        Example:
            handle_error = Activity(
                key="handle_error",
                worker="std",
                activity_name="http_request",
                depends_on=[Dependency.on(process, process.failed)],
            )
        """
        return f"{{{{ {self.key}.status == 'failed' }}}}"

    @property
    def succeeded(self) -> str:
        """Condition expression for activity success.

        Use in Dependency to run an activity only if this one succeeded.

        Example:
            next_step = Activity(
                key="next_step",
                worker="std",
                activity_name="http_request",
                depends_on=[Dependency.on(process, process.succeeded)],
            )
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
                "Set activity_name parameter when constructing the Activity."
            )

        result: dict[str, Any] = {
            "key": self.key,
            "worker": self.worker,
            "activity_name": self.activity_name,
        }

        if self.parameters:
            result["parameters"] = _serialize_parameters(self.parameters)

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
                "limit": self.settings.budget.limit,
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

        # Add outputs
        if self.outputs:
            outputs_list: list[str | dict[str, str]] = []
            for output in self.outputs:
                # Use shorthand if type is default "value", otherwise full object
                if output.output_type == OutputType.VALUE:
                    outputs_list.append(output.name)
                else:
                    outputs_list.append(
                        {
                            "name": output.name,
                            "type": output.output_type.value,
                        }
                    )
            result["outputs"] = outputs_list

        return result

    def __str__(self) -> str:
        """Return YAML representation of the activity.

        Returns:
            YAML string representation
        """
        return yaml.dump(
            self.to_dict(),
            Dumper=BlockLiteralDumper,
            sort_keys=False,
            default_flow_style=False,
        )


# =============================================================================
# ScriptActivity - Subclass for Python Script Activities
# =============================================================================


class ScriptActivity(Activity):
    """Factory class for creating Python script activities from functions.

    This class provides a clear, explicit way to create Activity instances that
    execute Python scripts using the built-in "script" activity type. It is
    distinct from the worker module's @activity decorator, which defines custom
    activity implementations.

    Functions use standard Python conventions:
    - Function parameters are populated from INPUT dict keys
    - Return value becomes the OUTPUT dict

    The class serves as a namespace and factory - instances created are still
    regular Activity objects, not ScriptActivity objects.

    Example:
        ```python
        from kruxiaflow import ScriptActivity


        @ScriptActivity.from_function()  # worker="py-std" is the default
        async def transform_data(records):
            import pandas as pd

            df = pd.DataFrame(records)
            return {"summary": df.describe().to_dict()}
        ```

    Note:
        This is not a different runtime type - it's a factory that returns
        regular Activity instances. Use it when you want to be explicit that
        you're creating a Python script activity.
    """

    @classmethod
    def from_function(
        cls,
        key: str | None = None,
        worker: str = "py-std",
        inputs: dict[str, Any] | None = None,
        depends_on: list[str] | None = None,
        **kwargs: Any,
    ) -> Callable[[Callable], Activity]:
        """Create a script Activity from a Python function.

        This provides IDE support for syntax highlighting, linting, and auto-formatting
        by allowing you to define script logic as actual Python code instead of strings.

        Function parameters are automatically extracted from the INPUT dict, and the
        return value becomes the OUTPUT dict.

        The activity key defaults to the function's __name__, similar to the worker
        module's @activity decorator.

        Args:
            key: Activity key/identifier. Defaults to the function's __name__.
            worker: Worker name (default: "py-std"). Optional - use the default unless you've
                    deployed a custom worker that extends py-std.
            inputs: Input data mapping (OutputRef expressions supported)
            depends_on: List of activity keys this depends on
            **kwargs: Additional Activity parameters (settings, outputs, etc.)

        Returns:
            Activity instance (not ScriptActivity - regular Activity)

        Raises:
            ValueError: If key cannot be determined from function name

        Example:
            ```python
            from kruxiaflow import ScriptActivity, Workflow


            # Function parameters are populated from INPUT dict
            # Return value becomes OUTPUT dict
            @ScriptActivity.from_function()  # worker defaults to "py-std"
            async def load_data(url):
                import httpx

                async with httpx.AsyncClient() as client:
                    response = await client.get(url)
                    data = response.json()

                return {"records": data}


            # Parameters are extracted from inputs
            @ScriptActivity.from_function(
                key="transform",
                inputs={"records": load_data["records"]},
                depends_on=["load_data"],
            )
            async def transform_records(records):
                import pandas as pd

                df = pd.DataFrame(records)
                return {"summary": df.to_dict()}


            workflow = Workflow(
                name="pipeline",
                activities=[load_data, transform_records],
            )
            ```

        Comparison with worker @activity:
            - worker.activity(name="foo") -> Defines activity implementation
            - ScriptActivity.from_function(key="foo") -> Creates activity instance
        """

        def decorator(func: Callable) -> Activity:
            """Create Activity from function."""
            # Default key to function name (like worker @activity decorator)
            activity_key = key or getattr(func, "__name__", None)
            if not activity_key:
                raise ValueError(
                    "Activity key could not be determined. "
                    "Provide explicit 'key' parameter or ensure function has __name__."
                )

            # Extract function body as script
            script_code = script(func)

            # Build parameters
            parameters = {"script": script_code}
            if inputs is not None:
                parameters["inputs"] = inputs

            # Create and return regular Activity (not ScriptActivity)
            return Activity(
                key=activity_key,
                worker=worker,
                activity_name="script",
                parameters=parameters,
                depends_on=cast(
                    list[str | Dependency], depends_on if depends_on is not None else []
                ),
                **kwargs,
            )

        return decorator


# =============================================================================
# Workflow Model
# =============================================================================


class Workflow(BaseModel):
    """Workflow definition.

    Workflows are constructed using declarative Pydantic-style parameters.

    Note: Version is auto-generated by the server when the workflow is stored.
    Namespace, description, and inputs are managed separately from the workflow definition.

    Example:
        workflow = Workflow(
            name="my_workflow",
            activities=[activity1, activity2],
        )
    """

    model_config = ConfigDict(validate_assignment=True)

    name: str
    activities: list[Activity] = Field(default_factory=list)

    # -------------------------------------------------------------------------
    # Serialization
    # -------------------------------------------------------------------------

    def to_dict(self) -> dict[str, Any]:
        """Convert to YAML-compatible dictionary format."""
        result: dict[str, Any] = {
            "name": self.name,
            "activities": [activity.to_dict() for activity in self.activities],
        }

        return result

    def to_yaml(self) -> str:
        """Serialize to YAML format with optional module docstring as header comment."""
        # Get the caller's module docstring by walking up the stack
        module_doc = None
        for frame_info in inspect.stack():
            frame = frame_info.frame
            # Look for __main__ module (when script is executed directly)
            if frame.f_globals.get("__name__") == "__main__":
                module_doc = frame.f_globals.get("__doc__")
                break

        # Generate base YAML
        base_yaml = yaml.dump(
            self.to_dict(),
            Dumper=BlockLiteralDumper,
            sort_keys=False,
            default_flow_style=False,
        )

        # Add blank lines before each activity for readability
        yaml_lines = base_yaml.split("\n")
        formatted_lines = []
        first_activity = True
        for line in yaml_lines:
            # Add blank line before each activity (except the first one)
            # Activities start with "- key:" in YAML
            if line.startswith("- key:"):
                if (
                    not first_activity
                    and formatted_lines
                    and formatted_lines[-1].strip()
                ):
                    formatted_lines.append("")
                first_activity = False
            formatted_lines.append(line)

        formatted_yaml = "\n".join(formatted_lines)

        # Prepend module docstring as YAML comment if present
        if module_doc:
            # Format docstring as YAML comments
            comment_lines = []
            for line in module_doc.strip().split("\n"):
                comment_lines.append(f"# {line}" if line.strip() else "#")
            comment_header = "\n".join(comment_lines) + "\n\n"
            return comment_header + formatted_yaml

        return formatted_yaml

    def to_json(self) -> dict[str, Any]:
        """Serialize to JSON-compatible dictionary (for API deployment)."""
        return self.model_dump(mode="json", exclude_none=True)

    def __str__(self) -> str:
        """Return YAML representation of the workflow.

        Returns:
            YAML string representation
        """
        return self.to_yaml()


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


def script(func: Callable) -> str:
    """Convert a Python function into a script activity that uses kwargs and return values.

    This helper function allows you to define script activity code as actual Python
    functions, enabling IDE features like syntax highlighting, linting, and formatting.

    The function should follow these conventions:
    - Function parameters are populated from INPUT dict keys
    - Return value becomes the OUTPUT dict
    - Should be an async function (async def) for async/await support
    - Has access to `ctx`, `logger`, `workflow_id`, `activity_key` variables

    Args:
        func: Python function (preferably async) containing the script logic

    Returns:
        String containing the complete script that:
        1. Defines the function
        2. Extracts parameters from INPUT
        3. Calls the function
        4. Assigns return value to OUTPUT

    Example:
        ```python
        @ScriptActivity.from_function(worker="py-data")
        async def transform_data(records):
            import pandas as pd

            df = pd.DataFrame(records)
            return {"summary": df.describe().to_dict()}


        # Generates script that extracts 'records' from INPUT,
        # calls the function, and assigns result to OUTPUT
        ```

    Note:
        - The function is never actually called during definition
        - Leading indentation is automatically removed
        - Both sync and async functions are supported
    """
    import textwrap

    # Get the source code and signature
    source = inspect.getsource(func)
    sig = inspect.signature(func)

    # Extract parameter names (excluding *args, **kwargs)
    param_names = [
        name
        for name, param in sig.parameters.items()
        if param.kind
        in (
            inspect.Parameter.POSITIONAL_OR_KEYWORD,
            inspect.Parameter.POSITIONAL_ONLY,
            inspect.Parameter.KEYWORD_ONLY,
        )
    ]

    # Parse the source to remove decorator lines
    lines = source.split("\n")

    # Find the function definition line (skip decorators)
    func_def_idx = 0
    for i, line in enumerate(lines):
        stripped = line.strip()
        if stripped.startswith(("def ", "async def ")):
            func_def_idx = i
            break

    # Get source without decorators
    source_without_decorators = "\n".join(lines[func_def_idx:])

    # Dedent the source
    dedented_source = textwrap.dedent(source_without_decorators).strip()

    # Check if it's async
    is_async = inspect.iscoroutinefunction(func)
    await_keyword = "await " if is_async else ""

    # Build the wrapper script
    func_name = func.__name__  # type: ignore[attr-defined]

    # Generate parameter extraction code
    if param_names:
        param_extract = "\n".join(
            f"{name} = INPUT.get('{name}')" for name in param_names
        )
        call_args = ", ".join(param_names)
        wrapper = f"""{dedented_source}

# Extract parameters from INPUT
{param_extract}

# Call function and assign result to OUTPUT
OUTPUT = {await_keyword}{func_name}({call_args})"""
    else:
        # No parameters
        wrapper = f"""{dedented_source}

# Call function and assign result to OUTPUT
OUTPUT = {await_keyword}{func_name}()"""

    return wrapper
