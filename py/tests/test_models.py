"""Tests for kruxiaflow.models module."""

import pytest

from kruxiaflow.models import (
    Activity,
    ActivitySettings,
    BackoffStrategy,
    BudgetAction,
    BudgetSettings,
    CacheSettings,
    Dependency,
    RetrySettings,
    ScriptActivity,
    Workflow,
    script,
)


class TestRetrySettings:
    """Tests for RetrySettings model."""

    def test_default_values(self):
        settings = RetrySettings()
        assert settings.max_attempts == 3
        assert settings.strategy == BackoffStrategy.EXPONENTIAL

    def test_custom_values(self):
        settings = RetrySettings(
            max_attempts=5,
            strategy=BackoffStrategy.FIXED,
            base_seconds=1.0,
            factor=2.0,
            max_seconds=60.0,
        )
        assert settings.max_attempts == 5
        assert settings.strategy == BackoffStrategy.FIXED
        assert settings.base_seconds == 1.0
        assert settings.factor == 2.0
        assert settings.max_seconds == 60.0


class TestCacheSettings:
    """Tests for CacheSettings model."""

    def test_required_ttl(self):
        settings = CacheSettings(ttl=3600)
        assert settings.ttl == 3600
        assert settings.enabled is True
        assert settings.key is None

    def test_custom_key(self):
        settings = CacheSettings(ttl=3600, key="custom_key")
        assert settings.key == "custom_key"


class TestBudgetSettings:
    """Tests for BudgetSettings model."""

    def test_default_action(self):
        settings = BudgetSettings(limit=10.0)
        assert settings.limit == 10.0
        assert settings.action == BudgetAction.ABORT

    def test_custom_action(self):
        settings = BudgetSettings(limit=5.0, action=BudgetAction.CONTINUE)
        assert settings.action == BudgetAction.CONTINUE


class TestActivitySettings:
    """Tests for ActivitySettings model."""

    def test_all_none_by_default(self):
        settings = ActivitySettings()
        assert settings.timeout_seconds is None
        assert settings.retry is None
        assert settings.cache is None
        assert settings.budget is None
        assert settings.delay is None
        assert settings.streaming is None

    def test_with_all_settings(self):
        settings = ActivitySettings(
            timeout_seconds=300,
            retry=RetrySettings(max_attempts=5),
            cache=CacheSettings(ttl=3600),
            budget=BudgetSettings(limit=1.0),
            delay="5s",
            streaming=True,
        )
        assert settings.timeout_seconds == 300
        assert settings.retry.max_attempts == 5
        assert settings.cache.ttl == 3600
        assert settings.budget.limit == 1.0
        assert settings.delay == "5s"
        assert settings.streaming is True


class TestDependency:
    """Tests for Dependency model."""

    def test_simple_dependency(self):
        dep = Dependency(activity_key="step1")
        assert dep.activity_key == "step1"
        assert dep.conditions == []

    def test_dependency_with_conditions(self):
        dep = Dependency(
            activity_key="step1",
            conditions=["{{ step1.status }} == 'succeeded'"],
        )
        assert dep.activity_key == "step1"
        assert len(dep.conditions) == 1

    def test_dependency_on_from_string(self):
        dep = Dependency.on("step1")
        assert dep.activity_key == "step1"
        assert dep.conditions == []

    def test_dependency_on_from_activity(self):
        activity = Activity(key="step1", activity_name="echo")
        dep = Dependency.on(activity)
        assert dep.activity_key == "step1"

    def test_dependency_on_with_conditions(self):
        activity = Activity(key="step1", activity_name="echo")
        dep = Dependency.on(activity, activity["status"] == "success")
        assert dep.activity_key == "step1"
        assert len(dep.conditions) == 1
        assert "step1.status == 'success'" in dep.conditions[0]


class TestActivity:
    """Tests for Activity model."""

    def test_minimal_activity(self):
        activity = Activity(key="test", activity_name="echo")
        assert activity.key == "test"
        assert activity.worker == "std"
        assert activity.activity_name == "echo"
        assert activity.parameters == {}
        assert activity.depends_on == []

    def test_full_activity(self):
        activity = Activity(
            key="fetch",
            worker="std",
            activity_name="http_request",
            parameters={"url": "https://example.com"},
            settings=ActivitySettings(timeout_seconds=60),
            depends_on=["step1"],
        )
        assert activity.key == "fetch"
        assert activity.worker == "std"
        assert activity.activity_name == "http_request"
        assert activity.parameters["url"] == "https://example.com"
        assert activity.settings.timeout_seconds == 60
        assert "step1" in activity.depends_on


class TestActivityDeclarativeConstruction:
    """Tests for Activity declarative construction."""

    def test_with_all_parameters(self):
        activity = Activity(
            key="test",
            worker="std",
            activity_name="echo",
            parameters={"url": "https://example.com", "method": "GET"},
        )
        assert activity.parameters["url"] == "https://example.com"
        assert activity.parameters["method"] == "GET"

    def test_with_settings(self):
        activity = Activity(
            key="test",
            activity_name="echo",
            settings=ActivitySettings(
                timeout_seconds=300,
                retry=RetrySettings(max_attempts=5, strategy=BackoffStrategy.FIXED),
                cache=CacheSettings(ttl=3600, key="my_key"),
                budget=BudgetSettings(limit=10.0),
                delay="5s",
                streaming=True,
            ),
        )
        assert activity.settings.timeout_seconds == 300
        assert activity.settings.retry.max_attempts == 5
        assert activity.settings.retry.strategy == BackoffStrategy.FIXED
        assert activity.settings.cache.ttl == 3600
        assert activity.settings.cache.key == "my_key"
        assert activity.settings.budget.limit == 10.0
        assert activity.settings.budget.action == BudgetAction.ABORT
        assert activity.settings.delay == "5s"
        assert activity.settings.streaming is True

    def test_with_dependencies_from_strings(self):
        activity = Activity(
            key="test",
            activity_name="echo",
            depends_on=["step1", "step2"],
        )
        assert "step1" in activity.depends_on
        assert "step2" in activity.depends_on

    def test_with_dependencies_from_dependency_objects(self):
        dep = Dependency(activity_key="step1", conditions=["{{ step1.ok }} == true"])
        activity = Activity(
            key="test",
            activity_name="echo",
            depends_on=[dep],
        )
        assert len(activity.depends_on) == 1
        assert isinstance(activity.depends_on[0], Dependency)

    def test_with_dependencies_mixed(self):
        dep = Dependency(activity_key="step2", conditions=["condition"])
        activity = Activity(
            key="test",
            activity_name="echo",
            depends_on=["step1", dep, "step3"],
        )
        assert len(activity.depends_on) == 3


class TestActivityOutputReference:
    """Tests for Activity output reference via subscript."""

    def test_getitem_returns_output_ref(self):
        from kruxiaflow.expressions import OutputRef

        activity = Activity(key="analyze", activity_name="sentiment")
        ref = activity["sentiment"]
        assert isinstance(ref, OutputRef)

    def test_output_ref_string(self):
        activity = Activity(key="analyze", activity_name="sentiment")
        ref = activity["sentiment"]
        assert str(ref) == "{{analyze.sentiment}}"

    def test_output_ref_nested_path(self):
        activity = Activity(key="fetch", activity_name="http_request")
        ref = activity["response.body.data"]
        assert str(ref) == "{{fetch.response.body.data}}"

    def test_failed_property(self):
        activity = Activity(key="process", activity_name="echo")
        assert "process.status == 'failed'" in activity.failed

    def test_succeeded_property(self):
        activity = Activity(key="process", activity_name="echo")
        assert "process.status == 'succeeded'" in activity.succeeded


class TestActivitySerialization:
    """Tests for Activity.to_dict() serialization."""

    def test_minimal_activity_to_dict(self):
        activity = Activity(key="test", activity_name="echo")
        d = activity.to_dict()
        assert d["key"] == "test"
        assert d["worker"] == "std"
        assert d["activity_name"] == "echo"
        assert "parameters" not in d
        assert "settings" not in d
        assert "depends_on" not in d

    def test_activity_with_parameters_to_dict(self):
        activity = Activity(
            key="test",
            activity_name="echo",
            parameters={"url": "https://example.com"},
        )
        d = activity.to_dict()
        assert d["parameters"]["url"] == "https://example.com"

    def test_activity_with_settings_to_dict(self):
        activity = Activity(
            key="test",
            activity_name="echo",
            settings=ActivitySettings(
                timeout_seconds=300,
                retry=RetrySettings(max_attempts=5, strategy=BackoffStrategy.FIXED),
            ),
        )
        d = activity.to_dict()
        assert d["settings"]["timeout_seconds"] == 300
        assert d["settings"]["retry"]["max_attempts"] == 5
        assert d["settings"]["retry"]["strategy"] == "fixed"

    def test_activity_with_simple_dependencies_to_dict(self):
        activity = Activity(
            key="test",
            activity_name="echo",
            depends_on=["step1", "step2"],
        )
        d = activity.to_dict()
        assert d["depends_on"] == ["step1", "step2"]

    def test_activity_with_conditional_dependency_to_dict(self):
        dep = Dependency(activity_key="step1", conditions=["{{ step1.ok }} == true"])
        activity = Activity(
            key="test",
            activity_name="echo",
            depends_on=[dep],
        )
        d = activity.to_dict()
        assert len(d["depends_on"]) == 1
        assert d["depends_on"][0]["activity_key"] == "step1"
        assert d["depends_on"][0]["conditions"] == ["{{ step1.ok }} == true"]

    def test_activity_with_dependency_object_no_conditions_to_dict(self):
        """Dependency object without conditions should serialize to string."""
        dep = Dependency(activity_key="step1")
        activity = Activity(
            key="test",
            activity_name="echo",
            depends_on=[dep],
        )
        d = activity.to_dict()
        assert d["depends_on"] == ["step1"]

    def test_activity_with_full_retry_settings_to_dict(self):
        """Test retry settings with all optional fields."""
        activity = Activity(
            key="test",
            activity_name="echo",
            settings=ActivitySettings(
                retry=RetrySettings(
                    max_attempts=5,
                    strategy=BackoffStrategy.EXPONENTIAL,
                    base_seconds=1.0,
                    factor=2.0,
                    max_seconds=60.0,
                ),
            ),
        )
        d = activity.to_dict()
        retry = d["settings"]["retry"]
        assert retry["max_attempts"] == 5
        assert retry["strategy"] == "exponential"
        assert retry["base_seconds"] == 1.0
        assert retry["factor"] == 2.0
        assert retry["max_seconds"] == 60.0

    def test_activity_with_scheduled_for_to_dict(self):
        """Test scheduled_for setting serialization."""
        activity = Activity(
            key="test",
            activity_name="echo",
            settings=ActivitySettings(scheduled_for="2024-01-01T00:00:00Z"),
        )
        d = activity.to_dict()
        assert d["settings"]["scheduled_for"] == "2024-01-01T00:00:00Z"

    def test_activity_with_iteration_scoped_to_dict(self):
        """Test iteration_scoped setting serialization."""
        activity = Activity(
            key="test",
            activity_name="echo",
            settings=ActivitySettings(iteration_scoped=True),
        )
        d = activity.to_dict()
        assert d["settings"]["iteration_scoped"] is True


class TestWorkflow:
    """Tests for Workflow model."""

    def test_minimal_workflow(self):
        wf = Workflow(name="test")
        assert wf.name == "test"
        assert wf.activities == []

    def test_workflow_with_activities(self):
        activity1 = Activity(key="step1", activity_name="echo")
        activity2 = Activity(key="step2", activity_name="echo")
        wf = Workflow(
            name="test",
            activities=[activity1, activity2],
        )
        assert wf.name == "test"
        assert len(wf.activities) == 2
        assert wf.activities[0].key == "step1"
        assert wf.activities[1].key == "step2"


class TestWorkflowSerialization:
    """Tests for Workflow serialization."""

    def test_minimal_workflow_to_dict(self):
        wf = Workflow(name="test")
        d = wf.to_dict()
        assert d["name"] == "test"
        assert d["activities"] == []

    def test_workflow_with_activities_to_dict(self):
        activity = Activity(key="step1", activity_name="echo")
        wf = Workflow(name="test", activities=[activity])
        d = wf.to_dict()
        assert len(d["activities"]) == 1
        assert d["activities"][0]["key"] == "step1"

    def test_workflow_to_yaml(self):
        activity = Activity(key="step1", activity_name="echo")
        wf = Workflow(name="test", activities=[activity])
        yaml_str = wf.to_yaml()
        assert "name: test" in yaml_str
        assert "step1" in yaml_str

    def test_workflow_to_json(self):
        wf = Workflow(name="test")
        json_dict = wf.to_json()
        assert json_dict["name"] == "test"


class TestScriptActivity:
    """Tests for ScriptActivity decorator functionality."""

    def test_from_function_basic(self):
        """Test basic function decoration."""

        @ScriptActivity.from_function()
        async def my_task():
            return {"result": "done"}

        assert isinstance(my_task, Activity)
        assert my_task.key == "my_task"
        assert my_task.worker == "py-std"
        assert my_task.activity_name == "script"
        assert "script" in my_task.parameters

    def test_from_function_with_params(self):
        """Test function decoration with parameters."""

        @ScriptActivity.from_function(
            key="custom_key", worker="py-data", inputs={"url": "https://api.example.com"}
        )
        async def fetch_data(url):
            import httpx

            async with httpx.AsyncClient() as client:
                response = await client.get(url)
                return {"data": response.json()}

        assert isinstance(fetch_data, Activity)
        assert fetch_data.key == "custom_key"
        assert fetch_data.worker == "py-data"
        assert fetch_data.parameters["inputs"]["url"] == "https://api.example.com"

    def test_from_function_with_depends_on(self):
        """Test function decoration with dependencies."""

        @ScriptActivity.from_function(depends_on=["task1", "task2"])
        async def dependent_task():
            return {"status": "ok"}

        assert isinstance(dependent_task, Activity)
        assert dependent_task.depends_on == ["task1", "task2"]

    def test_from_function_explicit_key(self):
        """Test that explicit key parameter is used when provided."""

        @ScriptActivity.from_function(key="explicit_name")
        async def my_function():
            return {"result": "ok"}

        assert my_function.key == "explicit_name"


class TestScriptHelper:
    """Tests for script() helper function."""

    def test_script_async_function(self):
        """Test script generation for async function."""

        async def process_data(records):
            import pandas as pd

            df = pd.DataFrame(records)
            return {"count": len(df)}

        result = script(process_data)
        assert "async def process_data" in result
        assert "records = INPUT.get('records')" in result
        assert "OUTPUT = await process_data(records)" in result

    def test_script_sync_function(self):
        """Test script generation for sync function."""

        def calculate(x, y):
            return {"sum": x + y}

        result = script(calculate)
        assert "def calculate" in result
        assert "x = INPUT.get('x')" in result
        assert "y = INPUT.get('y')" in result
        assert "OUTPUT = calculate(x, y)" in result

    def test_script_no_params(self):
        """Test script generation for function with no parameters."""

        async def get_timestamp():
            from datetime import datetime

            return {"timestamp": datetime.now().isoformat()}

        result = script(get_timestamp)
        assert "async def get_timestamp" in result
        assert "OUTPUT = await get_timestamp()" in result
        assert "INPUT.get" not in result
