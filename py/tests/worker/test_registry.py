"""Tests for activity registry."""

import asyncio
from uuid import uuid4

import pytest

from kruxiaflow.worker import (
    Activity,
    ActivityContext,
    ActivityNotFoundError,
    ActivityRegistry,
    ActivityResult,
    ActivityTimeoutError,
    activity,
)


@activity(name="echo")
async def echo_activity(params: dict, ctx: ActivityContext) -> ActivityResult:
    """Simple echo activity for testing."""
    return ActivityResult.value("result", params.get("input", ""))


@activity(name="double")
async def double_activity(params: dict, ctx: ActivityContext) -> ActivityResult:
    """Double the input value."""
    return ActivityResult.value("result", params["value"] * 2)


@activity(name="slow")
async def slow_activity(params: dict, ctx: ActivityContext) -> ActivityResult:
    """Activity that sleeps for testing timeout."""
    await asyncio.sleep(params.get("sleep", 10))
    return ActivityResult.value("result", "done")


@activity(name="failing")
async def failing_activity(params: dict, ctx: ActivityContext) -> ActivityResult:
    """Activity that always raises an exception."""
    raise ValueError("Intentional failure")


@activity(name="custom_activity")
async def custom_worker_activity(params: dict, ctx: ActivityContext) -> ActivityResult:
    """Activity for a custom worker type."""
    return ActivityResult.value("result", "custom")


class TestActivityRegistryBasic:
    """Test ActivityRegistry basic functionality."""

    def test_register_activity(self):
        registry = ActivityRegistry()
        registry.register(echo_activity, "python")
        assert registry.get("python", "echo") is echo_activity

    def test_register_multiple_activities(self):
        registry = ActivityRegistry()
        registry.register(echo_activity, "python")
        registry.register(double_activity, "python")

        assert registry.get("python", "echo") is echo_activity
        assert registry.get("python", "double") is double_activity

    def test_get_nonexistent_activity(self):
        registry = ActivityRegistry()
        assert registry.get("python", "nonexistent") is None

    def test_get_wrong_worker(self):
        registry = ActivityRegistry()
        registry.register(echo_activity, "python")
        assert registry.get("wrong_worker", "echo") is None

    def test_activity_types(self):
        registry = ActivityRegistry()
        registry.register(echo_activity, "python")
        registry.register(double_activity, "python")

        types = registry.activity_types()
        assert "python.echo" in types
        assert "python.double" in types
        assert len(types) == 2

    def test_activity_types_empty(self):
        registry = ActivityRegistry()
        assert registry.activity_types() == []

    def test_register_different_workers(self):
        registry = ActivityRegistry()
        registry.register(echo_activity, "python")
        registry.register(custom_worker_activity, "custom")

        types = registry.activity_types()
        assert "python.echo" in types
        assert "custom.custom_activity" in types


class TestActivityRegistryExecution:
    """Test ActivityRegistry execution."""

    @pytest.mark.asyncio
    async def test_execute_activity(self):
        registry = ActivityRegistry()
        registry.register(echo_activity, "python")

        ctx = ActivityContext(
            workflow_id=uuid4(),
            activity_id=uuid4(),
            activity_key="test",
        )
        result = await registry.execute(
            worker="python",
            name="echo",
            params={"input": "hello"},
            ctx=ctx,
            timeout=10.0,
        )

        assert result.to_output_dict() == {"result": "hello"}

    @pytest.mark.asyncio
    async def test_execute_with_computation(self):
        registry = ActivityRegistry()
        registry.register(double_activity, "python")

        ctx = ActivityContext(
            workflow_id=uuid4(),
            activity_id=uuid4(),
            activity_key="test",
        )
        result = await registry.execute(
            worker="python",
            name="double",
            params={"value": 21},
            ctx=ctx,
            timeout=10.0,
        )

        assert result.to_output_dict() == {"result": 42}

    @pytest.mark.asyncio
    async def test_execute_activity_not_found(self):
        registry = ActivityRegistry()

        ctx = ActivityContext(
            workflow_id=uuid4(),
            activity_id=uuid4(),
            activity_key="test",
        )

        with pytest.raises(
            ActivityNotFoundError,
            match=r"Activity implementation not found: python\.nonexistent",
        ):
            await registry.execute(
                worker="python",
                name="nonexistent",
                params={},
                ctx=ctx,
                timeout=10.0,
            )

    @pytest.mark.asyncio
    async def test_execute_timeout(self):
        registry = ActivityRegistry()
        registry.register(slow_activity, "python")

        ctx = ActivityContext(
            workflow_id=uuid4(),
            activity_id=uuid4(),
            activity_key="test",
        )

        with pytest.raises(
            ActivityTimeoutError,
            match=r"Activity execution timed out after 0\.1s",
        ):
            await registry.execute(
                worker="python",
                name="slow",
                params={"sleep": 10},
                ctx=ctx,
                timeout=0.1,  # Very short timeout
            )

    @pytest.mark.asyncio
    async def test_execute_propagates_exception(self):
        registry = ActivityRegistry()
        registry.register(failing_activity, "python")

        ctx = ActivityContext(
            workflow_id=uuid4(),
            activity_id=uuid4(),
            activity_key="test",
        )

        with pytest.raises(ValueError, match="Intentional failure"):
            await registry.execute(
                worker="python",
                name="failing",
                params={},
                ctx=ctx,
                timeout=10.0,
            )


class TestActivityRegistryWithClassBasedActivity:
    """Test ActivityRegistry with class-based activities."""

    @pytest.mark.asyncio
    async def test_register_class_activity(self):
        class AddActivity(Activity):
            @property
            def name(self) -> str:
                return "add"

            async def execute(
                self, params: dict, ctx: ActivityContext
            ) -> ActivityResult:
                result = params["a"] + params["b"]
                return ActivityResult.value("sum", result)

        registry = ActivityRegistry()
        add_activity = AddActivity()
        registry.register(add_activity, "python")

        ctx = ActivityContext(
            workflow_id=uuid4(),
            activity_id=uuid4(),
            activity_key="test",
        )
        result = await registry.execute(
            worker="python",
            name="add",
            params={"a": 3, "b": 5},
            ctx=ctx,
            timeout=10.0,
        )

        assert result.to_output_dict() == {"sum": 8}
