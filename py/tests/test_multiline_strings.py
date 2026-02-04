"""Tests for multiline string handling in YAML serialization."""

import yaml

from kruxiaflow.models import Activity, Workflow


class TestMultilineStrings:
    """Tests for multiline string YAML formatting."""

    def test_multiline_string_uses_block_literal_style(self):
        """Multiline strings should use |- block literal style."""
        activity = Activity(
            key="send_email",
            worker="std",
            activity_name="http_request",
            parameters={
                "method": "POST",
                "url": "http://example.com/send",
                "body": {
                    "subject": "Test Email",
                    "text": """
                        Line 1
                        Line 2
                        Line 3
                    """,
                },
            },
        )
        wf = Workflow(name="test", activities=[activity])
        yaml_str = wf.to_yaml()

        # Should use |- block literal style, not quoted strings with \n
        assert "|-" in yaml_str
        assert "\\n" not in yaml_str

        # Verify the YAML is still valid
        data = yaml.safe_load(yaml_str)
        text = data["activities"][0]["parameters"]["body"]["text"]

        # Should be dedented (no leading spaces) and properly formatted
        assert text == "Line 1\nLine 2\nLine 3"

    def test_indented_multiline_string_is_dedented(self):
        """Indented multiline strings should be dedented automatically."""
        activity = Activity(
            key="test",
            worker="std",
            activity_name="echo",
            parameters={
                "message": """
                    This is line 1
                    This is line 2
                    This is line 3
                """
            },
        )
        wf = Workflow(name="test", activities=[activity])
        yaml_str = wf.to_yaml()

        # Load and verify dedenting worked
        data = yaml.safe_load(yaml_str)
        message = data["activities"][0]["parameters"]["message"]

        # Should not have leading spaces on each line
        assert not message.startswith("    ")
        assert message == "This is line 1\nThis is line 2\nThis is line 3"

    def test_single_line_string_no_block_literal(self):
        """Single-line strings should not use block literal style."""
        activity = Activity(
            key="test",
            worker="std",
            activity_name="echo",
            parameters={"message": "Single line message"},
        )
        wf = Workflow(name="test", activities=[activity])
        yaml_str = wf.to_yaml()

        # Single line should not have |- for this specific param
        lines = yaml_str.split("\n")
        message_line = next(line for line in lines if "message:" in line)

        # Should be inline, not block literal
        assert "message: Single line message" in message_line

    def test_empty_lines_are_preserved(self):
        """Empty lines within multiline strings should be preserved."""
        activity = Activity(
            key="test",
            worker="std",
            activity_name="echo",
            parameters={
                "message": """
                    Paragraph 1

                    Paragraph 2
                """
            },
        )
        wf = Workflow(name="test", activities=[activity])
        yaml_str = wf.to_yaml()

        data = yaml.safe_load(yaml_str)
        message = data["activities"][0]["parameters"]["message"]

        # Should have empty line between paragraphs
        assert "\n\n" in message

    def test_multiline_with_special_characters(self):
        """Multiline strings with special characters should be properly escaped."""
        activity = Activity(
            key="test",
            worker="std",
            activity_name="echo",
            parameters={
                "sql": """
                    SELECT * FROM users
                    WHERE name = 'John''s'
                    AND email LIKE '%@example.com'
                """
            },
        )
        wf = Workflow(name="test", activities=[activity])
        yaml_str = wf.to_yaml()

        # Should parse correctly
        data = yaml.safe_load(yaml_str)
        sql = data["activities"][0]["parameters"]["sql"]

        # Special characters should be preserved
        assert "John''s" in sql or "John's" in sql
        assert "%@example.com" in sql
