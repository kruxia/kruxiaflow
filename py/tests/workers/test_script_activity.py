"""Tests for kruxiaflow.workers.script_activity module."""

from uuid import uuid4

import pytest

from kruxiaflow.worker import ActivityContext, ActivityResult
from kruxiaflow.workers.script_activity import script_activity


def create_context(**kwargs) -> ActivityContext:
    """Create a test ActivityContext."""
    defaults = {
        "workflow_id": uuid4(),
        "activity_id": uuid4(),
        "activity_key": "test_script",
    }
    defaults.update(kwargs)
    return ActivityContext(**defaults)


async def execute_script(params: dict, ctx: ActivityContext) -> ActivityResult:
    """Helper to execute the script activity."""
    return await script_activity.execute(params, ctx)


class TestScriptActivity:
    """Tests for the script_activity function."""

    @pytest.mark.asyncio
    async def test_simple_script_success(self):
        """Test successful script execution with simple output."""
        ctx = create_context()
        params = {
            "script": "OUTPUT = {'result': 42}",
            "inputs": {},
        }

        result = await execute_script(params, ctx)

        assert not result.is_error
        assert result.to_output_dict() == {"result": {"result": 42}}

    @pytest.mark.asyncio
    async def test_script_with_inputs(self):
        """Test script execution using INPUT variable."""
        ctx = create_context()
        params = {
            "script": "OUTPUT = {'doubled': INPUT['value'] * 2}",
            "inputs": {"value": 21},
        }

        result = await execute_script(params, ctx)

        assert not result.is_error
        assert result.to_output_dict() == {"result": {"doubled": 42}}

    @pytest.mark.asyncio
    async def test_script_with_context_access(self):
        """Test script can access ctx, logger, workflow_id, activity_key."""
        ctx = create_context()
        params = {
            "script": """
OUTPUT = {
    'has_ctx': ctx is not None,
    'has_logger': logger is not None,
    'workflow_id': workflow_id,
    'activity_key': activity_key,
}
""",
            "inputs": {},
        }

        result = await execute_script(params, ctx)

        assert not result.is_error
        output = result.to_output_dict()["result"]
        assert output["has_ctx"] is True
        assert output["has_logger"] is True
        assert output["workflow_id"] == str(ctx.workflow_id)
        assert output["activity_key"] == ctx.activity_key

    @pytest.mark.asyncio
    async def test_script_can_import_modules(self):
        """Test script can import standard library modules."""
        ctx = create_context()
        params = {
            "script": """
import json
data = json.dumps({'key': 'value'})
OUTPUT = {'json_data': data}
""",
            "inputs": {},
        }

        result = await execute_script(params, ctx)

        assert not result.is_error
        assert result.to_output_dict() == {"result": {"json_data": '{"key": "value"}'}}

    @pytest.mark.asyncio
    async def test_missing_script_returns_error(self):
        """Test that missing script returns an error result."""
        ctx = create_context()
        params = {"inputs": {}}  # No script

        result = await execute_script(params, ctx)

        assert result.is_error
        assert result.error_code == "MISSING_SCRIPT"
        assert "No script provided" in result.error_message
        assert result.retryable is False

    @pytest.mark.asyncio
    async def test_empty_script_returns_error(self):
        """Test that empty script returns an error result."""
        ctx = create_context()
        params = {"script": "", "inputs": {}}

        result = await execute_script(params, ctx)

        assert result.is_error
        assert result.error_code == "MISSING_SCRIPT"

    @pytest.mark.asyncio
    async def test_syntax_error_returns_error(self):
        """Test that syntax error in script returns formatted error."""
        ctx = create_context()
        params = {
            "script": "def broken(\n  # missing closing paren and body",
            "inputs": {},
        }

        result = await execute_script(params, ctx)

        assert result.is_error
        assert result.error_code == "SYNTAX_ERROR"
        assert "SyntaxError" in result.error_message
        assert result.retryable is False

    @pytest.mark.asyncio
    async def test_runtime_error_returns_error(self):
        """Test that runtime error in script returns formatted error."""
        ctx = create_context()
        params = {
            "script": "x = 1 / 0  # ZeroDivisionError",
            "inputs": {},
        }

        result = await execute_script(params, ctx)

        assert result.is_error
        assert result.error_code == "SCRIPT_ERROR"
        assert "ZeroDivisionError" in result.error_message
        assert result.retryable is False

    @pytest.mark.asyncio
    async def test_name_error_returns_error(self):
        """Test that undefined variable error returns formatted error."""
        ctx = create_context()
        params = {
            "script": "OUTPUT = {'value': undefined_variable}",
            "inputs": {},
        }

        result = await execute_script(params, ctx)

        assert result.is_error
        assert result.error_code == "SCRIPT_ERROR"
        assert "NameError" in result.error_message

    @pytest.mark.asyncio
    async def test_script_without_output_returns_empty_dict(self):
        """Test script that doesn't set OUTPUT returns empty dict."""
        ctx = create_context()
        params = {
            "script": "x = 1 + 1  # no OUTPUT assignment",
            "inputs": {},
        }

        result = await execute_script(params, ctx)

        assert not result.is_error
        assert result.to_output_dict() == {"result": {}}

    @pytest.mark.asyncio
    async def test_script_modifies_output_dict(self):
        """Test script that modifies pre-initialized OUTPUT dict."""
        ctx = create_context()
        params = {
            "script": """
OUTPUT['key1'] = 'value1'
OUTPUT['key2'] = 'value2'
""",
            "inputs": {},
        }

        result = await execute_script(params, ctx)

        assert not result.is_error
        assert result.to_output_dict() == {
            "result": {"key1": "value1", "key2": "value2"}
        }

    @pytest.mark.asyncio
    async def test_default_inputs_is_empty_dict(self):
        """Test that missing inputs defaults to empty dict."""
        ctx = create_context()
        params = {
            "script": "OUTPUT = {'input_type': type(INPUT).__name__, 'input_len': len(INPUT)}",
            # No 'inputs' key
        }

        result = await execute_script(params, ctx)

        assert not result.is_error
        output = result.to_output_dict()["result"]
        assert output["input_type"] == "dict"
        assert output["input_len"] == 0

    @pytest.mark.asyncio
    async def test_complex_data_types(self):
        """Test script handling complex data types."""
        ctx = create_context()
        params = {
            "script": """
OUTPUT = {
    'list': [1, 2, 3],
    'nested': {'a': {'b': 'c'}},
    'mixed': [{'x': 1}, {'y': 2}],
}
""",
            "inputs": {},
        }

        result = await execute_script(params, ctx)

        assert not result.is_error
        output = result.to_output_dict()["result"]
        assert output["list"] == [1, 2, 3]
        assert output["nested"] == {"a": {"b": "c"}}
        assert output["mixed"] == [{"x": 1}, {"y": 2}]
