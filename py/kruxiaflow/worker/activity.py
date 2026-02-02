"""Activity interface and result types.

Mirrors Rust ActivityImpl trait and ActivityResult for interface compatibility.
"""

from abc import ABC, abstractmethod
from collections.abc import Callable, Coroutine
from decimal import Decimal
from enum import Enum
from typing import Any

from pydantic import BaseModel, Field, PrivateAttr
from typing_extensions import Self

from .context import ActivityContext


class OutputType(str, Enum):
    """Activity output type. Mirrors Rust ActivityOutputType."""

    VALUE = "value"
    FILE = "file"
    FOLDER = "folder"


class ActivityOutput(BaseModel):
    """Single activity output. Mirrors Rust ActivityOutput."""

    name: str
    output_type: OutputType
    value: Any


class ActivityResult(BaseModel):
    """
    Activity execution result. Mirrors Rust ActivityResult.

    Interface kept identical for PyO3 compatibility.
    """

    outputs: list[ActivityOutput] = Field(default_factory=list)
    cost_usd: Decimal | None = None
    metadata: dict[str, Any] | None = None

    # Error state (for ActivityResult.error()) - private attributes
    _is_error: bool = PrivateAttr(default=False)
    _error_message: str | None = PrivateAttr(default=None)
    _error_code: str | None = PrivateAttr(default=None)
    _retryable: bool = PrivateAttr(default=True)

    @classmethod
    def value(cls, name: str, value: Any) -> Self:
        """Create result with single value output."""
        return cls(
            outputs=[
                ActivityOutput(name=name, output_type=OutputType.VALUE, value=value)
            ]
        )

    @classmethod
    def values(cls, outputs: list[ActivityOutput]) -> Self:
        """Create result with multiple outputs."""
        return cls(outputs=outputs)

    def with_cost(self, cost_usd: Decimal) -> Self:
        """Add cost tracking. Returns self for chaining."""
        self.cost_usd = cost_usd
        return self

    def with_metadata(self, metadata: dict[str, Any]) -> Self:
        """Add metadata. Returns self for chaining."""
        self.metadata = metadata
        return self

    @classmethod
    def error(
        cls,
        message: str,
        code: str = "EXECUTION_ERROR",
        retryable: bool = True,
    ) -> Self:
        """Create error result."""
        result = cls()
        result._is_error = True
        result._error_message = message
        result._error_code = code
        result._retryable = retryable
        return result

    @property
    def is_error(self) -> bool:
        """Check if this is an error result."""
        return self._is_error

    @property
    def error_message(self) -> str | None:
        """Get error message if this is an error result."""
        return self._error_message

    @property
    def error_code(self) -> str | None:
        """Get error code if this is an error result."""
        return self._error_code

    @property
    def retryable(self) -> bool:
        """Check if error is retryable."""
        return self._retryable

    def to_output_dict(self) -> dict[str, Any]:
        """Convert outputs to dict for API. Mirrors Rust to_json_value()."""
        return {
            out.name: out.value
            for out in self.outputs
            if out.output_type == OutputType.VALUE
        }


class Activity(ABC):
    """
    Activity implementation interface.

    Mirrors Rust ActivityImpl trait for interface compatibility.
    Python implementations should subclass this or use the @activity decorator.

    Note: Activities are defined without a worker assignment. The worker type
    is assigned when the activity is registered with an ActivityRegistry.
    """

    @property
    @abstractmethod
    def name(self) -> str:
        """Activity name (e.g., 'analyze_text')."""

    @abstractmethod
    async def execute(
        self, params: dict[str, Any], ctx: ActivityContext
    ) -> ActivityResult:
        """
        Execute the activity.

        Args:
            params: Activity input parameters (JSON-compatible dict)
            ctx: Execution context with workflow_id, activity_id, heartbeat, file ops

        Returns:
            ActivityResult with outputs, optional cost, optional metadata

        Raises:
            Exception: Activity failed (will be reported as retryable error)
        """


# Type alias for activity handler functions
ActivityHandler = Callable[
    [dict[str, Any], ActivityContext],
    Coroutine[Any, Any, ActivityResult],
]


def activity(
    name: str | None = None,
) -> Callable[[ActivityHandler], Activity]:
    """
    Decorator to create an Activity from an async function.

    The activity name defaults to the function name. Provide an explicit name
    if you want a different activity name than the function name.

    The worker type is assigned when the activity is registered with an
    ActivityRegistry, not at definition time.

    Args:
        name: Activity name. Defaults to the decorated function's __name__.

    Raises:
        ValueError: If the activity name cannot be determined.

    Usage:
        @activity()
        async def my_activity(params: dict, ctx: ActivityContext) -> ActivityResult:
            return ActivityResult.value("result", params["input"] * 2)

        @activity(name="custom_name")
        async def another(params: dict, ctx: ActivityContext) -> ActivityResult:
            ...
    """

    def decorator(func: ActivityHandler) -> Activity:
        activity_name = name or getattr(func, "__name__", None)
        if not activity_name:
            raise ValueError(
                "Activity name could not be determined. "
                "Provide an explicit name: @activity(name='my_activity')"
            )

        class DecoratedActivity(Activity):
            @property
            def name(self) -> str:
                return activity_name

            async def execute(
                self, params: dict[str, Any], ctx: ActivityContext
            ) -> ActivityResult:
                return await func(params, ctx)

        return DecoratedActivity()

    return decorator
