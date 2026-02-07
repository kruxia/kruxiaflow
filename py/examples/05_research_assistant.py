"""Research Assistant Workflow - LLM with automatic fallback and budget-aware selection.

This example demonstrates:
- Multi-model LLM fallback chains for high availability
- Budget-aware model selection (skips expensive models when budget insufficient)
- Automatic provider switching on failure or rate limits
- Cost tracking per provider
- Storing results with provider metadata
- Retry configuration with exponential backoff

Prerequisites:
- At least one LLM provider API key configured:
  - OPENAI_API_KEY for OpenAI models (o1-pro will be skipped due to budget)
  - ANTHROPIC_API_KEY for Anthropic Claude models (Sonnet may be skipped with tight budget)
  - GOOGLE_API_KEY for Google Gemini models (Flash Lite will fit in budget)
- PostgreSQL database with research_log table

Database Setup:
CREATE TABLE research_log (
    id SERIAL PRIMARY KEY,
    question TEXT NOT NULL,
    answer TEXT NOT NULL,
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    cost DECIMAL(10, 6),
    prompt_tokens INTEGER,
    output_tokens INTEGER,
    total_tokens INTEGER,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

Budget-Aware Fallback Behavior (with $0.01 budget):
For typical prompt (100 input tokens + 1000 output tokens):

1. openai/o1-pro ($150/$600 per M tokens):
   Estimated cost: ~$0.615 → SKIPPED (exceeds budget)

2. anthropic/claude-sonnet-4-5-20250929 ($3/$15 per M tokens):
   Estimated cost: ~$0.0153 → SKIPPED (exceeds $0.01 budget)

3. google/gemini-2.0-flash-lite ($0.075/$0.30 per M tokens):
   Estimated cost: ~$0.0003 → USED (well under budget)

The workflow will automatically skip expensive models and use the cheapest
option that fits within the budget.
"""

from kruxiaflow import (
    Activity,
    ActivitySettings,
    BackoffStrategy,
    BudgetAction,
    BudgetSettings,
    Input,
    RetrySettings,
    SecretRef,
    Workflow,
)

# Define workflow inputs
question = Input("question", type=str, required=True)

# Secret for database connection
db_url = SecretRef("db_url")

# Step 1: Ask question using LLM with model fallback chain
# Budget-aware: expensive models automatically skipped if cost exceeds budget
ask_question = Activity(
    key="ask_question",
    worker="std",
    activity_name="llm_prompt",
    parameters={
        # Model fallback chain - tries each in order until success
        # Format: "provider/model" (see config/llm_models.yaml for full catalog)
        "model": [
            "openai/o1-pro",  # Expensive: $150/$600 per M tokens (will be skipped)
            "anthropic/claude-sonnet-4-5-20250929",  # Moderate: $3/$15 per M tokens (may be skipped)
            "google/gemini-2.0-flash-lite",  # Very cheap: $0.075/$0.30 per M tokens (will be used)
        ],
        "prompt": str(question),
        "max_tokens": 1000,
    },
    outputs=["result"],
    settings=ActivitySettings(
        retry=RetrySettings(
            max_attempts=3,
            strategy=BackoffStrategy.EXPONENTIAL,
            base_seconds=2,
            factor=2,
            max_seconds=60,
        ),
        budget=BudgetSettings(
            limit=0.01,  # Tight budget to demonstrate budget-aware fallback
            action=BudgetAction.ABORT,
        ),
    ),
)

# Step 2: Store the response in the database with provider metadata
store_response = Activity(
    key="store_response",
    worker="std",
    activity_name="postgres_query",
    parameters={
        "db_url": str(db_url),
        "query": """
            INSERT INTO research_log
            (question, answer, provider, model, cost, prompt_tokens, output_tokens, total_tokens, created_at)
            VALUES ($1, $2, $3, $4, $5::numeric, $6, $7, $8, NOW())
            RETURNING id
        """,
        "params": [
            str(question),
            ask_question["result.content"],
            ask_question["result.provider"],
            ask_question["result.model"],
            ask_question["result.cost_usd"],
            ask_question["result.usage.prompt_tokens"],
            ask_question["result.usage.output_tokens"],
            ask_question["result.usage.total_tokens"],
        ],
    },
    outputs=["result"],
    depends_on=["ask_question"],
)

# Build the workflow
research_workflow = Workflow(
    name="research_assistant",
    activities=[ask_question, store_response],
)

if __name__ == "__main__":
    # Print the compiled YAML to verify
    print(research_workflow)
