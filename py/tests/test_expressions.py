"""Tests for kruxiaflow.expressions module."""

from kruxiaflow.expressions import (
    And,
    Comparison,
    EnvRef,
    Eq,
    Expression,
    Ge,
    Gt,
    Input,
    IsNotNull,
    IsNull,
    Le,
    Literal,
    Lt,
    Ne,
    Not,
    Or,
    OutputRef,
    SecretRef,
    WorkflowRef,
    and_,
    contains,
    in_,
    is_not_null,
    is_null,
    not_,
    or_,
    workflow,
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
        activity = Activity(key="analyze", worker="std", activity_name="echo")
        ref = OutputRef(activity, "sentiment")
        assert ref.activity_key == "analyze"
        assert ref.output_key == "sentiment"

    def test_output_ref_str(self):
        activity = Activity(key="analyze", worker="std", activity_name="echo")
        ref = OutputRef(activity, "sentiment")
        assert str(ref) == "{{analyze.sentiment}}"

    def test_output_ref_repr(self):
        activity = Activity(key="analyze", worker="std", activity_name="echo")
        ref = OutputRef(activity, "sentiment")
        assert repr(ref) == "OutputRef('analyze', 'sentiment')"

    def test_output_ref_format(self):
        activity = Activity(key="analyze", worker="std", activity_name="echo")
        ref = OutputRef(activity, "sentiment")
        result = f"Result: {ref}"
        assert result == "Result: {{analyze.sentiment}}"

    def test_output_ref_nested_path(self):
        activity = Activity(key="fetch", worker="std", activity_name="echo")
        ref = OutputRef(activity, "response.body.data")
        assert str(ref) == "{{fetch.response.body.data}}"

    def test_output_ref_equality_comparison(self):
        activity = Activity(key="analyze", worker="std", activity_name="echo")
        ref = activity["status"]
        comparison = ref == "success"
        assert isinstance(comparison, Eq)
        assert "analyze.status == 'success'" in str(comparison)

    def test_output_ref_inequality_comparison(self):
        activity = Activity(key="analyze", worker="std", activity_name="echo")
        ref = activity["status"]
        comparison = ref != "failed"
        assert isinstance(comparison, Ne)
        assert "analyze.status != 'failed'" in str(comparison)

    def test_output_ref_greater_than(self):
        activity = Activity(key="analyze", worker="std", activity_name="echo")
        ref = activity["confidence"]
        comparison = ref > 0.8
        assert isinstance(comparison, Gt)
        assert "analyze.confidence > 0.8" in str(comparison)

    def test_output_ref_less_than(self):
        activity = Activity(key="analyze", worker="std", activity_name="echo")
        ref = activity["confidence"]
        comparison = ref < 0.5
        assert isinstance(comparison, Lt)
        assert "analyze.confidence < 0.5" in str(comparison)

    def test_output_ref_greater_equal(self):
        activity = Activity(key="analyze", worker="std", activity_name="echo")
        ref = activity["confidence"]
        comparison = ref >= 0.8
        assert isinstance(comparison, Ge)
        assert "analyze.confidence >= 0.8" in str(comparison)

    def test_output_ref_less_equal(self):
        activity = Activity(key="analyze", worker="std", activity_name="echo")
        ref = activity["confidence"]
        comparison = ref <= 0.5
        assert isinstance(comparison, Le)
        assert "analyze.confidence <= 0.5" in str(comparison)

    def test_output_ref_comparison_with_string(self):
        activity = Activity(key="analyze", worker="std", activity_name="echo")
        ref = activity["sentiment"]
        comparison = ref == "positive"
        assert "'positive'" in str(comparison)

    def test_output_ref_comparison_with_boolean_true(self):
        activity = Activity(key="validate", worker="std", activity_name="echo")
        ref = activity["valid"]
        comparison = ref == True  # noqa: E712
        assert "true" in str(comparison)

    def test_output_ref_comparison_with_boolean_false(self):
        activity = Activity(key="validate", worker="std", activity_name="echo")
        ref = activity["valid"]
        comparison = ref == False  # noqa: E712
        assert "false" in str(comparison)

    def test_output_ref_comparison_with_none(self):
        activity = Activity(key="fetch", worker="std", activity_name="echo")
        ref = activity["data"]
        comparison = ref == None  # noqa: E711
        assert "null" in str(comparison)


class TestComparison:
    """Tests for Comparison expression classes."""

    def test_eq_comparison_str(self):
        activity = Activity(key="test", worker="std", activity_name="echo")
        comparison = activity["x"] == 5
        assert str(comparison) == "{{test.x == 5}}"

    def test_gt_comparison_str(self):
        activity = Activity(key="test", worker="std", activity_name="echo")
        comparison = activity["x"] > 5
        assert str(comparison) == "{{test.x > 5}}"

    def test_comparison_repr(self):
        activity = Activity(key="test", worker="std", activity_name="echo")
        comparison = activity["x"] > 5
        assert "Gt(" in repr(comparison)

    def test_comparison_and(self):
        activity = Activity(key="test", worker="std", activity_name="echo")
        c1 = activity["x"] > 5
        c2 = activity["y"] < 10
        combined = c1 & c2
        assert isinstance(combined, And)
        assert "(test.x > 5) && (test.y < 10)" in str(combined)

    def test_comparison_or(self):
        activity = Activity(key="test", worker="std", activity_name="echo")
        c1 = activity["x"] > 5
        c2 = activity["y"] < 10
        combined = c1 | c2
        assert isinstance(combined, Or)
        assert "(test.x > 5) || (test.y < 10)" in str(combined)

    def test_comparison_not(self):
        activity = Activity(key="test", worker="std", activity_name="echo")
        c = activity["x"] > 5
        negated = ~c
        assert isinstance(negated, Not)
        assert "!(test.x > 5)" in str(negated)

    def test_complex_combined_condition(self):
        activity = Activity(key="analyze", worker="std", activity_name="echo")
        c1 = activity["confidence"] > 0.8
        c2 = activity["sentiment"] == "positive"
        combined = c1 & c2
        combined_str = str(combined)
        assert "confidence > 0.8" in combined_str
        assert "sentiment == 'positive'" in combined_str
        assert "&&" in combined_str


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
        activity = Activity(
            key="analyze",
            worker="std",
            activity_name="echo",
            parameters={"prompt": text_input},
        )
        # Should be serialized to string
        assert activity.parameters["prompt"] == "{{INPUT.text}}"

    def test_output_ref_in_params(self):
        step1 = Activity(key="step1", worker="std", activity_name="echo")
        step2 = Activity(
            key="step2",
            worker="std",
            activity_name="echo",
            parameters={"data": step1["result"]},
        )
        assert step2.parameters["data"] == "{{step1.result}}"

    def test_secret_ref_in_params(self):
        secret = SecretRef("api_key")
        activity = Activity(
            key="fetch",
            worker="std",
            activity_name="echo",
            parameters={"headers": {"Authorization": f"Bearer {secret}"}},
        )
        assert "{{SECRET.api_key}}" in activity.parameters["headers"]["Authorization"]

    def test_env_ref_in_params(self):
        env = EnvRef("DATABASE_URL")
        activity = Activity(
            key="query",
            worker="std",
            activity_name="echo",
            parameters={"db_url": str(env)},
        )
        assert activity.parameters["db_url"] == "${DATABASE_URL}"

    def test_fstring_with_input(self):
        text_input = Input("text")
        activity = Activity(
            key="analyze",
            worker="std",
            activity_name="echo",
            parameters={"prompt": f"Analyze this: {text_input}"},
        )
        assert activity.parameters["prompt"] == "Analyze this: {{INPUT.text}}"

    def test_mixed_expressions_in_list(self):
        inp = Input("query")
        step1 = Activity(key="step1", worker="std", activity_name="echo")
        activity = Activity(
            key="step2",
            worker="std",
            activity_name="echo",
            parameters={"items": [inp, step1["result"], "static"]},
        )
        assert activity.parameters["items"][0] == "{{INPUT.query}}"
        assert activity.parameters["items"][1] == "{{step1.result}}"
        assert activity.parameters["items"][2] == "static"

    def test_expressions_in_nested_dict(self):
        inp = Input("text")
        activity = Activity(
            key="process",
            worker="std",
            activity_name="echo",
            parameters={
                "config": {
                    "input": inp,
                    "nested": {
                        "value": inp,
                    },
                }
            },
        )
        assert activity.parameters["config"]["input"] == "{{INPUT.text}}"
        assert activity.parameters["config"]["nested"]["value"] == "{{INPUT.text}}"


class TestLiteral:
    """Tests for Literal expression class."""

    def test_literal_string(self):
        lit = Literal("hello")
        assert lit._to_template() == "'hello'"

    def test_literal_int(self):
        lit = Literal(42)
        assert lit._to_template() == "42"

    def test_literal_float(self):
        lit = Literal(3.14)
        assert lit._to_template() == "3.14"

    def test_literal_bool_true(self):
        lit = Literal(True)
        assert lit._to_template() == "true"

    def test_literal_bool_false(self):
        lit = Literal(False)
        assert lit._to_template() == "false"

    def test_literal_none(self):
        lit = Literal(None)
        assert lit._to_template() == "null"

    def test_literal_escapes_quotes(self):
        lit = Literal("it's a test")
        assert lit._to_template() == "'it\\'s a test'"


class TestWorkflowRef:
    """Tests for WorkflowRef expression class."""

    def test_workflow_id(self):
        ref = workflow.id
        assert isinstance(ref, WorkflowRef)
        assert str(ref) == "{{WORKFLOW.id}}"

    def test_workflow_name(self):
        ref = workflow.name
        assert str(ref) == "{{WORKFLOW.name}}"

    def test_workflow_ref_in_params(self):
        activity = Activity(
            key="log",
            worker="std",
            activity_name="echo",
            parameters={
                "workflow_id": workflow.id,
                "workflow_name": workflow.name,
            },
        )
        assert activity.parameters["workflow_id"] == "{{WORKFLOW.id}}"
        assert activity.parameters["workflow_name"] == "{{WORKFLOW.name}}"


class TestHelperFunctions:
    """Tests for helper functions (and_, or_, not_, etc.)."""

    def test_and_two_expressions(self):
        activity = Activity(key="test", worker="std", activity_name="echo")
        result = and_(activity["a"] > 1, activity["b"] < 10)
        assert isinstance(result, And)
        assert "&&" in str(result)

    def test_and_three_expressions(self):
        activity = Activity(key="test", worker="std", activity_name="echo")
        result = and_(
            activity["a"] > 1,
            activity["b"] < 10,
            activity["c"] == "ok",
        )
        # Should create nested And: And(And(a, b), c)
        assert "&&" in str(result)
        assert "test.a > 1" in str(result)
        assert "test.c == 'ok'" in str(result)

    def test_and_single_expression(self):
        activity = Activity(key="test", worker="std", activity_name="echo")
        result = and_(activity["a"] > 1)
        # Single expression should return the expression itself (wrapped)
        assert "test.a > 1" in str(result)

    def test_or_two_expressions(self):
        activity = Activity(key="test", worker="std", activity_name="echo")
        result = or_(activity["a"] > 1, activity["b"] < 10)
        assert isinstance(result, Or)
        assert "||" in str(result)

    def test_or_three_expressions(self):
        activity = Activity(key="test", worker="std", activity_name="echo")
        result = or_(
            activity["a"] > 1,
            activity["b"] < 10,
            activity["c"] == "ok",
        )
        assert "||" in str(result)

    def test_not_expression(self):
        activity = Activity(key="test", worker="std", activity_name="echo")
        result = not_(activity["valid"])
        assert isinstance(result, Not)
        assert "!(test.valid)" in str(result)

    def test_is_null(self):
        activity = Activity(key="test", worker="std", activity_name="echo")
        result = is_null(activity["result"])
        assert isinstance(result, IsNull)
        assert "test.result == null" in str(result)

    def test_is_not_null(self):
        activity = Activity(key="test", worker="std", activity_name="echo")
        result = is_not_null(activity["result"])
        assert isinstance(result, IsNotNull)
        assert "test.result != null" in str(result)

    def test_contains(self):
        activity = Activity(key="test", worker="std", activity_name="echo")
        result = contains(activity["tags"], "urgent")
        assert "test.tags contains 'urgent'" in str(result)

    def test_in_(self):
        activity = Activity(key="test", worker="std", activity_name="echo")
        result = in_(activity["status"], ["ok", "done"])
        assert "test.status in" in str(result)


class TestExpressionBase:
    """Tests for Expression base class functionality."""

    def test_all_expressions_inherit_from_expression(self):
        assert issubclass(Input, Expression)
        assert issubclass(OutputRef, Expression)
        assert issubclass(SecretRef, Expression)
        assert issubclass(EnvRef, Expression)
        assert issubclass(WorkflowRef, Expression)
        assert issubclass(Literal, Expression)
        assert issubclass(And, Expression)
        assert issubclass(Or, Expression)
        assert issubclass(Not, Expression)

    def test_expression_and_operator(self):
        activity = Activity(key="test", worker="std", activity_name="echo")
        e1 = activity["a"] > 1
        e2 = activity["b"] < 10
        result = e1 & e2
        assert isinstance(result, And)

    def test_expression_or_operator(self):
        activity = Activity(key="test", activity_name="echo")
        e1 = activity["a"] > 1
        e2 = activity["b"] < 10
        result = e1 | e2
        assert isinstance(result, Or)

    def test_expression_invert_operator(self):
        activity = Activity(key="test", activity_name="echo")
        e = activity["valid"]
        result = ~(e == True)  # noqa: E712
        assert isinstance(result, Not)

    def test_expression_rand_operator(self):
        """Test reversed AND operator (__rand__)."""
        activity = Activity(key="test", activity_name="echo")
        e = activity["a"] > 1
        # To trigger __rand__, the left operand must NOT be an Expression
        # and its __and__ must return NotImplemented
        # Using True (bool) - bool.__and__ returns NotImplemented for non-bool types
        result = True & e  # This calls e.__rand__(True)
        assert isinstance(result, And)

    def test_expression_ror_operator(self):
        """Test reversed OR operator (__ror__)."""
        activity = Activity(key="test", activity_name="echo")
        e = activity["a"] > 1
        # To trigger __ror__, similar to __rand__
        # Using True (bool) - bool.__or__ returns NotImplemented for non-bool types
        result = True | e  # This calls e.__ror__(True)
        assert isinstance(result, Or)


class TestLiteralProperty:
    """Tests for Literal value property."""

    def test_literal_value_property(self):
        """Test that Literal.value property returns the stored value."""
        lit = Literal(42)
        assert lit.value == 42

    def test_literal_value_property_string(self):
        lit = Literal("hello")
        assert lit.value == "hello"

    def test_literal_value_property_none(self):
        lit = Literal(None)
        assert lit.value is None


class TestWorkflowRefProperties:
    """Tests for WorkflowRef field property and repr."""

    def test_workflow_ref_field_property(self):
        """Test WorkflowRef.field property."""
        ref = workflow.id
        assert ref.field == "id"

    def test_workflow_ref_repr(self):
        """Test WorkflowRef __repr__."""
        ref = workflow.name
        assert repr(ref) == "WorkflowRef('name')"


class TestExpressionRepr:
    """Tests for __repr__ methods of expression classes."""

    def test_and_repr(self):
        """Test And.__repr__."""
        activity = Activity(key="test", activity_name="echo")
        e1 = activity["a"] > 1
        e2 = activity["b"] < 10
        result = e1 & e2
        repr_str = repr(result)
        assert "And(" in repr_str

    def test_or_repr(self):
        """Test Or.__repr__."""
        activity = Activity(key="test", activity_name="echo")
        e1 = activity["a"] > 1
        e2 = activity["b"] < 10
        result = e1 | e2
        repr_str = repr(result)
        assert "Or(" in repr_str

    def test_not_repr(self):
        """Test Not.__repr__."""
        activity = Activity(key="test", activity_name="echo")
        e = activity["valid"]
        result = ~(e == True)  # noqa: E712
        repr_str = repr(result)
        assert "Not(" in repr_str

    def test_is_null_repr(self):
        """Test IsNull.__repr__."""
        activity = Activity(key="test", activity_name="echo")
        result = is_null(activity["result"])
        repr_str = repr(result)
        assert "IsNull(" in repr_str

    def test_is_not_null_repr(self):
        """Test IsNotNull.__repr__."""
        activity = Activity(key="test", activity_name="echo")
        result = is_not_null(activity["result"])
        repr_str = repr(result)
        assert "IsNotNull(" in repr_str

    def test_contains_repr(self):
        """Test Contains.__repr__."""
        activity = Activity(key="test", activity_name="echo")
        result = contains(activity["tags"], "urgent")
        repr_str = repr(result)
        assert "Contains(" in repr_str

    def test_in_repr(self):
        """Test In.__repr__."""
        activity = Activity(key="test", activity_name="echo")
        result = in_(activity["status"], ["ok", "done"])
        repr_str = repr(result)
        assert "In(" in repr_str


class TestHelperFunctionsEdgeCases:
    """Tests for edge cases in helper functions."""

    def test_and_no_args_raises_error(self):
        """Test that and_() with no args raises ValueError."""
        import pytest

        with pytest.raises(ValueError, match="at least one expression"):
            and_()

    def test_or_no_args_raises_error(self):
        """Test that or_() with no args raises ValueError."""
        import pytest

        with pytest.raises(ValueError, match="at least one expression"):
            or_()

    def test_or_single_expression(self):
        """Test or_() with single expression returns that expression."""
        activity = Activity(key="test", activity_name="echo")
        expr = activity["a"] > 1
        result = or_(expr)
        # Should return the expression itself (not wrapped in Or)
        assert "test.a > 1" in str(result)


class TestEnvRefToTemplate:
    """Tests for EnvRef _to_template method."""

    def test_env_ref_to_template(self):
        """Test EnvRef._to_template() returns just the name."""
        env = EnvRef("DATABASE_URL")
        # _to_template is called internally but returns just the name
        # The __str__ method wraps it differently
        assert env._to_template() == "DATABASE_URL"
