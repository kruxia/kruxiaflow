"""Tests for execution tools."""

import yaml

import pytest

from kruxiaflow_mcp.schemas.workflow import WorkflowDefinition


def test_validate_workflow_valid() -> None:
    """Test validating a valid workflow using schema."""
    workflow_data = yaml.safe_load("""
name: test_workflow
activities:
  - key: fetch_data
    activity_name: http_request
    parameters:
      url: "https://api.example.com"
  - key: process_data
    activity_name: llm_prompt
    parameters:
      prompt: "Process data"
    depends_on:
      - fetch_data
""")

    workflow = WorkflowDefinition(**workflow_data)
    errors = workflow.validate_dependencies()

    assert len(errors) == 0
    assert len(workflow.activities) == 2
    assert workflow.activities[1].depends_on == ["fetch_data"]


def test_validate_workflow_circular_dependency() -> None:
    """Test detecting circular dependencies."""
    workflow_data = yaml.safe_load("""
name: circular_workflow
activities:
  - key: activity_a
    activity_name: http_request
    depends_on:
      - activity_b
  - key: activity_b
    activity_name: http_request
    depends_on:
      - activity_a
""")

    workflow = WorkflowDefinition(**workflow_data)
    errors = workflow.validate_dependencies()

    assert len(errors) > 0
    assert any("circular" in error.lower() for error in errors)


def test_validate_workflow_undefined_dependency() -> None:
    """Test detecting undefined dependencies."""
    workflow_data = yaml.safe_load("""
name: undefined_dep_workflow
activities:
  - key: activity_a
    activity_name: http_request
    depends_on:
      - nonexistent_activity
""")

    workflow = WorkflowDefinition(**workflow_data)
    errors = workflow.validate_dependencies()

    assert len(errors) > 0
    assert any("undefined activity" in error.lower() for error in errors)
