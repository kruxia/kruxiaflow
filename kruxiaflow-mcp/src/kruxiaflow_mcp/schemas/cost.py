"""Pydantic schemas for cost tracking and estimation."""


from pydantic import BaseModel, Field


class TokenUsage(BaseModel):
    """Token usage statistics for LLM calls."""

    prompt_tokens: int = Field(0, ge=0, description="Number of prompt tokens")
    output_tokens: int = Field(0, ge=0, description="Number of output tokens")
    total_tokens: int = Field(0, ge=0, description="Total tokens")


class ActivityCost(BaseModel):
    """Cost information for a single activity."""

    activity_key: str = Field(..., description="Activity identifier")
    activity_name: str = Field(..., description="Activity type")
    cost_usd: float = Field(0.0, ge=0, description="Cost in USD")
    provider: str | None = Field(None, description="Service provider (e.g., anthropic, openai)")
    model: str | None = Field(None, description="Model used (e.g., claude-sonnet-4-5)")
    tokens: TokenUsage | None = Field(None, description="Token usage (for LLM activities)")

    class Config:
        extra = "allow"


class WorkflowCost(BaseModel):
    """Complete cost breakdown for a workflow execution."""

    workflow_id: str = Field(..., description="Workflow execution identifier")
    total_cost_usd: float = Field(0.0, ge=0, description="Total cost in USD")
    budget_limit_usd: float | None = Field(None, ge=0, description="Budget limit if set")
    budget_used_percent: float | None = Field(
        None,
        ge=0,
        le=100,
        description="Percentage of budget used",
    )
    activities: list[ActivityCost] = Field(
        default_factory=list,
        description="Per-activity cost breakdown",
    )
    providers: dict[str, float] = Field(
        default_factory=dict,
        description="Cost breakdown by provider",
    )

    class Config:
        extra = "allow"


class CostRange(BaseModel):
    """Cost range for estimation."""

    min: float = Field(0.0, ge=0, description="Minimum estimated cost")
    max: float = Field(0.0, ge=0, description="Maximum estimated cost")


class ActivityCostEstimate(BaseModel):
    """Cost estimate for a single activity."""

    activity_key: str = Field(..., description="Activity identifier")
    activity_name: str = Field(..., description="Activity type")
    estimated_cost_usd: float = Field(0.0, ge=0, description="Estimated cost in USD")
    cost_range_usd: CostRange = Field(..., description="Min/max cost range")


class WorkflowCostEstimate(BaseModel):
    """Pre-execution cost estimate for a workflow."""

    definition_name: str = Field(..., description="Workflow definition name")
    estimated_cost_usd: float = Field(0.0, ge=0, description="Estimated total cost")
    cost_range_usd: CostRange = Field(..., description="Min/max cost range")
    activities: list[ActivityCostEstimate] = Field(
        default_factory=list,
        description="Per-activity cost estimates",
    )
    assumptions: list[str] = Field(
        default_factory=list,
        description="Assumptions made in estimation",
    )
    note: str | None = Field(
        None,
        description="Additional notes about the estimate",
    )


class ModelPricing(BaseModel):
    """Pricing information for a model."""

    model_pattern: str = Field(..., description="Model pattern (e.g., 'anthropic/claude-opus-4')")
    input_price_per_million: float = Field(..., ge=0, description="Input token price per million")
    output_price_per_million: float = Field(..., ge=0, description="Output token price per million")
    provider: str = Field(..., description="Provider name (e.g., 'anthropic', 'openai')")
