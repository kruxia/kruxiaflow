#!/usr/bin/env python3
"""Example custom Python worker.

This example demonstrates how to build a custom Python worker using the
Kruxia Flow Worker SDK. Run this worker alongside your Kruxia Flow server
to execute Python activities.

Usage:
    export KRUXIAFLOW_API_URL=http://localhost:8080
    export KRUXIAFLOW_CLIENT_SECRET=your_secret
    python examples/custom_worker.py
"""

import asyncio

from kruxiaflow.worker import (
    Activity,
    ActivityContext,
    ActivityRegistry,
    ActivityResult,
    WorkerConfig,
    WorkerManager,
    activity,
)

# ============================================================================
# Example 1: Activity using @activity decorator (recommended for simple cases)
# ============================================================================


@activity()  # Name defaults to function name: "analyze_sentiment"
async def analyze_sentiment(params: dict, ctx: ActivityContext) -> ActivityResult:
    """Analyze text sentiment using a simple keyword-based approach."""
    text = params["text"]

    # Your custom logic here
    positive_words = {"good", "great", "excellent", "happy", "love"}
    negative_words = {"bad", "terrible", "awful", "sad", "hate"}

    words = set(text.lower().split())
    positive_count = len(words & positive_words)
    negative_count = len(words & negative_words)

    if positive_count > negative_count:
        sentiment = "positive"
        confidence = positive_count / (positive_count + negative_count + 1)
    elif negative_count > positive_count:
        sentiment = "negative"
        confidence = negative_count / (positive_count + negative_count + 1)
    else:
        sentiment = "neutral"
        confidence = 0.5

    return ActivityResult.value(
        "result",
        {
            "sentiment": sentiment,
            "confidence": confidence,
        },
    )


# ============================================================================
# Example 2: Activity using class-based approach (for complex activities)
# ============================================================================


class ProcessFileActivity(Activity):
    """Process a file with periodic heartbeats.

    Use class-based activities when you need:
    - State or configuration
    - Dependency injection
    - More complex initialization
    """

    @property
    def name(self) -> str:
        return "process_file"

    async def execute(self, params: dict, ctx: ActivityContext) -> ActivityResult:
        """Process file contents with heartbeating."""
        file_url = params["file_url"]

        # Download file from workflow storage
        local_path = await ctx.download_file(file_url)

        # Process file (example: uppercase all lines)
        with open(local_path) as f:
            lines = f.readlines()

        processed = []
        for i, line in enumerate(lines):
            # Send heartbeat every 10 lines to prevent timeout
            if i % 10 == 0:
                await ctx.heartbeat()
                ctx.logger.info(f"Processed {i} lines")

            processed.append(line.upper())

        # Write result to local temp file
        result_path = f"{local_path}.processed"
        with open(result_path, "w") as f:
            f.writelines(processed)

        # Upload result back to storage
        result_url = await ctx.upload_file(result_path, "processed.txt")

        return ActivityResult.value(
            "result",
            {
                "result_url": result_url,
                "line_count": len(processed),
            },
        )


# ============================================================================
# Example 3: Activity with cost tracking
# ============================================================================


@activity()  # Name defaults to function name: "call_llm"
async def call_llm(params: dict, ctx: ActivityContext) -> ActivityResult:
    """Call an LLM API and track costs."""
    from decimal import Decimal

    prompt = params["prompt"]

    # Simulate LLM API call
    # In production, you would call OpenAI, Anthropic, etc.
    response = f"This is a simulated response to: {prompt}"

    # Calculate cost based on token usage
    input_tokens = len(prompt.split())
    output_tokens = len(response.split())
    cost_per_1k_tokens = Decimal("0.002")
    total_cost = (Decimal(input_tokens + output_tokens) / 1000) * cost_per_1k_tokens

    return ActivityResult.value("result", response).with_cost(total_cost)


# ============================================================================
# Main: Register activities and run worker
# ============================================================================


async def main() -> None:
    # Load config from environment variables (automatic via Pydantic BaseSettings)
    config = WorkerConfig()  # type: ignore[call-arg]

    # Create registry and register all activities with the worker type from config
    registry = ActivityRegistry()

    # Register decorator-based activities (they are already Activity instances)
    registry.register(analyze_sentiment, config.worker)
    registry.register(call_llm, config.worker)

    # Register class-based activities (need to instantiate)
    registry.register(ProcessFileActivity(), config.worker)

    print(f"Starting worker with ID: {config.worker_id}")
    print(f"Connecting to API: {config.api_url}")
    print(f"Worker type: {config.worker}")
    print(f"Registered activities: {registry.activity_types()}")

    # Create manager and run until shutdown (SIGINT/SIGTERM)
    manager = WorkerManager(config, registry)
    await manager.run_until_shutdown()


if __name__ == "__main__":
    asyncio.run(main())
