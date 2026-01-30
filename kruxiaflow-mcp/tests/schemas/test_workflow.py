"""Tests for workflow schemas."""

import pytest
from pydantic import ValidationError

from kruxiaflow_mcp.schemas.workflow import (
    ActivitySchema,
    BudgetSettings,
    RetrySettings,
    WorkflowDefinition,
    WorkflowSettings,
    WorkflowValidationResult,
)


def test_activity_schema_valid() -> None:
    """Test creating a valid activity schema."""
    activity = ActivitySchema(
        key="fetch_data",
        activity_name="http_request",
        parameters={"url": "https://api.example.com"},
        outputs=["response"],
    )

    assert activity.key == "fetch_data"
    assert activity.activity_name == "http_request"
    assert activity.parameters["url"] == "https://api.example.com"
    assert activity.outputs == ["response"]
    assert activity.depends_on == []


def test_activity_schema_with_dependencies() -> None:
    """Test activity with dependencies."""
    activity = ActivitySchema(
        key="process_data",
        activity_name="llm_prompt",
        parameters={"prompt": "Process"},
        depends_on=["fetch_data"],
    )

    assert activity.depends_on == ["fetch_data"]


def test_retry_settings() -> None:
    """Test retry settings validation."""
    retry = RetrySettings(
        max_attempts=5,
        strategy="exponential",
        base_seconds=2.0,
        factor=3.0,
        max_seconds=120.0,
    )

    assert retry.max_attempts == 5
    assert retry.strategy == "exponential"
    assert retry.factor == 3.0


def test_retry_settings_invalid_max_attempts() -> None:
    """Test retry settings with invalid max_attempts."""
    with pytest.raises(ValidationError):
        RetrySettings(max_attempts=0)  # Must be >= 1


def test_budget_settings() -> None:
    """Test budget settings validation."""
    budget = BudgetSettings(limit_usd=10.0, action="abort")

    assert budget.limit_usd == 10.0
    assert budget.action == "abort"


def test_budget_settings_negative_limit() -> None:
    """Test budget settings with negative limit."""
    with pytest.raises(ValidationError):
        BudgetSettings(limit_usd=-1.0, action="abort")


def test_workflow_definition_valid() -> None:
    """Test creating a valid workflow definition."""
    workflow = WorkflowDefinition(
        name="test_workflow",
        description="Test workflow",
        activities=[
            ActivitySchema(
                key="fetch",
                activity_name="http_request",
                parameters={"url": "https://example.com"},
            )
        ],
    )

    assert workflow.name == "test_workflow"
    assert len(workflow.activities) == 1
    assert workflow.activities[0].key == "fetch"


def test_workflow_definition_no_activities() -> None:
    """Test workflow definition requires at least one activity."""
    with pytest.raises(ValidationError):
        WorkflowDefinition(name="empty_workflow", activities=[])


def test_workflow_definition_validate_dependencies() -> None:
    """Test dependency validation."""
    workflow = WorkflowDefinition(
        name="valid_deps",
        activities=[
            ActivitySchema(key="a", activity_name="http_request"),
            ActivitySchema(key="b", activity_name="llm_prompt", depends_on=["a"]),
        ],
    )

    errors = workflow.validate_dependencies()
    assert len(errors) == 0


def test_workflow_definition_undefined_dependency() -> None:
    """Test detecting undefined dependencies."""
    workflow = WorkflowDefinition(
        name="invalid_deps",
        activities=[
            ActivitySchema(key="a", activity_name="http_request", depends_on=["nonexistent"])
        ],
    )

    errors = workflow.validate_dependencies()
    assert len(errors) > 0
    assert any("undefined activity" in error.lower() for error in errors)


def test_workflow_definition_circular_dependency() -> None:
    """Test detecting circular dependencies."""
    workflow = WorkflowDefinition(
        name="circular",
        activities=[
            ActivitySchema(key="a", activity_name="http_request", depends_on=["b"]),
            ActivitySchema(key="b", activity_name="http_request", depends_on=["a"]),
        ],
    )

    errors = workflow.validate_dependencies()
    assert len(errors) > 0
    assert any("circular" in error.lower() for error in errors)


def test_workflow_validation_result() -> None:
    """Test workflow validation result schema."""
    result = WorkflowValidationResult(
        valid=True,
        errors=[],
        warnings=["Consider adding retry settings"],
        activities=3,
        dependencies={"a": [], "b": ["a"], "c": ["b"]},
    )

    assert result.valid is True
    assert len(result.warnings) == 1
    assert result.activities == 3
