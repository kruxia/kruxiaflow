"""Expression system for workflow definitions.

This module provides a SQLAlchemy-style expression system for building
workflow expressions. Expressions are composed into trees and compiled
to template strings at serialization time.

Example:
    from kruxiaflow import Activity, Dependency
    from kruxiaflow.expressions import Input, workflow, and_, is_null

    check = Activity(key="check").with_worker("std", "http_request")

    # Simple condition
    success = Activity(key="success").with_dependencies(
        Dependency.on(check, check["status"] == "ok")
    )

    # Complex condition with operators
    guarded = Activity(key="guarded").with_dependencies(
        Dependency.on(check,
            (check["status"] == "ok") & (check["score"] > 0.8)
        )
    )

    # Using helper functions
    safe = Activity(key="safe").with_dependencies(
        Dependency.on(check, and_(
            ~is_null(check["result"]),
            check["result.count"] > 0
        ))
    )
"""

from __future__ import annotations

from abc import ABC, abstractmethod
from typing import TYPE_CHECKING, Any

if TYPE_CHECKING:
    from .models import Activity


# =============================================================================
# Base Expression Class
# =============================================================================


class Expression(ABC):
    """Base class for all expressions.

    Expressions form a tree structure that is compiled to template strings
    at serialization time. All expressions support logical operators:
    - `&` for AND
    - `|` for OR
    - `~` for NOT
    """

    @abstractmethod
    def _to_template(self) -> str:
        """Convert to template string format (without {{ }} wrapper).

        This is the internal representation used for building expressions.
        """

    def __str__(self) -> str:
        """Render as template expression with {{ }} wrapper."""
        return f"{{{{{self._to_template()}}}}}"

    def __format__(self, format_spec: str) -> str:
        """Support f-string formatting."""
        return str(self)

    def __and__(self, other: Expression) -> And:
        """Combine expressions with AND."""
        return And(self, _to_expr(other))

    def __rand__(self, other: Any) -> And:
        """Support reversed AND for non-Expression left operands."""
        return And(_to_expr(other), self)

    def __or__(self, other: Expression) -> Or:
        """Combine expressions with OR."""
        return Or(self, _to_expr(other))

    def __ror__(self, other: Any) -> Or:
        """Support reversed OR for non-Expression left operands."""
        return Or(_to_expr(other), self)

    def __invert__(self) -> Not:
        """Negate the expression."""
        return Not(self)


# =============================================================================
# Value Expressions
# =============================================================================


class Literal(Expression):
    """A literal value in an expression.

    Automatically created when comparing expressions with Python values.
    """

    def __init__(self, value: Any):
        self._value = value

    @property
    def value(self) -> Any:
        """Get the literal value."""
        return self._value

    def _to_template(self) -> str:
        """Format the value for template syntax."""
        if isinstance(self._value, bool):
            return "true" if self._value else "false"
        if self._value is None:
            return "null"
        if isinstance(self._value, str):
            escaped = self._value.replace("'", "\\'")
            return f"'{escaped}'"
        return str(self._value)

    def __repr__(self) -> str:
        return f"Literal({self._value!r})"


class Input(Expression):
    """Reference to a workflow input parameter.

    Used in activity parameters to reference values passed when starting a workflow.

    Example:
        user_text = Input("text", type=str, required=True)

        analyze = (
            Activity(key="analyze")
            .with_worker("std", "llm_prompt")
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

    def _to_template(self) -> str:
        return f"INPUT.{self._name}"

    def __repr__(self) -> str:
        return f"Input({self._name!r})"

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


class SecretRef(Expression):
    """Reference to a secret value.

    Used to reference secrets that are injected at runtime.

    Example:
        api_key = SecretRef("api_key")

        call_api = (
            Activity(key="call_api")
            .with_worker("std", "http_request")
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

    def _to_template(self) -> str:
        return f"SECRET.{self._name}"

    def __repr__(self) -> str:
        return f"SecretRef({self._name!r})"


class EnvRef(Expression):
    """Reference to an environment variable.

    Uses ${VAR} syntax instead of {{ }} for environment variables.

    Example:
        db_url = EnvRef("DATABASE_URL")

        query = (
            Activity(key="query")
            .with_worker("std", "postgres_query")
            .with_params(database_url=db_url)
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

    def _to_template(self) -> str:
        # EnvRef uses different syntax
        return self._name

    def __str__(self) -> str:
        """Render as environment variable reference."""
        return f"${{{self._name}}}"

    def __repr__(self) -> str:
        return f"EnvRef({self._name!r})"


class _WorkflowMeta:
    """Accessor for workflow metadata fields.

    Provides attribute access to workflow metadata like id, name, etc.

    Example:
        from kruxiaflow.expressions import workflow

        activity = (
            Activity(key="log")
            .with_params(
                workflow_id=workflow.id,
                workflow_name=workflow.name,
            )
        )
    """

    def __getattr__(self, name: str) -> WorkflowRef:
        """Get a workflow metadata field."""
        return WorkflowRef(name)


class WorkflowRef(Expression):
    """Reference to a workflow metadata field.

    Created via the `workflow` accessor object.

    Example:
        from kruxiaflow.expressions import workflow

        body = {"workflow_id": workflow.id}
    """

    def __init__(self, field: str):
        """Create a workflow metadata reference.

        Args:
            field: The metadata field name (e.g., "id", "name")
        """
        self._field = field

    @property
    def field(self) -> str:
        """Get the metadata field name."""
        return self._field

    def _to_template(self) -> str:
        return f"WORKFLOW.{self._field}"

    def __repr__(self) -> str:
        return f"WorkflowRef({self._field!r})"


# Singleton accessor for workflow metadata
workflow = _WorkflowMeta()


# =============================================================================
# Output References
# =============================================================================


class OutputRef(Expression):
    """Reference to an activity output field.

    Supports comparison operators for use in Dependency conditions.

    Example:
        analyze = Activity(key="analyze").with_worker("std", "llm_prompt")

        # Access output field
        sentiment = analyze["sentiment"]

        # Use in dependency conditions
        notify = (
            Activity(key="notify")
            .with_dependencies(Dependency.on(analyze, analyze["confidence"] > 0.8))
        )

        # Complex conditions
        alert = (
            Activity(key="alert")
            .with_dependencies(Dependency.on(analyze,
                (analyze["confidence"] > 0.8) & (analyze["sentiment"] == "negative")
            ))
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

    def _to_template(self) -> str:
        return f"{self._activity.key}.{self._key}"

    def __repr__(self) -> str:
        return f"OutputRef({self._activity.key!r}, {self._key!r})"

    # Comparison operators return expression objects

    def __eq__(self, other: object) -> Eq:  # type: ignore[override]
        """Create equality comparison."""
        return Eq(self, _to_expr(other))

    def __ne__(self, other: object) -> Ne:  # type: ignore[override]
        """Create inequality comparison."""
        return Ne(self, _to_expr(other))

    def __gt__(self, other: object) -> Gt:
        """Create greater-than comparison."""
        return Gt(self, _to_expr(other))

    def __lt__(self, other: object) -> Lt:
        """Create less-than comparison."""
        return Lt(self, _to_expr(other))

    def __ge__(self, other: object) -> Ge:
        """Create greater-than-or-equal comparison."""
        return Ge(self, _to_expr(other))

    def __le__(self, other: object) -> Le:
        """Create less-than-or-equal comparison."""
        return Le(self, _to_expr(other))


# =============================================================================
# Comparison Expressions
# =============================================================================


class Comparison(Expression):
    """Base class for binary comparison expressions."""

    op: str  # Operator symbol

    def __init__(self, left: Expression, right: Expression):
        self.left = left
        self.right = right

    def _to_template(self) -> str:
        left_str = self.left._to_template()
        right_str = self.right._to_template()
        return f"{left_str} {self.op} {right_str}"

    def __repr__(self) -> str:
        return f"{self.__class__.__name__}({self.left!r}, {self.right!r})"


class Eq(Comparison):
    """Equality comparison (==)."""

    op = "=="


class Ne(Comparison):
    """Inequality comparison (!=)."""

    op = "!="


class Gt(Comparison):
    """Greater-than comparison (>)."""

    op = ">"


class Lt(Comparison):
    """Less-than comparison (<)."""

    op = "<"


class Ge(Comparison):
    """Greater-than-or-equal comparison (>=)."""

    op = ">="


class Le(Comparison):
    """Less-than-or-equal comparison (<=)."""

    op = "<="


# =============================================================================
# Logical Expressions
# =============================================================================


class And(Expression):
    """Logical AND of two expressions.

    Created using the `&` operator or `and_()` function.

    Example:
        (check["status"] == "ok") & (check["score"] > 0.8)
        and_(check["status"] == "ok", check["score"] > 0.8)
    """

    def __init__(self, left: Expression, right: Expression):
        self.left = left
        self.right = right

    def _to_template(self) -> str:
        left_str = self.left._to_template()
        right_str = self.right._to_template()
        return f"({left_str}) && ({right_str})"

    def __repr__(self) -> str:
        return f"And({self.left!r}, {self.right!r})"


class Or(Expression):
    """Logical OR of two expressions.

    Created using the `|` operator or `or_()` function.

    Example:
        (check["score"] > 0.9) | (check["override"] == True)
        or_(check["score"] > 0.9, check["override"] == True)
    """

    def __init__(self, left: Expression, right: Expression):
        self.left = left
        self.right = right

    def _to_template(self) -> str:
        left_str = self.left._to_template()
        right_str = self.right._to_template()
        return f"({left_str}) || ({right_str})"

    def __repr__(self) -> str:
        return f"Or({self.left!r}, {self.right!r})"


class Not(Expression):
    """Logical NOT of an expression.

    Created using the `~` operator or `not_()` function.

    Example:
        ~(check["valid"] == True)
        not_(check["valid"])
    """

    def __init__(self, expr: Expression):
        self.expr = expr

    def _to_template(self) -> str:
        return f"!({self.expr._to_template()})"

    def __repr__(self) -> str:
        return f"Not({self.expr!r})"


# =============================================================================
# Utility Expressions
# =============================================================================


class IsNull(Expression):
    """Check if a value is null.

    Example:
        is_null(check["result"])  # True if result is null
    """

    def __init__(self, expr: Expression):
        self.expr = expr

    def _to_template(self) -> str:
        return f"{self.expr._to_template()} == null"

    def __repr__(self) -> str:
        return f"IsNull({self.expr!r})"


class IsNotNull(Expression):
    """Check if a value is not null.

    Example:
        is_not_null(check["result"])  # True if result is not null
    """

    def __init__(self, expr: Expression):
        self.expr = expr

    def _to_template(self) -> str:
        return f"{self.expr._to_template()} != null"

    def __repr__(self) -> str:
        return f"IsNotNull({self.expr!r})"


class Contains(Expression):
    """Check if a collection contains a value.

    Example:
        contains(check["tags"], "urgent")  # True if tags contains "urgent"
    """

    def __init__(self, collection: Expression, value: Expression):
        self.collection = collection
        self.value = value

    def _to_template(self) -> str:
        coll_str = self.collection._to_template()
        val_str = self.value._to_template()
        return f"{coll_str} contains {val_str}"

    def __repr__(self) -> str:
        return f"Contains({self.collection!r}, {self.value!r})"


class In(Expression):
    """Check if a value is in a collection.

    Example:
        in_(check["status"], ["ok", "complete"])  # True if status is ok or complete
    """

    def __init__(self, value: Expression, collection: Expression):
        self.value = value
        self.collection = collection

    def _to_template(self) -> str:
        val_str = self.value._to_template()
        coll_str = self.collection._to_template()
        return f"{val_str} in {coll_str}"

    def __repr__(self) -> str:
        return f"In({self.value!r}, {self.collection!r})"


# =============================================================================
# Helper Functions
# =============================================================================


def _to_expr(value: Any) -> Expression:
    """Convert a Python value to an Expression."""
    if isinstance(value, Expression):
        return value
    return Literal(value)


def and_(*exprs: Expression | Any) -> Expression:
    """Combine multiple expressions with AND.

    Example:
        and_(
            check["status"] == "ok",
            check["score"] > 0.8,
            check["valid"] == True
        )
    """
    if len(exprs) == 0:
        raise ValueError("and_() requires at least one expression")
    if len(exprs) == 1:
        return _to_expr(exprs[0])

    result = _to_expr(exprs[0])
    for expr in exprs[1:]:
        result = And(result, _to_expr(expr))
    return result


def or_(*exprs: Expression | Any) -> Expression:
    """Combine multiple expressions with OR.

    Example:
        or_(
            check["status"] == "error",
            check["score"] < 0.5,
            is_null(check["result"])
        )
    """
    if len(exprs) == 0:
        raise ValueError("or_() requires at least one expression")
    if len(exprs) == 1:
        return _to_expr(exprs[0])

    result = _to_expr(exprs[0])
    for expr in exprs[1:]:
        result = Or(result, _to_expr(expr))
    return result


def not_(expr: Expression | Any) -> Not:
    """Negate an expression.

    Example:
        not_(check["valid"])  # Equivalent to ~check["valid"]
    """
    return Not(_to_expr(expr))


def is_null(expr: Expression | Any) -> IsNull:
    """Check if a value is null.

    Example:
        is_null(check["result"])
    """
    return IsNull(_to_expr(expr))


def is_not_null(expr: Expression | Any) -> IsNotNull:
    """Check if a value is not null.

    Example:
        is_not_null(check["result"])
    """
    return IsNotNull(_to_expr(expr))


def contains(collection: Expression | Any, value: Any) -> Contains:
    """Check if a collection contains a value.

    Example:
        contains(check["tags"], "urgent")
    """
    return Contains(_to_expr(collection), _to_expr(value))


def in_(value: Expression | Any, collection: list | Expression) -> In:
    """Check if a value is in a collection.

    Example:
        in_(check["status"], ["ok", "complete", "done"])
    """
    return In(_to_expr(value), _to_expr(collection))


# =============================================================================
# Backward Compatibility
# =============================================================================

# OutputComparison is now just an alias for the base Comparison type
# This maintains compatibility with existing code that references it
OutputComparison = Comparison
