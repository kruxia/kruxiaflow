"""Pydantic schemas for workflow visualization."""

from pydantic import BaseModel, Field


class DiagramResponse(BaseModel):
    """Response containing a Mermaid diagram."""

    diagram: str = Field(..., description="Mermaid diagram syntax")
    format: str = Field("mermaid", description="Diagram format")
    type: str = Field(..., description="Diagram type (flowchart, graph, etc.)")
    workflow_name: str | None = Field(None, description="Name of the workflow")
    activity_count: int | None = Field(None, ge=0, description="Number of activities")

    class Config:
        extra = "allow"


class CostDiagramResponse(DiagramResponse):
    """Response containing a cost breakdown diagram."""

    total_cost_usd: float = Field(0.0, ge=0, description="Total workflow cost in USD")
