"""Document Processing Workflow - Parallel execution (fan-out/fan-in) example.

This example demonstrates:
- Parallel activity execution (fan-out)
- Multiple dependencies for aggregation (fan-in)
- Referencing outputs from multiple upstream activities
- HTTP POST requests with body data
"""

from kruxiaflow import Activity, Input, Workflow, workflow

# Define workflow inputs
doc1_url = Input("doc1_url", type=str, required=True)
doc2_url = Input("doc2_url", type=str, required=True)
doc3_url = Input("doc3_url", type=str, required=True)
processing_service_url = Input("processing_service_url", type=str, required=True)
aggregator_url = Input("aggregator_url", type=str, required=True)
storage_webhook_url = Input("storage_webhook_url", type=str, required=True)


# === PARALLEL FETCH (Fan-Out) ===
# These three activities have no dependencies and execute in parallel

fetch_doc1 = (
    Activity(key="fetch_doc1")
    .with_worker("builtin", "http_request")
    .with_params(method="GET", url=doc1_url)
)

fetch_doc2 = (
    Activity(key="fetch_doc2")
    .with_worker("builtin", "http_request")
    .with_params(method="GET", url=doc2_url)
)

fetch_doc3 = (
    Activity(key="fetch_doc3")
    .with_worker("builtin", "http_request")
    .with_params(method="GET", url=doc3_url)
)


# === PARALLEL PROCESS (Fan-Out) ===
# Each process activity depends on its corresponding fetch activity
# These also execute in parallel (independent dependencies)

process_doc1 = (
    Activity(key="process_doc1")
    .with_worker("builtin", "http_request")
    .with_params(
        method="POST",
        url=processing_service_url,
        body={
            "document_data": fetch_doc1["response.body"],
            "operation": "extract_text",
            "language": "en",
        },
    )
    .with_dependencies(fetch_doc1)
)

process_doc2 = (
    Activity(key="process_doc2")
    .with_worker("builtin", "http_request")
    .with_params(
        method="POST",
        url=processing_service_url,
        body={
            "document_data": fetch_doc2["response.body"],
            "operation": "extract_text",
            "language": "en",
        },
    )
    .with_dependencies(fetch_doc2)
)

process_doc3 = (
    Activity(key="process_doc3")
    .with_worker("builtin", "http_request")
    .with_params(
        method="POST",
        url=processing_service_url,
        body={
            "document_data": fetch_doc3["response.body"],
            "operation": "extract_text",
            "language": "en",
        },
    )
    .with_dependencies(fetch_doc3)
)


# === FAN-IN AGGREGATION ===
# This activity waits for ALL three process activities to complete

aggregate_results = (
    Activity(key="aggregate_results")
    .with_worker("builtin", "http_request")
    .with_params(
        method="POST",
        url=aggregator_url,
        body={
            "workflow_id": workflow.id,
            "doc1_result": process_doc1["response.body"],
            "doc2_result": process_doc2["response.body"],
            "doc3_result": process_doc3["response.body"],
            "operation": "summarize",
        },
    )
    .with_dependencies(
        process_doc1, process_doc2, process_doc3
    )  # Fan-in: waits for all three
)


# === FINAL STORAGE ===
# Store the final summary

store_summary = (
    Activity(key="store_summary")
    .with_worker("builtin", "http_request")
    .with_params(
        method="POST",
        url=storage_webhook_url,
        body={
            "workflow_id": workflow.id,
            "summary": aggregate_results["response.body"],
            "document_count": 3,
        },
    )
    .with_dependencies(aggregate_results)
)


# Build the workflow
document_workflow = (
    Workflow(name="process_documents")
    .with_inputs(
        doc1_url,
        doc2_url,
        doc3_url,
        processing_service_url,
        aggregator_url,
        storage_webhook_url,
    )
    .with_activities(
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
    )
)

if __name__ == "__main__":
    # Print the compiled YAML to verify
    print(document_workflow.to_yaml())
