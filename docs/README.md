# StreamFlow

**AI-Native Workflow Orchestration with Built-in Cost Control**

StreamFlow is a lightweight, high-performance workflow engine designed for AI applications. Track every token, cache intelligently, and never exceed your LLM budget.

## Key Features

- **Durable Execution** - Workflows survive crashes and restart from where they left off
- **LLM Cost Tracking** - Every token is tracked, every dollar is accounted for
- **Budget Enforcement** - Set limits per workflow, activity, or model
- **Semantic Caching** - Save 50-80% on LLM costs by caching similar queries
- **Multi-Provider LLM** - Native support for Anthropic, OpenAI, Google, Ollama

## Why StreamFlow?

| Feature              | StreamFlow | Temporal | Airflow | LangChain |
|----------------------|:----------:|:--------:|:-------:|:---------:|
| Durable execution    | **Yes**    | Yes      | Yes     | No        |
| LLM cost tracking    | **Yes**    | No       | No      | No        |
| Budget enforcement   | **Yes**    | No       | No      | No        |
| Semantic caching     | **Yes**    | No       | No      | Partial   |
| Single binary        | **7.5MB**  | ~200MB   | ~500MB+ | N/A       |
| Docker image         | **63MB**   | ~500MB   | ~1GB+   | N/A       |

## Quick Links

- [Quick Start](quickstart.md) - Get running in 60 seconds
- [Architecture](architecture.md) - System design overview
- [GitHub Repository](https://github.com/kruxia/streamflow)
