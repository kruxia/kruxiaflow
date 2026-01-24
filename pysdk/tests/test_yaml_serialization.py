"""Tests for YAML serialization of workflows."""

import pytest
import yaml

from kruxiaflow.expressions import Input, SecretRef
from kruxiaflow.models import Activity, Dependency, Workflow


class TestSimpleWorkflowYAML:
    """Tests for simple workflow YAML serialization."""

    def test_minimal_workflow_yaml(self):
        wf = Workflow(name="test")
        yaml_str = wf.to_yaml()
        data = yaml.safe_load(yaml_str)

        assert data["name"] == "test"
        assert data["version"] == "1.0.0"

    def test_single_activity_workflow_yaml(self):
        activity = (
            Activity(key="fetch")
            .with_worker("builtin", "http_request")
            .with_params(url="https://example.com", method="GET")
        )
        wf = Workflow(name="simple").with_activities(activity)
        yaml_str = wf.to_yaml()
        data = yaml.safe_load(yaml_str)

        assert data["name"] == "simple"
        assert len(data["activities"]) == 1
        assert data["activities"][0]["key"] == "fetch"
        assert data["activities"][0]["worker"] == "builtin"
        assert data["activities"][0]["activity_name"] == "http_request"
        assert data["activities"][0]["parameters"]["url"] == "https://example.com"

    def test_workflow_with_inputs_yaml(self):
        text_input = Input("text", type=str, required=True, description="Input text")
        count_input = Input("count", type=int, default=10)

        wf = Workflow(name="with_inputs").with_inputs(text_input, count_input)
        yaml_str = wf.to_yaml()
        data = yaml.safe_load(yaml_str)

        assert "inputs" in data
        assert data["inputs"]["text"]["type"] == "string"
        assert data["inputs"]["text"]["required"] is True
        assert data["inputs"]["text"]["description"] == "Input text"
        assert data["inputs"]["count"]["type"] == "integer"
        assert data["inputs"]["count"]["default"] == 10


class TestSequentialWorkflowYAML:
    """Tests for sequential workflow YAML serialization."""

    def test_two_step_sequential_workflow(self):
        step1 = (
            Activity(key="step1")
            .with_worker("builtin", "http_request")
            .with_params(url="https://api.example.com")
        )
        step2 = (
            Activity(key="step2")
            .with_worker("builtin", "http_request")
            .with_params(url="https://api.example.com/next", data=step1["response"])
            .with_dependencies(step1)
        )
        wf = Workflow(name="sequential").with_activities(step1, step2)
        yaml_str = wf.to_yaml()
        data = yaml.safe_load(yaml_str)

        assert len(data["activities"]) == 2
        assert data["activities"][0]["key"] == "step1"
        assert "depends_on" not in data["activities"][0]
        assert data["activities"][1]["key"] == "step2"
        assert data["activities"][1]["depends_on"] == ["step1"]
        assert "{{step1.response}}" in data["activities"][1]["parameters"]["data"]


class TestParallelWorkflowYAML:
    """Tests for parallel workflow YAML serialization."""

    def test_fan_out_workflow(self):
        # Three parallel activities with no dependencies
        activities = [
            Activity(key=f"fetch_{i}")
            .with_worker("builtin", "http_request")
            .with_params(url=f"https://api{i}.example.com")
            for i in range(3)
        ]
        wf = Workflow(name="fan_out").with_activities(*activities)
        yaml_str = wf.to_yaml()
        data = yaml.safe_load(yaml_str)

        assert len(data["activities"]) == 3
        for activity in data["activities"]:
            assert "depends_on" not in activity

    def test_fan_in_workflow(self):
        # Three parallel activities that fan into one
        fetches = [
            Activity(key=f"fetch_{i}")
            .with_worker("builtin", "http_request")
            .with_params(url=f"https://api{i}.example.com")
            for i in range(3)
        ]
        aggregate = (
            Activity(key="aggregate")
            .with_worker("builtin", "http_request")
            .with_params(results=[f["response"] for f in fetches])
            .with_dependencies(*fetches)
        )
        wf = Workflow(name="fan_in").with_activities(*fetches, aggregate)
        yaml_str = wf.to_yaml()
        data = yaml.safe_load(yaml_str)

        assert len(data["activities"]) == 4
        aggregate_data = next(a for a in data["activities"] if a["key"] == "aggregate")
        assert set(aggregate_data["depends_on"]) == {"fetch_0", "fetch_1", "fetch_2"}


class TestConditionalWorkflowYAML:
    """Tests for conditional workflow YAML serialization."""

    def test_conditional_dependency(self):
        check = (
            Activity(key="check")
            .with_worker("builtin", "http_request")
            .with_params(url="https://api.example.com/validate")
        )
        success_path = (
            Activity(key="success")
            .with_worker("builtin", "http_request")
            .with_params(url="https://api.example.com/success")
            .with_dependencies(
                Dependency.on(check, check["valid"] == True)  # noqa: E712
            )
        )
        failure_path = (
            Activity(key="failure")
            .with_worker("builtin", "http_request")
            .with_params(url="https://api.example.com/failure")
            .with_dependencies(
                Dependency.on(check, check["valid"] == False)  # noqa: E712
            )
        )
        wf = Workflow(name="conditional").with_activities(
            check, success_path, failure_path
        )
        yaml_str = wf.to_yaml()
        data = yaml.safe_load(yaml_str)

        success_data = next(a for a in data["activities"] if a["key"] == "success")
        failure_data = next(a for a in data["activities"] if a["key"] == "failure")

        # Conditional dependencies should have conditions
        assert len(success_data["depends_on"]) == 1
        assert success_data["depends_on"][0]["activity_key"] == "check"
        assert len(success_data["depends_on"][0]["conditions"]) == 1
        assert "true" in success_data["depends_on"][0]["conditions"][0]

        assert len(failure_data["depends_on"]) == 1
        assert failure_data["depends_on"][0]["activity_key"] == "check"
        assert "false" in failure_data["depends_on"][0]["conditions"][0]


class TestActivitySettingsYAML:
    """Tests for activity settings YAML serialization."""

    def test_activity_with_timeout(self):
        activity = (
            Activity(key="slow")
            .with_worker("builtin", "http_request")
            .with_params(url="https://slow.api.com")
            .with_timeout(300)
        )
        wf = Workflow(name="timeout").with_activities(activity)
        yaml_str = wf.to_yaml()
        data = yaml.safe_load(yaml_str)

        assert data["activities"][0]["settings"]["timeout_seconds"] == 300

    def test_activity_with_retry(self):
        activity = (
            Activity(key="retry")
            .with_worker("builtin", "http_request")
            .with_params(url="https://flaky.api.com")
            .with_retry(max_attempts=5, strategy="exponential")
        )
        wf = Workflow(name="retry").with_activities(activity)
        yaml_str = wf.to_yaml()
        data = yaml.safe_load(yaml_str)

        retry_settings = data["activities"][0]["settings"]["retry"]
        assert retry_settings["max_attempts"] == 5
        assert retry_settings["strategy"] == "exponential"

    def test_activity_with_cache(self):
        activity = (
            Activity(key="cached")
            .with_worker("builtin", "http_request")
            .with_params(url="https://api.example.com")
            .with_cache(ttl=3600, key="custom_key")
        )
        wf = Workflow(name="cache").with_activities(activity)
        yaml_str = wf.to_yaml()
        data = yaml.safe_load(yaml_str)

        cache_settings = data["activities"][0]["settings"]["cache"]
        assert cache_settings["enabled"] is True
        assert cache_settings["ttl"] == 3600
        assert cache_settings["key"] == "custom_key"

    def test_activity_with_budget(self):
        activity = (
            Activity(key="llm")
            .with_worker("builtin", "llm_prompt")
            .with_params(prompt="Hello")
            .with_budget(limit_usd=1.0, action="abort")
        )
        wf = Workflow(name="budget").with_activities(activity)
        yaml_str = wf.to_yaml()
        data = yaml.safe_load(yaml_str)

        budget_settings = data["activities"][0]["settings"]["budget"]
        assert budget_settings["limit_usd"] == 1.0
        assert budget_settings["action"] == "abort"

    def test_activity_with_all_settings(self):
        activity = (
            Activity(key="complete")
            .with_worker("builtin", "http_request")
            .with_params(url="https://api.example.com")
            .with_timeout(300)
            .with_retry(max_attempts=3)
            .with_cache(ttl=3600)
            .with_budget(limit_usd=0.5)
            .with_delay("5s")
            .with_streaming(True)
        )
        wf = Workflow(name="all_settings").with_activities(activity)
        yaml_str = wf.to_yaml()
        data = yaml.safe_load(yaml_str)

        settings = data["activities"][0]["settings"]
        assert settings["timeout_seconds"] == 300
        assert settings["retry"]["max_attempts"] == 3
        assert settings["cache"]["ttl"] == 3600
        assert settings["budget"]["limit_usd"] == 0.5
        assert settings["delay"] == "5s"
        assert settings["streaming"] is True


class TestCompleteWorkflowYAML:
    """Tests for complete workflow YAML serialization."""

    def test_weather_report_workflow(self):
        """Test a realistic weather report workflow."""
        webhook_url = Input("webhook_url", type=str, required=True)

        fetch_weather = (
            Activity(key="fetch_weather")
            .with_worker("builtin", "http_request")
            .with_params(
                method="GET",
                url="https://api.weather.gov/gridpoints/LOT/76,73/forecast",
            )
        )

        send_notification = (
            Activity(key="send_notification")
            .with_worker("builtin", "http_request")
            .with_params(
                method="POST",
                url=webhook_url,
                headers={"Content-Type": "application/json"},
                body={
                    "temperature": fetch_weather[
                        "response.json.properties.periods[0].temperature"
                    ],
                    "workflow_id": "{{WORKFLOW.id}}",
                },
            )
            .with_dependencies(fetch_weather)
        )

        workflow = (
            Workflow(name="weather_report")
            .with_inputs(webhook_url)
            .with_activities(fetch_weather, send_notification)
        )

        yaml_str = workflow.to_yaml()
        data = yaml.safe_load(yaml_str)

        assert data["name"] == "weather_report"
        assert "inputs" in data
        assert data["inputs"]["webhook_url"]["type"] == "string"
        assert len(data["activities"]) == 2
        assert data["activities"][1]["depends_on"] == ["fetch_weather"]

    def test_user_validation_workflow(self):
        """Test a realistic user validation workflow with branching."""
        email = Input("email", type=str, required=True)
        db_url = SecretRef("db_url")

        check_email = (
            Activity(key="check_email")
            .with_worker("builtin", "http_request")
            .with_params(method="GET", url="https://httpbin.org/json")
        )

        store_valid = (
            Activity(key="store_valid")
            .with_worker("builtin", "postgres_query")
            .with_params(db_url=db_url, query="INSERT INTO valid_users...")
            .with_dependencies(
                Dependency.on(check_email, check_email["response.success"] == True)  # noqa: E712
            )
        )

        store_invalid = (
            Activity(key="store_invalid")
            .with_worker("builtin", "postgres_query")
            .with_params(db_url=db_url, query="INSERT INTO invalid_users...")
            .with_dependencies(
                Dependency.on(check_email, check_email["response.success"] != True)  # noqa: E712
            )
        )

        workflow = (
            Workflow(name="validate_user")
            .with_inputs(email)
            .with_activities(check_email, store_valid, store_invalid)
        )

        yaml_str = workflow.to_yaml()
        data = yaml.safe_load(yaml_str)

        assert data["name"] == "validate_user"
        assert len(data["activities"]) == 3

        # Check conditional dependencies
        valid_activity = next(
            a for a in data["activities"] if a["key"] == "store_valid"
        )
        assert len(valid_activity["depends_on"]) == 1
        assert valid_activity["depends_on"][0]["activity_key"] == "check_email"
        assert len(valid_activity["depends_on"][0]["conditions"]) == 1


class TestYAMLRoundTrip:
    """Tests to ensure YAML output is valid and parseable."""

    def test_yaml_is_valid(self):
        """Ensure generated YAML is valid YAML syntax."""
        activity = Activity(key="test").with_worker("builtin", "echo")
        wf = Workflow(name="test").with_activities(activity)
        yaml_str = wf.to_yaml()

        # Should not raise
        data = yaml.safe_load(yaml_str)
        assert data is not None

    def test_yaml_no_python_objects(self):
        """Ensure no Python-specific objects leak into YAML."""
        text_input = Input("text", type=str)
        activity = (
            Activity(key="test")
            .with_worker("builtin", "echo")
            .with_params(input=text_input)
        )
        wf = Workflow(name="test").with_inputs(text_input).with_activities(activity)
        yaml_str = wf.to_yaml()

        # Should not contain Python object representations
        assert "!!python" not in yaml_str
        assert "<" not in yaml_str  # No <object at 0x...>

    def test_special_characters_in_strings(self):
        """Ensure special characters are properly escaped."""
        activity = (
            Activity(key="test")
            .with_worker("builtin", "echo")
            .with_params(
                text="Hello 'world'",
                query="SELECT * FROM users WHERE name = 'John's'",
            )
        )
        wf = Workflow(name="test").with_activities(activity)
        yaml_str = wf.to_yaml()
        data = yaml.safe_load(yaml_str)

        # Should be parseable and preserve values
        assert data["activities"][0]["parameters"]["text"] == "Hello 'world'"
