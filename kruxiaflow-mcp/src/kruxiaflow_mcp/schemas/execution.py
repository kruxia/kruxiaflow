"""Pydantic schemas for workflow execution and status."""

from datetime import datetime
from typing import Any

from pydantic import BaseModel, Field


class WorkflowSubmission(BaseModel):
    """Schema for submitting a workflow for execution."""

    workflow_definition: str = Field(..., description="Name of the workflow definition")
    input: dict[str, Any] = Field(
        default_factory=dict,
        description="Input parameters for the workflow",
    )
    budget_limit_usd: float | None = Field(
        None,
        ge=0,
        description="Optional budget limit in USD",
    )


class ActivityStatus(BaseModel):
    """Status of a single activity in a workflow execution."""

    key: str = Field(..., description="Activity identifier")
    activity_name: str = Field(..., description="Activity type")
    status: str = Field(
        ...,
        description="Activity status (pending, running, completed, failed, skipped)",
    )
    started_at: datetime | None = Field(None, description="When activity started")
    completed_at: datetime | None = Field(None, description="When activity completed")
    error: str | None = Field(None, description="Error message if failed")
    retry_count: int = Field(0, ge=0, description="Number of retries attempted")

    class Config:
        extra = "allow"


class WorkflowStatus(BaseModel):
    """Complete workflow execution status."""

    workflow_id: str = Field(..., description="Unique workflow execution identifier")
    definition_name: str = Field(..., description="Name of the workflow definition")
    status: str = Field(
        ...,
        description="Workflow status (pending, running, completed, failed, canceled)",
    )
    started_at: datetime | None = Field(None, description="When workflow started")
    completed_at: datetime | None = Field(None, description="When workflow completed")
    activities: list[ActivityStatus] | None = Field(
        None,
        description="Status of all activities (if requested)",
    )
    error: str | None = Field(None, description="Error message if failed")

    class Config:
        extra = "allow"


class WorkflowList(BaseModel):
    """Paginated list of workflow executions."""

    workflows: list[WorkflowStatus] = Field(
        default_factory=list,
        description="List of workflow summaries",
    )
    total: int = Field(0, ge=0, description="Total count of workflows")
    limit: int = Field(20, ge=1, description="Requested limit")
    offset: int = Field(0, ge=0, description="Requested offset")


class WorkflowSignal(BaseModel):
    """Signal to send to a waiting workflow."""

    signal_name: str = Field(..., description="Name of the signal")
    signal_data: dict[str, Any] | None = Field(
        None,
        description="Optional data to send with the signal",
    )


class WorkflowCancellation(BaseModel):
    """Request to cancel a workflow."""

    reason: str | None = Field(None, description="Reason for cancellation")
