"""Document Processing Workflow - Parallel execution (fan-out/fan-in) example.

This example demonstrates:
- Declarative activity definition with parallel execution
- Multiple dependencies for aggregation (fan-in)
- Referencing outputs from multiple upstream activities
- Echo activity to simulate processing
- Email notification with aggregated results

Workflow DAG:

  fetch_doc1 ──→ process_doc1 ──┐
  fetch_doc2 ──→ process_doc2 ──┼──→ aggregate_results ──→ store_summary
  fetch_doc3 ──→ process_doc3 ──┘
"""

from textwrap import dedent

from kruxiaflow import Activity, Workflow, workflow

# === PARALLEL FETCH (Fan-Out) ===
# These three activities have no dependencies and execute in parallel

fetch_doc1 = Activity(
    key="fetch_doc1",
    worker="std",
    activity_name="http_request",
    parameters={
        "method": "GET",
        "url": "https://httpbin.org/get?doc=1&title=Introduction",
    },
    outputs=["response"],  # String auto-converted to ActivityOutputDefinition
)

fetch_doc2 = Activity(
    key="fetch_doc2",
    worker="std",
    activity_name="http_request",
    parameters={
        "method": "GET",
        "url": "https://httpbin.org/get?doc=2&title=Architecture",
    },
    outputs=["response"],  # String auto-converted to ActivityOutputDefinition
)

fetch_doc3 = Activity(
    key="fetch_doc3",
    worker="std",
    activity_name="http_request",
    parameters={
        "method": "GET",
        "url": "https://httpbin.org/get?doc=3&title=Conclusion",
    },
    outputs=["response"],  # String auto-converted to ActivityOutputDefinition
)


# === PARALLEL PROCESS (Fan-Out) ===
# Each process activity depends on its corresponding fetch activity.
# Echo passes through the fetched data, simulating a processing step.

process_doc1 = Activity(
    key="process_doc1",
    worker="std",
    activity_name="echo",
    parameters={
        "doc": 1,
        "title": fetch_doc1["response.body.args.title"],
        "url": fetch_doc1["response.body.url"],
    },
    depends_on=["fetch_doc1"],
)

process_doc2 = Activity(
    key="process_doc2",
    worker="std",
    activity_name="echo",
    parameters={
        "doc": 2,
        "title": fetch_doc2["response.body.args.title"],
        "url": fetch_doc2["response.body.url"],
    },
    depends_on=["fetch_doc2"],
)

process_doc3 = Activity(
    key="process_doc3",
    worker="std",
    activity_name="echo",
    parameters={
        "doc": 3,
        "title": fetch_doc3["response.body.args.title"],
        "url": fetch_doc3["response.body.url"],
    },
    depends_on=["fetch_doc3"],
)


# === FAN-IN AGGREGATION ===
# This activity waits for ALL three process activities to complete

aggregate_results = Activity(
    key="aggregate_results",
    worker="std",
    activity_name="echo",
    parameters={
        "workflow_id": workflow.id,
        "doc1": process_doc1["echo.title"],
        "doc2": process_doc2["echo.title"],
        "doc3": process_doc3["echo.title"],
        "documents_processed": 3,
    },
    depends_on=["process_doc1", "process_doc2", "process_doc3"],
)


# === FINAL NOTIFICATION ===
# Send the summary via Mailpit

store_summary = Activity(
    key="store_summary",
    worker="std",
    activity_name="http_request",
    parameters={
        "method": "POST",
        "url": "http://mailpit:8025/api/v1/send",
        "headers": {"Content-Type": "application/json"},
        "body": {
            "From": {
                "Name": "Kruxia Flow",
                "Email": "workflow@kruxiaflow.local",
            },
            "To": [
                {
                    "Name": "Document Consumer",
                    "Email": "docs@example.com",
                }
            ],
            "Subject": f"Document Processing Summary - {workflow.id}",
            "Text": dedent(f"""\
                Documents Processed: {aggregate_results["echo.documents_processed"]}
                Doc 1: {aggregate_results["echo.doc1"]}
                Doc 2: {aggregate_results["echo.doc2"]}
                Doc 3: {aggregate_results["echo.doc3"]}
            """),
        },
    },
    depends_on=["aggregate_results"],
)


# Build the workflow
document_workflow = Workflow(
    name="process_documents",
    activities=[
        # Parallel fetch
        fetch_doc1,
        fetch_doc2,
        fetch_doc3,
        # Parallel process
        process_doc1,
        process_doc2,
        process_doc3,
        # Aggregation and storage
        aggregate_results,
        store_summary,
    ],
)

if __name__ == "__main__":
    # Print the compiled YAML to verify
    print(document_workflow)
