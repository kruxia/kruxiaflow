"""Template expressions for workflow definitions.

This module provides classes for creating template expressions that reference
workflow inputs and activity outputs. These expressions are rendered as template
strings in the generated YAML.
"""

from __future__ import annotations

from typing import TYPE_CHECKING, Any

if TYPE_CHECKING:
    from .models import Activity


class Input:
    """Reference to a workflow input parameter.

    Used in activity parameters to reference values passed when starting a workflow.

    Example:
        user_text = Input("text", type=str, required=True)

        analyze = (
            Activity(key="analyze")
            .with_worker("builtin", "llm_prompt")
            .with_params(prompt=f"Analyze: {user_text}")
        )
    """

    def __init__(
        self,
        name: str,
        *,
        type: type | None = None,
        required: bool = True,
        default: Any = None,
        description: str | None = None,
    ):
        """Create an input reference.

        Args:
            name: The input parameter name
            type: Expected type (for documentation/validation)
            required: Whether the input is required
            default: Default value if not provided
            description: Human-readable description
        """
        self._name = name
        self._type = type
        self._required = required
        self._default = default
        self._description = description

    @property
    def name(self) -> str:
        """Get the input parameter name."""
        return self._name

    @property
    def required(self) -> bool:
        """Check if input is required."""
        return self._required

    @property
    def default(self) -> Any:
        """Get the default value."""
        return self._default

    def __str__(self) -> str:
        """Render as template expression."""
        return f"{{{{INPUT.{self._name}}}}}"

    def __repr__(self) -> str:
        return f"Input({self._name!r})"

    def __format__(self, format_spec: str) -> str:
        """Support f-string formatting."""
        return str(self)

    def to_schema(self) -> dict[str, Any]:
        """Convert to input schema for workflow definition."""
        schema: dict[str, Any] = {
            "required": self._required,
        }
        if self._type is not None:
            type_map = {
                str: "string",
                int: "integer",
                float: "number",
                bool: "boolean",
                list: "array",
                dict: "object",
            }
            schema["type"] = type_map.get(self._type, "string")
        if self._default is not None:
            schema["default"] = self._default
        if self._description is not None:
            schema["description"] = self._description
        return schema


class OutputRef:
    """Reference to an activity output field.

    Supports comparison operators for use in Dependency conditions.

    Example:
        analyze = Activity(key="analyze").with_worker("builtin", "llm_prompt")...

        # Access output field
        sentiment = analyze["sentiment"]

        # Use in dependency conditions
        notify = (
            Activity(key="notify")
            .with_dependencies(Dependency.on(analyze, analyze["confidence"] > 0.8))
        )
    """

    def __init__(self, activity: Activity, key: str):
        """Create an output reference.

        Args:
            activity: The activity whose output to reference
            key: The output field key (supports dot notation for nested paths)
        """
        self._activity = activity
        self._key = key

    @property
    def activity_key(self) -> str:
        """Get the referenced activity's key."""
        return self._activity.key

    @property
    def output_key(self) -> str:
        """Get the output field key."""
        return self._key

    def __str__(self) -> str:
        """Render as template expression."""
        return f"{{{{{self._activity.key}.{self._key}}}}}"

    def __repr__(self) -> str:
        return f"OutputRef({self._activity.key!r}, {self._key!r})"

    def __format__(self, format_spec: str) -> str:
        """Support f-string formatting."""
        return str(self)

    def __eq__(self, other: object) -> OutputComparison:  # type: ignore[override]
        """Create equality comparison."""
        return OutputComparison(
            f"{self._activity.key}.{self._key} == {_format_value(other)}"
        )

    def __ne__(self, other: object) -> OutputComparison:  # type: ignore[override]
        """Create inequality comparison."""
        return OutputComparison(
            f"{self._activity.key}.{self._key} != {_format_value(other)}"
        )

    def __gt__(self, other: object) -> OutputComparison:
        """Create greater-than comparison."""
        return OutputComparison(
            f"{self._activity.key}.{self._key} > {_format_value(other)}"
        )

    def __lt__(self, other: object) -> OutputComparison:
        """Create less-than comparison."""
        return OutputComparison(
            f"{self._activity.key}.{self._key} < {_format_value(other)}"
        )

    def __ge__(self, other: object) -> OutputComparison:
        """Create greater-than-or-equal comparison."""
        return OutputComparison(
            f"{self._activity.key}.{self._key} >= {_format_value(other)}"
        )

    def __le__(self, other: object) -> OutputComparison:
        """Create less-than-or-equal comparison."""
        return OutputComparison(
            f"{self._activity.key}.{self._key} <= {_format_value(other)}"
        )


class OutputComparison:
    """Comparison expression for use in Dependency conditions.

    Created by comparing OutputRef with values. Can be combined with & (and) and | (or).

    Example:
        # Simple condition
        Dependency.on(analyze, analyze["confidence"] > 0.8)

        # Combined conditions
        Dependency.on(analyze,
            (analyze["confidence"] > 0.8) & (analyze["sentiment"] == "positive")
        )
    """

    def __init__(self, expr: str):
        """Create a comparison expression.

        Args:
            expr: The comparison expression string
        """
        self._expr = expr

    @property
    def expression(self) -> str:
        """Get the raw expression string."""
        return self._expr

    def __str__(self) -> str:
        """Render as template expression."""
        return f"{{{{ {self._expr} }}}}"

    def __repr__(self) -> str:
        return f"OutputComparison({self._expr!r})"

    def __and__(self, other: OutputComparison) -> OutputComparison:
        """Combine with AND."""
        return OutputComparison(f"({self._expr}) && ({other._expr})")

    def __or__(self, other: OutputComparison) -> OutputComparison:
        """Combine with OR."""
        return OutputComparison(f"({self._expr}) || ({other._expr})")

    def __invert__(self) -> OutputComparison:
        """Negate the condition."""
        return OutputComparison(f"!({self._expr})")


class SecretRef:
    """Reference to a secret value.

    Used to reference secrets that are injected at runtime, not stored in workflow definitions.

    Example:
        api_key = SecretRef("api_key")

        call_api = (
            Activity(key="call_api")
            .with_worker("builtin", "http_request")
            .with_params(headers={"Authorization": f"Bearer {api_key}"})
        )
    """

    def __init__(self, name: str):
        """Create a secret reference.

        Args:
            name: The secret name
        """
        self._name = name

    @property
    def name(self) -> str:
        """Get the secret name."""
        return self._name

    def __str__(self) -> str:
        """Render as template expression."""
        return f"{{{{SECRET.{self._name}}}}}"

    def __repr__(self) -> str:
        return f"SecretRef({self._name!r})"

    def __format__(self, format_spec: str) -> str:
        """Support f-string formatting."""
        return str(self)


class EnvRef:
    """Reference to an environment variable.

    Used to reference environment variables available at workflow execution time.

    Example:
        db_url = EnvRef("DATABASE_URL")

        query = (
            Activity(key="query")
            .with_worker("builtin", "postgres_query")
            .with_params(database_url=str(db_url))
        )
    """

    def __init__(self, name: str):
        """Create an environment variable reference.

        Args:
            name: The environment variable name
        """
        self._name = name

    @property
    def name(self) -> str:
        """Get the environment variable name."""
        return self._name

    def __str__(self) -> str:
        """Render as template expression."""
        return f"${{{self._name}}}"

    def __repr__(self) -> str:
        return f"EnvRef({self._name!r})"

    def __format__(self, format_spec: str) -> str:
        """Support f-string formatting."""
        return str(self)


def _format_value(value: object) -> str:
    """Format a value for use in a comparison expression."""
    if isinstance(value, str):
        # Escape single quotes and wrap in quotes
        escaped = value.replace("'", "\\'")
        return f"'{escaped}'"
    if isinstance(value, bool):
        return "true" if value else "false"
    if value is None:
        return "null"
    return str(value)
