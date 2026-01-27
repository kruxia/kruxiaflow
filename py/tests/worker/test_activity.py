"""Tests for activity interface and result types."""

from decimal import Decimal
from uuid import uuid4

import pytest

from kruxiaflow.worker import (
    Activity,
    ActivityContext,
    ActivityOutput,
    ActivityResult,
    OutputType,
    activity,
)


class TestOutputType:
    """Test OutputType enum."""

    def test_value_type(self):
        assert OutputType.VALUE == "value"

    def test_file_type(self):
        assert OutputType.FILE == "file"

    def test_folder_type(self):
        assert OutputType.FOLDER == "folder"


class TestActivityOutput:
    """Test ActivityOutput model."""

    def test_value_output(self):
        output = ActivityOutput(name="result", output_type=OutputType.VALUE, value=42)
        assert output.name == "result"
        assert output.output_type == OutputType.VALUE
        assert output.value == 42

    def test_file_output(self):
        output = ActivityOutput(
            name="data",
            output_type=OutputType.FILE,
            value="postgres://workflow/activity/file.csv",
        )
        assert output.name == "data"
        assert output.output_type == OutputType.FILE
        assert output.value == "postgres://workflow/activity/file.csv"

    def test_complex_value_output(self):
        value = {"nested": {"data": [1, 2, 3]}, "flag": True}
        output = ActivityOutput(
            name="complex", output_type=OutputType.VALUE, value=value
        )
        assert output.value == value


class TestActivityResult:
    """Test ActivityResult model."""

    def test_value_constructor(self):
        result = ActivityResult.value("output", 42)
        assert len(result.outputs) == 1
        assert result.outputs[0].name == "output"
        assert result.outputs[0].output_type == OutputType.VALUE
        assert result.outputs[0].value == 42

    def test_value_with_string(self):
        result = ActivityResult.value("message", "hello world")
        assert result.outputs[0].value == "hello world"

    def test_value_with_dict(self):
        data = {"key": "value", "count": 10}
        result = ActivityResult.value("data", data)
        assert result.outputs[0].value == data

    def test_value_with_list(self):
        items = [1, 2, 3, "four"]
        result = ActivityResult.value("items", items)
        assert result.outputs[0].value == items

    def test_values_constructor(self):
        outputs = [
            ActivityOutput(name="a", output_type=OutputType.VALUE, value=1),
            ActivityOutput(name="b", output_type=OutputType.VALUE, value=2),
        ]
        result = ActivityResult.values(outputs)
        assert len(result.outputs) == 2
        assert result.outputs[0].name == "a"
        assert result.outputs[1].name == "b"

    def test_with_cost(self):
        result = ActivityResult.value("result", "done").with_cost(Decimal("0.05"))
        assert result.cost_usd == Decimal("0.05")

    def test_with_cost_chaining(self):
        result = ActivityResult.value("result", "done")
        chained = result.with_cost(Decimal("0.10"))
        assert chained is result  # Returns self

    def test_with_metadata(self):
        metadata = {"model": "gpt-4", "tokens": 1500}
        result = ActivityResult.value("result", "done").with_metadata(metadata)
        assert result.metadata == metadata

    def test_with_metadata_chaining(self):
        result = ActivityResult.value("result", "done")
        chained = result.with_metadata({"key": "value"})
        assert chained is result  # Returns self

    def test_chaining_cost_and_metadata(self):
        result = (
            ActivityResult.value("result", "done")
            .with_cost(Decimal("0.01"))
            .with_metadata({"model": "claude"})
        )
        assert result.cost_usd == Decimal("0.01")
        assert result.metadata == {"model": "claude"}

    def test_error_constructor(self):
        result = ActivityResult.error("Something went wrong")
        assert result.is_error is True
        assert result.error_message == "Something went wrong"
        assert result.error_code == "EXECUTION_ERROR"
        assert result.retryable is True

    def test_error_with_custom_code(self):
        result = ActivityResult.error("Rate limited", code="RATE_LIMIT")
        assert result.error_code == "RATE_LIMIT"

    def test_error_non_retryable(self):
        result = ActivityResult.error("Invalid input", retryable=False)
        assert result.retryable is False

    def test_is_error_false_for_success(self):
        result = ActivityResult.value("result", "success")
        assert result.is_error is False

    def test_error_properties_none_for_success(self):
        result = ActivityResult.value("result", "success")
        assert result.error_message is None
        assert result.error_code is None

    def test_to_output_dict(self):
        result = ActivityResult.values(
            [
                ActivityOutput(name="a", output_type=OutputType.VALUE, value=1),
                ActivityOutput(name="b", output_type=OutputType.VALUE, value="two"),
                ActivityOutput(
                    name="c", output_type=OutputType.FILE, value="postgres://..."
                ),
            ]
        )
        output_dict = result.to_output_dict()
        # Should only include VALUE outputs
        assert output_dict == {"a": 1, "b": "two"}

    def test_to_output_dict_single_value(self):
        result = ActivityResult.value("result", {"status": "ok"})
        assert result.to_output_dict() == {"result": {"status": "ok"}}

    def test_default_empty_outputs(self):
        result = ActivityResult()
        assert result.outputs == []
        assert result.cost_usd is None
        assert result.metadata is None


class TestActivityDecorator:
    """Test @activity decorator."""

    def test_basic_decorator(self):
        @activity()
        async def my_activity(params: dict, ctx: ActivityContext) -> ActivityResult:
            return ActivityResult.value("result", params["input"])

        assert isinstance(my_activity, Activity)
        assert my_activity.name == "my_activity"

    def test_custom_name(self):
        @activity(name="custom_name")
        async def my_activity(params: dict, ctx: ActivityContext) -> ActivityResult:
            return ActivityResult.value("result", "done")

        assert my_activity.name == "custom_name"

    @pytest.mark.asyncio
    async def test_execute_decorated_activity(self):
        @activity()
        async def double(params: dict, ctx: ActivityContext) -> ActivityResult:
            return ActivityResult.value("result", params["value"] * 2)

        ctx = ActivityContext(
            workflow_id=uuid4(),
            activity_id=uuid4(),
            activity_key="test_key",
        )
        result = await double.execute({"value": 21}, ctx)
        assert result.to_output_dict() == {"result": 42}

    @pytest.mark.asyncio
    async def test_execute_with_context_access(self):
        @activity()
        async def log_ids(params: dict, ctx: ActivityContext) -> ActivityResult:
            return ActivityResult.value(
                "result",
                {
                    "workflow_id": str(ctx.workflow_id),
                    "activity_key": ctx.activity_key,
                },
            )

        workflow_id = uuid4()
        ctx = ActivityContext(
            workflow_id=workflow_id,
            activity_id=uuid4(),
            activity_key="test_activity",
        )
        result = await log_ids.execute({}, ctx)
        output = result.to_output_dict()["result"]
        assert output["workflow_id"] == str(workflow_id)
        assert output["activity_key"] == "test_activity"


class TestActivityABC:
    """Test Activity abstract base class."""

    def test_subclass_must_implement_execute(self):
        with pytest.raises(TypeError, match="execute"):

            class IncompleteActivity(Activity):
                @property
                def name(self) -> str:
                    return "incomplete"

            IncompleteActivity()

    def test_subclass_must_implement_name(self):
        with pytest.raises(TypeError, match="name"):

            class IncompleteActivity(Activity):
                async def execute(
                    self, params: dict, ctx: ActivityContext
                ) -> ActivityResult:
                    return ActivityResult.value("result", "done")

            IncompleteActivity()

    @pytest.mark.asyncio
    async def test_complete_subclass(self):
        class MyActivity(Activity):
            @property
            def name(self) -> str:
                return "my_activity"

            async def execute(
                self, params: dict, ctx: ActivityContext
            ) -> ActivityResult:
                return ActivityResult.value("result", params["x"] + 1)

        activity_impl = MyActivity()
        assert activity_impl.name == "my_activity"

        ctx = ActivityContext(
            workflow_id=uuid4(),
            activity_id=uuid4(),
            activity_key="test",
        )
        result = await activity_impl.execute({"x": 10}, ctx)
        assert result.to_output_dict() == {"result": 11}


class TestActivityDecoratorEdgeCases:
    """Test edge cases for activity decorator."""

    def test_activity_decorator_raises_when_name_not_determined(self):
        """Test that decorator raises ValueError when activity name cannot be determined."""

        # Create a callable object without __name__ attribute
        class NoNameCallable:
            async def __call__(
                self, params: dict, ctx: ActivityContext
            ) -> ActivityResult:
                return ActivityResult.value("result", "done")

        # Remove __name__ if it exists (class instances don't have __name__ by default)
        callable_without_name = NoNameCallable()

        # Verify it doesn't have __name__
        assert not hasattr(callable_without_name, "__name__")

        with pytest.raises(ValueError, match="Activity name could not be determined"):
            activity()(callable_without_name)
