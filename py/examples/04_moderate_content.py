"""Content Moderation Workflow - LLM-based content analysis with cost control.

This example demonstrates:
- LLM prompting for content moderation using Claude Haiku
- JSON response parsing from LLM output
- Budget enforcement (abort if cost exceeds limit)
- Retry strategy with exponential backoff
- Database logging of moderation results
- Cost and token tracking

The workflow analyzes user-generated content for community guideline violations
and stores the moderation decision in a PostgreSQL database for audit purposes.
"""

from kruxiaflow import Activity, Input, SecretRef, Workflow

# Define workflow inputs
content_id = Input("content_id", type=str, required=True)
user_content = Input("user_content", type=str, required=True)

# Secret for database connection
db_url = SecretRef("db_url")

# Activity 1: Analyze content using Claude Haiku
analyze_content = Activity(
    key="analyze_content",
    worker="std",
    activity_name="llm_prompt",
    parameters={
        "model": "anthropic/claude-haiku-4-5-20251001",
        "prompt": f"""
            You are a content moderation assistant. Analyze the following text and determine
            if it violates community guidelines.

            Text to moderate:
            {user_content!s}

            Respond with a JSON object containing:
            - "violates": true/false
            - "reason": explanation of your decision
            - "severity": "none", "low", "medium", or "high"
            - "confidence": probability score (0.00 to 1.00) of confidence in your decision
        """,
        "max_tokens": 500,
    },
    outputs=["result"],
    settings={
        "timeout_seconds": 30,
        "retry": {
            "max_attempts": 3,
            "strategy": "exponential",
            "base_seconds": 2,
            "factor": 2,
            "max_seconds": 60,
        },
        "budget": {"limit": 0.50, "action": "abort"},
    },
)

# Activity 2: Store moderation result in database
store_moderation_result = Activity(
    key="store_moderation_result",
    worker="std",
    activity_name="postgres_query",
    parameters={
        "db_url": str(db_url),
        "query": """
            INSERT INTO moderation_log
            (content_id, decision, cost, tokens, moderated_at)
            VALUES ($1, $2, $3::numeric, $4, NOW())
        """,
        "params": [
            str(content_id),
            analyze_content["result.content"],
            analyze_content["result.cost_usd"],
            analyze_content["result.usage.total_tokens"],
        ],
    },
    depends_on=["analyze_content"],
)

# Build the workflow
moderate_content = Workflow(
    name="moderate_content",
    activities=[analyze_content, store_moderation_result],
)

if __name__ == "__main__":
    # Print the compiled YAML to verify
    print(moderate_content)
