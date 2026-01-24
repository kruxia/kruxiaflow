"""Tests for kruxiaflow.expressions module."""

import pytest

from kruxiaflow.expressions import (
    EnvRef,
    Input,
    OutputComparison,
    OutputRef,
    SecretRef,
)
from kruxiaflow.models import Activity


class TestInput:
    """Tests for Input expression class."""

    def test_basic_input(self):
        inp = Input("text")
        assert inp.name == "text"
        assert inp.required is True
        assert inp.default is None

    def test_input_with_type(self):
        inp = Input("count", type=int)
        assert inp._type is int

    def test_input_optional(self):
        inp = Input("text", required=False)
        assert inp.required is False

    def test_input_with_default(self):
        inp = Input("count", type=int, default=10)
        assert inp.default == 10

    def test_input_with_description(self):
        inp = Input("text", description="User input text")
        assert inp._description == "User input text"

    def test_input_str(self):
        inp = Input("text")
        assert str(inp) == "{{INPUT.text}}"

    def test_input_repr(self):
        inp = Input("text")
        assert repr(inp) == "Input('text')"

    def test_input_format(self):
        inp = Input("text")
        result = f"Value: {inp}"
        assert result == "Value: {{INPUT.text}}"

    def test_input_to_schema_string(self):
        inp = Input("text", type=str, required=True)
        schema = inp.to_schema()
        assert schema["type"] == "string"
        assert schema["required"] is True

    def test_input_to_schema_integer(self):
        inp = Input("count", type=int)
        schema = inp.to_schema()
        assert schema["type"] == "integer"

    def test_input_to_schema_number(self):
        inp = Input("rate", type=float)
        schema = inp.to_schema()
        assert schema["type"] == "number"

    def test_input_to_schema_boolean(self):
        inp = Input("enabled", type=bool)
        schema = inp.to_schema()
        assert schema["type"] == "boolean"

    def test_input_to_schema_array(self):
        inp = Input("items", type=list)
        schema = inp.to_schema()
        assert schema["type"] == "array"

    def test_input_to_schema_object(self):
        inp = Input("config", type=dict)
        schema = inp.to_schema()
        assert schema["type"] == "object"

    def test_input_to_schema_with_default(self):
        inp = Input("count", type=int, default=10)
        schema = inp.to_schema()
        assert schema["default"] == 10

    def test_input_to_schema_with_description(self):
        inp = Input("text", description="User input")
        schema = inp.to_schema()
        assert schema["description"] == "User input"

    def test_input_to_schema_no_type(self):
        inp = Input("text")
        schema = inp.to_schema()
        assert "type" not in schema


class TestOutputRef:
    """Tests for OutputRef expression class."""

    def test_output_ref_creation(self):
        activity = Activity(key="analyze")
        ref = OutputRef(activity, "sentiment")
        assert ref.activity_key == "analyze"
        assert ref.output_key == "sentiment"

    def test_output_ref_str(self):
        activity = Activity(key="analyze")
        ref = OutputRef(activity, "sentiment")
        assert str(ref) == "{{analyze.sentiment}}"

    def test_output_ref_repr(self):
        activity = Activity(key="analyze")
        ref = OutputRef(activity, "sentiment")
        assert repr(ref) == "OutputRef('analyze', 'sentiment')"

    def test_output_ref_format(self):
        activity = Activity(key="analyze")
        ref = OutputRef(activity, "sentiment")
        result = f"Result: {ref}"
        assert result == "Result: {{analyze.sentiment}}"

    def test_output_ref_nested_path(self):
        activity = Activity(key="fetch")
        ref = OutputRef(activity, "response.body.data")
        assert str(ref) == "{{fetch.response.body.data}}"

    def test_output_ref_equality_comparison(self):
        activity = Activity(key="analyze")
        ref = activity["status"]
        comparison = ref == "success"
        assert isinstance(comparison, OutputComparison)
        assert "analyze.status == 'success'" in str(comparison)

    def test_output_ref_inequality_comparison(self):
        activity = Activity(key="analyze")
        ref = activity["status"]
        comparison = ref != "failed"
        assert isinstance(comparison, OutputComparison)
        assert "analyze.status != 'failed'" in str(comparison)

    def test_output_ref_greater_than(self):
        activity = Activity(key="analyze")
        ref = activity["confidence"]
        comparison = ref > 0.8
        assert isinstance(comparison, OutputComparison)
        assert "analyze.confidence > 0.8" in str(comparison)

    def test_output_ref_less_than(self):
        activity = Activity(key="analyze")
        ref = activity["confidence"]
        comparison = ref < 0.5
        assert isinstance(comparison, OutputComparison)
        assert "analyze.confidence < 0.5" in str(comparison)

    def test_output_ref_greater_equal(self):
        activity = Activity(key="analyze")
        ref = activity["confidence"]
        comparison = ref >= 0.8
        assert isinstance(comparison, OutputComparison)
        assert "analyze.confidence >= 0.8" in str(comparison)

    def test_output_ref_less_equal(self):
        activity = Activity(key="analyze")
        ref = activity["confidence"]
        comparison = ref <= 0.5
        assert isinstance(comparison, OutputComparison)
        assert "analyze.confidence <= 0.5" in str(comparison)

    def test_output_ref_comparison_with_string(self):
        activity = Activity(key="analyze")
        ref = activity["sentiment"]
        comparison = ref == "positive"
        assert "'positive'" in str(comparison)

    def test_output_ref_comparison_with_boolean_true(self):
        activity = Activity(key="validate")
        ref = activity["valid"]
        comparison = ref == True  # noqa: E712
        assert "true" in str(comparison)

    def test_output_ref_comparison_with_boolean_false(self):
        activity = Activity(key="validate")
        ref = activity["valid"]
        comparison = ref == False  # noqa: E712
        assert "false" in str(comparison)

    def test_output_ref_comparison_with_none(self):
        activity = Activity(key="fetch")
        ref = activity["data"]
        comparison = ref == None  # noqa: E711
        assert "null" in str(comparison)


class TestOutputComparison:
    """Tests for OutputComparison expression class."""

    def test_comparison_str(self):
        comparison = OutputComparison("x > 5")
        assert str(comparison) == "{{ x > 5 }}"

    def test_comparison_repr(self):
        comparison = OutputComparison("x > 5")
        assert repr(comparison) == "OutputComparison('x > 5')"

    def test_comparison_expression_property(self):
        comparison = OutputComparison("x > 5")
        assert comparison.expression == "x > 5"

    def test_comparison_and(self):
        c1 = OutputComparison("x > 5")
        c2 = OutputComparison("y < 10")
        combined = c1 & c2
        assert isinstance(combined, OutputComparison)
        assert "(x > 5) && (y < 10)" in combined.expression

    def test_comparison_or(self):
        c1 = OutputComparison("x > 5")
        c2 = OutputComparison("y < 10")
        combined = c1 | c2
        assert isinstance(combined, OutputComparison)
        assert "(x > 5) || (y < 10)" in combined.expression

    def test_comparison_not(self):
        c = OutputComparison("x > 5")
        negated = ~c
        assert isinstance(negated, OutputComparison)
        assert "!(x > 5)" in negated.expression

    def test_complex_combined_condition(self):
        activity = Activity(key="analyze")
        c1 = activity["confidence"] > 0.8
        c2 = activity["sentiment"] == "positive"
        combined = c1 & c2
        assert "confidence > 0.8" in combined.expression
        assert "sentiment == 'positive'" in combined.expression
        assert "&&" in combined.expression


class TestSecretRef:
    """Tests for SecretRef expression class."""

    def test_secret_ref_creation(self):
        secret = SecretRef("api_key")
        assert secret.name == "api_key"

    def test_secret_ref_str(self):
        secret = SecretRef("api_key")
        assert str(secret) == "{{SECRET.api_key}}"

    def test_secret_ref_repr(self):
        secret = SecretRef("api_key")
        assert repr(secret) == "SecretRef('api_key')"

    def test_secret_ref_format(self):
        secret = SecretRef("api_key")
        result = f"Key: {secret}"
        assert result == "Key: {{SECRET.api_key}}"


class TestEnvRef:
    """Tests for EnvRef expression class."""

    def test_env_ref_creation(self):
        env = EnvRef("DATABASE_URL")
        assert env.name == "DATABASE_URL"

    def test_env_ref_str(self):
        env = EnvRef("DATABASE_URL")
        assert str(env) == "${DATABASE_URL}"

    def test_env_ref_repr(self):
        env = EnvRef("DATABASE_URL")
        assert repr(env) == "EnvRef('DATABASE_URL')"

    def test_env_ref_format(self):
        env = EnvRef("DATABASE_URL")
        result = f"URL: {env}"
        assert result == "URL: ${DATABASE_URL}"


class TestExpressionInParameters:
    """Tests for using expressions in activity parameters."""

    def test_input_in_params(self):
        text_input = Input("text")
        activity = Activity(key="analyze").with_params(prompt=text_input)
        # Should be serialized to string
        assert activity.parameters["prompt"] == "{{INPUT.text}}"

    def test_output_ref_in_params(self):
        step1 = Activity(key="step1")
        step2 = Activity(key="step2").with_params(data=step1["result"])
        assert step2.parameters["data"] == "{{step1.result}}"

    def test_secret_ref_in_params(self):
        secret = SecretRef("api_key")
        activity = Activity(key="fetch").with_params(
            headers={"Authorization": f"Bearer {secret}"}
        )
        assert "{{SECRET.api_key}}" in activity.parameters["headers"]["Authorization"]

    def test_env_ref_in_params(self):
        env = EnvRef("DATABASE_URL")
        activity = Activity(key="query").with_params(db_url=str(env))
        assert activity.parameters["db_url"] == "${DATABASE_URL}"

    def test_fstring_with_input(self):
        text_input = Input("text")
        activity = Activity(key="analyze").with_params(
            prompt=f"Analyze this: {text_input}"
        )
        assert activity.parameters["prompt"] == "Analyze this: {{INPUT.text}}"

    def test_mixed_expressions_in_list(self):
        inp = Input("query")
        step1 = Activity(key="step1")
        activity = Activity(key="step2").with_params(
            items=[inp, step1["result"], "static"]
        )
        assert activity.parameters["items"][0] == "{{INPUT.query}}"
        assert activity.parameters["items"][1] == "{{step1.result}}"
        assert activity.parameters["items"][2] == "static"

    def test_expressions_in_nested_dict(self):
        inp = Input("text")
        activity = Activity(key="process").with_params(
            config={
                "input": inp,
                "nested": {
                    "value": inp,
                },
            }
        )
        assert activity.parameters["config"]["input"] == "{{INPUT.text}}"
        assert activity.parameters["config"]["nested"]["value"] == "{{INPUT.text}}"
