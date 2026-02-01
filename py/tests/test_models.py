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
    InputSchema,
    RetrySettings,
    Workflow,
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
        settings = BudgetSettings(limit_usd=10.0)
        assert settings.limit_usd == 10.0
        assert settings.action == BudgetAction.ABORT

    def test_custom_action(self):
        settings = BudgetSettings(limit_usd=5.0, action=BudgetAction.CONTINUE)
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
            budget=BudgetSettings(limit_usd=1.0),
            delay="5s",
            streaming=True,
        )
        assert settings.timeout_seconds == 300
        assert settings.retry.max_attempts == 5
        assert settings.cache.ttl == 3600
        assert settings.budget.limit_usd == 1.0
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

    def test_activity_allows_empty_activity_name_for_fluent_api(self):
        """Activity can be constructed without activity_name for fluent API."""
        activity = Activity(key="test")
        assert activity.activity_name == ""
        # with_worker sets activity_name
        activity.with_worker("std", "echo")
        assert activity.activity_name == "echo"

    def test_to_dict_requires_activity_name(self):
        """to_dict() raises error if activity_name is empty."""
        activity = Activity(key="test")
        with pytest.raises(ValueError, match="has no activity_name"):
            activity.to_dict()

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


class TestActivityFluentMethods:
    """Tests for Activity fluent builder methods."""

    def test_with_worker(self):
        activity = Activity(key="test", activity_name="placeholder").with_worker(
            "std", "http_request"
        )
        assert activity.worker == "std"
        assert activity.activity_name == "http_request"

    def test_with_worker_returns_self(self):
        activity = Activity(key="test", activity_name="placeholder")
        result = activity.with_worker("std", "http_request")
        assert result is activity

    def test_with_params(self):
        activity = Activity(key="test", activity_name="echo").with_params(
            url="https://example.com", method="GET"
        )
        assert activity.parameters["url"] == "https://example.com"
        assert activity.parameters["method"] == "GET"

    def test_with_params_merges(self):
        activity = (
            Activity(key="test", activity_name="echo")
            .with_params(url="https://example.com")
            .with_params(method="POST")
        )
        assert activity.parameters["url"] == "https://example.com"
        assert activity.parameters["method"] == "POST"

    def test_with_timeout(self):
        activity = Activity(key="test", activity_name="echo").with_timeout(300)
        assert activity.settings.timeout_seconds == 300

    def test_with_retry(self):
        activity = Activity(key="test", activity_name="echo").with_retry(
            max_attempts=5, strategy=BackoffStrategy.FIXED
        )
        assert activity.settings.retry.max_attempts == 5
        assert activity.settings.retry.strategy == BackoffStrategy.FIXED

    def test_with_retry_defaults(self):
        activity = Activity(key="test", activity_name="echo").with_retry()
        assert activity.settings.retry.max_attempts == 3
        assert activity.settings.retry.strategy == BackoffStrategy.EXPONENTIAL

    def test_with_cache(self):
        activity = Activity(key="test", activity_name="echo").with_cache(
            ttl=3600, key="my_key"
        )
        assert activity.settings.cache.ttl == 3600
        assert activity.settings.cache.key == "my_key"

    def test_with_budget(self):
        activity = Activity(key="test", activity_name="echo").with_budget(
            limit_usd=10.0
        )
        assert activity.settings.budget.limit_usd == 10.0
        assert activity.settings.budget.action == BudgetAction.ABORT

    def test_with_delay(self):
        activity = Activity(key="test", activity_name="echo").with_delay("5s")
        assert activity.settings.delay == "5s"

    def test_with_streaming(self):
        activity = Activity(key="test", activity_name="echo").with_streaming(True)
        assert activity.settings.streaming is True

    def test_with_dependencies_from_activities(self):
        step1 = Activity(key="step1", activity_name="echo")
        step2 = Activity(key="step2", activity_name="echo")
        step3 = Activity(key="step3", activity_name="echo").with_dependencies(
            step1, step2
        )
        assert "step1" in step3.depends_on
        assert "step2" in step3.depends_on

    def test_with_dependencies_from_strings(self):
        activity = Activity(key="test", activity_name="echo").with_dependencies(
            "step1", "step2"
        )
        assert "step1" in activity.depends_on
        assert "step2" in activity.depends_on

    def test_with_dependencies_from_dependency_objects(self):
        dep = Dependency(activity_key="step1", conditions=["{{ step1.ok }} == true"])
        activity = Activity(key="test", activity_name="echo").with_dependencies(dep)
        assert len(activity.depends_on) == 1
        assert isinstance(activity.depends_on[0], Dependency)

    def test_with_dependencies_mixed(self):
        step1 = Activity(key="step1", activity_name="echo")
        dep = Dependency(activity_key="step2", conditions=["condition"])
        activity = Activity(key="test", activity_name="echo").with_dependencies(
            step1, dep, "step3"
        )
        assert len(activity.depends_on) == 3

    def test_chaining_all_methods(self):
        activity = (
            Activity(key="complete", activity_name="placeholder")
            .with_worker("std", "http_request")
            .with_params(url="https://example.com")
            .with_timeout(300)
            .with_retry(max_attempts=3)
            .with_cache(ttl=3600)
            .with_budget(limit_usd=1.0)
            .with_delay("1s")
            .with_streaming(True)
        )
        assert activity.key == "complete"
        assert activity.worker == "std"
        assert activity.activity_name == "http_request"
        assert activity.parameters["url"] == "https://example.com"
        assert activity.settings.timeout_seconds == 300
        assert activity.settings.retry.max_attempts == 3
        assert activity.settings.cache.ttl == 3600
        assert activity.settings.budget.limit_usd == 1.0
        assert activity.settings.delay == "1s"
        assert activity.settings.streaming is True


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
        activity = Activity(key="test", activity_name="echo").with_params(
            url="https://example.com"
        )
        d = activity.to_dict()
        assert d["parameters"]["url"] == "https://example.com"

    def test_activity_with_settings_to_dict(self):
        activity = (
            Activity(key="test", activity_name="echo")
            .with_timeout(300)
            .with_retry(max_attempts=5, strategy=BackoffStrategy.FIXED)
        )
        d = activity.to_dict()
        assert d["settings"]["timeout_seconds"] == 300
        assert d["settings"]["retry"]["max_attempts"] == 5
        assert d["settings"]["retry"]["strategy"] == "fixed"

    def test_activity_with_simple_dependencies_to_dict(self):
        activity = Activity(key="test", activity_name="echo").with_dependencies(
            "step1", "step2"
        )
        d = activity.to_dict()
        assert d["depends_on"] == ["step1", "step2"]

    def test_activity_with_conditional_dependency_to_dict(self):
        dep = Dependency(activity_key="step1", conditions=["{{ step1.ok }} == true"])
        activity = Activity(key="test", activity_name="echo").with_dependencies(dep)
        d = activity.to_dict()
        assert len(d["depends_on"]) == 1
        assert d["depends_on"][0]["activity_key"] == "step1"
        assert d["depends_on"][0]["conditions"] == ["{{ step1.ok }} == true"]

    def test_activity_with_dependency_object_no_conditions_to_dict(self):
        """Dependency object without conditions should serialize to string."""
        dep = Dependency(activity_key="step1")
        activity = Activity(key="test", activity_name="echo").with_dependencies(dep)
        d = activity.to_dict()
        assert d["depends_on"] == ["step1"]

    def test_activity_with_full_retry_settings_to_dict(self):
        """Test retry settings with all optional fields."""
        activity = Activity(key="test", activity_name="echo").with_retry(
            max_attempts=5,
            strategy=BackoffStrategy.EXPONENTIAL,
            base_seconds=1.0,
            factor=2.0,
            max_seconds=60.0,
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
        activity = Activity(key="test", activity_name="echo")
        activity.settings.scheduled_for = "2024-01-01T00:00:00Z"
        d = activity.to_dict()
        assert d["settings"]["scheduled_for"] == "2024-01-01T00:00:00Z"

    def test_activity_with_iteration_scoped_to_dict(self):
        """Test iteration_scoped setting serialization."""
        activity = Activity(key="test", activity_name="echo")
        activity.settings.iteration_scoped = True
        d = activity.to_dict()
        assert d["settings"]["iteration_scoped"] is True


class TestWorkflow:
    """Tests for Workflow model."""

    def test_minimal_workflow(self):
        wf = Workflow(name="test")
        assert wf.name == "test"
        assert wf.version == "1.0.0"
        assert wf.namespace == "default"
        assert wf.activities == []
        assert wf.inputs == {}

    def test_full_workflow(self):
        wf = Workflow(
            name="test",
            version="2.0.0",
            namespace="production",
            description="A test workflow",
        )
        assert wf.name == "test"
        assert wf.version == "2.0.0"
        assert wf.namespace == "production"
        assert wf.description == "A test workflow"


class TestWorkflowFluentMethods:
    """Tests for Workflow fluent builder methods."""

    def test_with_version(self):
        wf = Workflow(name="test").with_version("2.0.0")
        assert wf.version == "2.0.0"

    def test_with_namespace(self):
        wf = Workflow(name="test").with_namespace("production")
        assert wf.namespace == "production"

    def test_with_description(self):
        wf = Workflow(name="test").with_description("My workflow")
        assert wf.description == "My workflow"

    def test_with_inputs(self):
        from kruxiaflow.expressions import Input

        text_input = Input("text", type=str, required=True)
        count_input = Input("count", type=int, default=10)
        wf = Workflow(name="test").with_inputs(text_input, count_input)

        assert "text" in wf.inputs
        assert wf.inputs["text"].type == "string"
        assert wf.inputs["text"].required is True
        assert "count" in wf.inputs
        assert wf.inputs["count"].type == "integer"
        assert wf.inputs["count"].default == 10

    def test_with_activities(self):
        activity1 = Activity(key="step1", activity_name="echo")
        activity2 = Activity(key="step2", activity_name="echo")
        wf = Workflow(name="test").with_activities(activity1, activity2)
        assert len(wf.activities) == 2
        assert wf.activities[0].key == "step1"
        assert wf.activities[1].key == "step2"

    def test_with_activities_appends(self):
        activity1 = Activity(key="step1", activity_name="echo")
        activity2 = Activity(key="step2", activity_name="echo")
        activity3 = Activity(key="step3", activity_name="echo")
        wf = (
            Workflow(name="test")
            .with_activities(activity1, activity2)
            .with_activities(activity3)
        )
        assert len(wf.activities) == 3

    def test_chaining_all_methods(self):
        from kruxiaflow.expressions import Input

        text_input = Input("text", type=str)
        activity = Activity(key="process", activity_name="echo")

        wf = (
            Workflow(name="complete")
            .with_version("2.0.0")
            .with_namespace("production")
            .with_description("Complete workflow")
            .with_inputs(text_input)
            .with_activities(activity)
        )

        assert wf.name == "complete"
        assert wf.version == "2.0.0"
        assert wf.namespace == "production"
        assert wf.description == "Complete workflow"
        assert "text" in wf.inputs
        assert len(wf.activities) == 1


class TestWorkflowSerialization:
    """Tests for Workflow serialization."""

    def test_minimal_workflow_to_dict(self):
        wf = Workflow(name="test")
        d = wf.to_dict()
        assert d["name"] == "test"
        assert d["version"] == "1.0.0"
        assert "namespace" not in d  # default namespace omitted
        assert "inputs" not in d
        assert "activities" not in d

    def test_workflow_with_namespace_to_dict(self):
        wf = Workflow(name="test").with_namespace("production")
        d = wf.to_dict()
        assert d["namespace"] == "production"

    def test_workflow_with_inputs_to_dict(self):
        from kruxiaflow.expressions import Input

        text_input = Input("text", type=str, required=True, description="User text")
        wf = Workflow(name="test").with_inputs(text_input)
        d = wf.to_dict()
        assert "inputs" in d
        assert d["inputs"]["text"]["type"] == "string"
        assert d["inputs"]["text"]["required"] is True
        assert d["inputs"]["text"]["description"] == "User text"

    def test_workflow_with_activities_to_dict(self):
        activity = Activity(key="step1", activity_name="echo")
        wf = Workflow(name="test").with_activities(activity)
        d = wf.to_dict()
        assert len(d["activities"]) == 1
        assert d["activities"][0]["key"] == "step1"

    def test_workflow_with_description_to_dict(self):
        wf = Workflow(name="test").with_description("My workflow description")
        d = wf.to_dict()
        assert d["description"] == "My workflow description"

    def test_workflow_to_yaml(self):
        activity = Activity(key="step1", activity_name="echo")
        wf = Workflow(name="test").with_activities(activity)
        yaml_str = wf.to_yaml()
        assert "name: test" in yaml_str
        assert "version: '1.0.0'" in yaml_str or "version: 1.0.0" in yaml_str
        assert "step1" in yaml_str

    def test_workflow_to_json(self):
        wf = Workflow(name="test")
        json_dict = wf.to_json()
        assert json_dict["name"] == "test"

    def test_workflow_with_unknown_input_type(self):
        """Unknown types should default to 'string' in schema."""
        from kruxiaflow.expressions import Input

        # Using bytes as an unknown type (not in type_map)
        inp = Input("data", type=bytes)
        wf = Workflow(name="test").with_inputs(inp)
        d = wf.to_dict()
        # Unknown types fall back to "string"
        assert d["inputs"]["data"]["type"] == "string"

    def test_workflow_with_input_no_type(self):
        """Input without type should default to 'string' in schema."""
        from kruxiaflow.expressions import Input

        # No type specified (type=None internally)
        inp = Input("text")
        wf = Workflow(name="test").with_inputs(inp)
        d = wf.to_dict()
        # None type defaults to "string"
        assert d["inputs"]["text"]["type"] == "string"


class TestInputSchema:
    """Tests for InputSchema model."""

    def test_default_values(self):
        schema = InputSchema()
        assert schema.type == "string"
        assert schema.required is True
        assert schema.default is None
        assert schema.description is None

    def test_all_fields(self):
        schema = InputSchema(
            type="integer",
            required=False,
            default=42,
            description="Count value",
        )
        assert schema.type == "integer"
        assert schema.required is False
        assert schema.default == 42
        assert schema.description == "Count value"
