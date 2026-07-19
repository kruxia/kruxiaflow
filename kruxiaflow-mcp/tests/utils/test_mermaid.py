"""Tests for Mermaid diagram generator."""

import pytest

from kruxiaflow_mcp.utils.mermaid import (
    generate_cost_breakdown_diagram,
    generate_workflow_diagram,
)


def test_generate_workflow_diagram_simple() -> None:
    """Test generating a simple workflow diagram."""
    workflow_def = {
        "name": "simple_workflow",
        "activities": [
            {
                "key": "fetch_data",
                "activity_name": "http_request",
                "parameters": {"url": "https://api.example.com"},
            },
            {
                "key": "process_data",
                "activity_name": "llm_prompt",
                "parameters": {"prompt": "Process data"},
                "depends_on": ["fetch_data"],
            },
        ],
    }

    diagram = generate_workflow_diagram(workflow_def)

    # Verify diagram structure
    assert "flowchart TB" in diagram
    assert "start([Start])" in diagram
    assert "complete([Complete])" in diagram
    assert "fetch_data[fetch_data<br/>http_request]" in diagram
    assert "process_data[process_data<br/>llm_prompt]" in diagram
    assert "fetch_data --> process_data" in diagram
    assert "start --> fetch_data" in diagram
    assert "process_data --> complete" in diagram


def test_generate_workflow_diagram_parallel() -> None:
    """Test generating a diagram with parallel activities."""
    workflow_def = {
        "name": "parallel_workflow",
        "activities": [
            {"key": "fetch_a", "activity_name": "http_request"},
            {"key": "fetch_b", "activity_name": "http_request"},
            {
                "key": "combine",
                "activity_name": "llm_prompt",
                "depends_on": ["fetch_a", "fetch_b"],
            },
        ],
    }

    diagram = generate_workflow_diagram(workflow_def)

    # Verify parallel structure
    assert "start --> fetch_a" in diagram
    assert "start --> fetch_b" in diagram
    assert "fetch_a --> combine" in diagram
    assert "fetch_b --> combine" in diagram
    assert "combine --> complete" in diagram


def test_generate_workflow_diagram_with_status() -> None:
    """Test generating a diagram with execution status colors."""
    workflow_def = {
        "name": "status_workflow",
        "activities": [
            {"key": "completed_activity", "activity_name": "http_request"},
            {
                "key": "running_activity",
                "activity_name": "llm_prompt",
                "depends_on": ["completed_activity"],
            },
        ],
    }

    execution_status = {
        "status": "running",
        "activities": [
            {"key": "completed_activity", "status": "completed"},
            {"key": "running_activity", "status": "running"},
        ],
    }

    diagram = generate_workflow_diagram(workflow_def, execution_status)

    # Verify status colors
    assert "style completed_activity fill:#90EE90" in diagram  # Green
    assert "style running_activity fill:#FFD700" in diagram  # Gold
    assert "style start fill:#90EE90" in diagram  # Green start


def test_generate_workflow_diagram_failed_status() -> None:
    """Test diagram with failed activity."""
    workflow_def = {
        "name": "failed_workflow",
        "activities": [{"key": "failed_activity", "activity_name": "http_request"}],
    }

    execution_status = {
        "status": "failed",
        "activities": [{"key": "failed_activity", "status": "failed"}],
    }

    diagram = generate_workflow_diagram(workflow_def, execution_status)

    # Verify failed color
    assert "style failed_activity fill:#FF6B6B" in diagram  # Red


def test_generate_cost_breakdown_diagram() -> None:
    """Test generating a cost breakdown diagram."""
    cost_data = {
        "total_cost_usd": 0.045,
        "activities": [
            {
                "activity_key": "ask_question",
                "cost_usd": 0.042,
                "provider": "anthropic",
            },
            {
                "activity_key": "store_response",
                "cost_usd": 0.003,
                "provider": "postgres",
            },
        ],
    }

    diagram = generate_cost_breakdown_diagram(cost_data)

    # Verify diagram structure
    assert "graph LR" in diagram
    assert 'Total["Total: $0.0450"]' in diagram
    assert "ask_question: $0.0420" in diagram
    assert "store_response: $0.0030" in diagram
    assert "anthropic" in diagram


def test_generate_cost_breakdown_with_zero_costs() -> None:
    """Test cost diagram filters out zero-cost activities."""
    cost_data = {
        "total_cost_usd": 0.042,
        "activities": [
            {"activity_key": "llm_activity", "cost_usd": 0.042, "provider": "anthropic"},
            {"activity_key": "http_activity", "cost_usd": 0.0, "provider": ""},
        ],
    }

    diagram = generate_cost_breakdown_diagram(cost_data)

    # Verify zero-cost activities are not included
    assert "llm_activity" in diagram
    assert "http_activity" not in diagram


def test_generate_workflow_diagram_activity_keys_with_hyphens() -> None:
    """Test that activity keys with special characters are handled correctly."""
    workflow_def = {
        "name": "special_chars_workflow",
        "activities": [
            {"key": "fetch-data-api", "activity_name": "http_request"},
            {"key": "process_data", "activity_name": "llm_prompt"},
        ],
    }

    diagram = generate_workflow_diagram(workflow_def)

    # Diagram should include the keys as-is (Mermaid handles them)
    assert "fetch-data-api[" in diagram
    assert "process_data[" in diagram
