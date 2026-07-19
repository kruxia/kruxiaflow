"""Pydantic schemas for workflow definitions and activities."""

from typing import Any

from pydantic import BaseModel, Field


class ActivitySchema(BaseModel):
    """Schema for a workflow activity."""

    key: str = Field(..., description="Unique activity identifier within the workflow")
    activity_name: str = Field(
        ...,
        description="Type of activity (e.g., http_request, llm_prompt, postgres_query)",
    )
    worker: str | None = Field(
        None,
        description="Worker to execute the activity (e.g., builtin, py-std, py-data)",
    )
    parameters: dict[str, Any] = Field(
        default_factory=dict,
        description="Activity-specific parameters",
    )
    outputs: list[str] = Field(
        default_factory=list,
        description="Named outputs produced by this activity",
    )
    depends_on: list[str] = Field(
        default_factory=list,
        description="List of activity keys this activity depends on",
    )
    dependency_of: list[str] = Field(
        default_factory=list,
        description="List of activity keys that depend on this activity",
    )
    settings: dict[str, Any] | None = Field(
        None,
        description="Activity settings (retry, timeout, budget)",
    )

    class Config:
        extra = "allow"  # Allow additional fields


class RetrySettings(BaseModel):
    """Retry policy configuration."""

    max_attempts: int = Field(3, ge=1, description="Maximum number of retry attempts")
    strategy: str = Field("exponential", description="Retry strategy (exponential, linear)")
    base_seconds: float = Field(1.0, ge=0, description="Base delay in seconds")
    factor: float = Field(2.0, ge=1, description="Backoff factor for exponential strategy")
    max_seconds: float = Field(60.0, ge=0, description="Maximum delay between retries")


class BudgetSettings(BaseModel):
    """Budget limit configuration."""

    limit_usd: float = Field(..., ge=0, description="Budget limit in USD")
    action: str = Field("abort", description="Action when budget exceeded (abort, skip)")


class WorkflowSettings(BaseModel):
    """Workflow-level settings."""

    retry: RetrySettings | None = None
    budget: BudgetSettings | None = None
    timeout: float | None = Field(None, ge=0, description="Workflow timeout in seconds")

    class Config:
        extra = "allow"


class WorkflowDefinition(BaseModel):
    """Complete workflow definition schema."""

    name: str = Field(..., description="Unique workflow name")
    description: str | None = Field(None, description="Human-readable description")
    activities: list[ActivitySchema] = Field(
        ...,
        min_length=1,
        description="List of activities in the workflow",
    )
    parameters: dict[str, Any] = Field(
        default_factory=dict,
        description="Input parameters the workflow accepts",
    )
    settings: WorkflowSettings | None = Field(
        None,
        description="Workflow-level settings",
    )
    namespace: str | None = Field(None, description="Workflow namespace for organization")

    class Config:
        extra = "allow"

    def validate_dependencies(self) -> list[str]:
        """Validate activity dependencies and return any errors.

        Returns:
            List of validation error messages
        """
        errors = []
        activity_keys = {activity.key for activity in self.activities}

        # Check for undefined dependencies
        for activity in self.activities:
            for dep in activity.depends_on:
                if dep not in activity_keys:
                    errors.append(
                        f"Activity '{activity.key}' depends on undefined activity '{dep}'"
                    )

        # Check for circular dependencies
        dependencies: dict[str, list[str]] = {
            activity.key: activity.depends_on for activity in self.activities
        }

        def has_cycle(node: str, visited: set[str], rec_stack: set[str]) -> bool:
            visited.add(node)
            rec_stack.add(node)

            for neighbor in dependencies.get(node, []):
                if neighbor not in visited:
                    if has_cycle(neighbor, visited, rec_stack):
                        return True
                elif neighbor in rec_stack:
                    return True

            rec_stack.remove(node)
            return False

        visited: set[str] = set()
        for activity_key in activity_keys:
            if activity_key not in visited and has_cycle(activity_key, visited, set()):
                errors.append("Workflow contains circular dependencies")
                break

        return errors


class WorkflowValidationResult(BaseModel):
    """Result of workflow validation."""

    valid: bool = Field(..., description="Whether the workflow is valid")
    errors: list[str] = Field(default_factory=list, description="Validation errors")
    warnings: list[str] = Field(default_factory=list, description="Validation warnings")
    activities: int = Field(0, description="Number of activities")
    dependencies: dict[str, list[str]] = Field(
        default_factory=dict,
        description="Activity dependency map",
    )
