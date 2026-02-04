"""User Validation Workflow - Conditional branching example.

This example demonstrates:
- Declarative activity definition with conditional dependencies
- Branching based on workflow input values
- Using secrets for sensitive configuration
- Database operations (postgres_query)
- Fan-out to parallel conditional branches
- Echo activity to pass through input data
"""

from kruxiaflow import Activity, Dependency, Input, SecretRef, Workflow, workflow

# Define workflow inputs
email = Input("email", type=str, required=True)
valid = Input("valid", type=bool, required=True)

# Secret for database connection
db_url = SecretRef("db_url")

# Step 1: Echo the input so conditional branches can check the "valid" field
check_email = Activity(
    key="check_email",
    worker="std",
    activity_name="echo",
    parameters={
        "email": str(email),
        "valid": str(valid),
    },
    outputs=["echo"],  # String auto-converted to ActivityOutputDefinition
)

# Step 2a: Store valid user (only runs if valid)
store_valid_user = Activity(
    key="store_valid_user",
    worker="std",
    activity_name="postgres_query",
    parameters={
        "db_url": str(db_url),
        "query": """
            INSERT INTO valid_users (email, validated_at) VALUES ($1, NOW())
            ON CONFLICT (email) DO NOTHING
        """,
        "params": [str(email)],
    },
    depends_on=[
        Dependency.on(check_email, check_email["echo.valid"] == True)  # noqa: E712
    ],
)

# Step 2b: Store invalid user (only runs if invalid)
store_invalid_user = Activity(
    key="store_invalid_user",
    worker="std",
    activity_name="postgres_query",
    parameters={
        "db_url": str(db_url),
        "query": """\
            INSERT INTO invalid_users (email, reason, checked_at)
            VALUES ($1, $2, NOW()) ON CONFLICT (email) DO NOTHING
        """,
        "params": [str(email), "Email validation failed"],
    },
    depends_on=[
        Dependency.on(check_email, check_email["echo.valid"] != True)  # noqa: E712
    ],
)

# Step 3: Send notification (runs after either store activity completes)
# Note: Both dependencies are listed with conditions; only one will execute
send_notification = Activity(
    key="send_notification",
    worker="std",
    activity_name="http_request",
    parameters={
        "method": "POST",
        "url": "http://mailpit:8025/api/v1/send",
        "headers": {"Content-Type": "application/json"},
        "body": {
            "From": {
                "Name": "Kruxia Flow",
                "Email": "workflow@kruxiaflow.local",
            },
            "To": [
                {
                    "Name": "Admin",
                    "Email": "admin@example.com",
                }
            ],
            "Subject": f"User Validation Result - {email!s}",
            "Text": f"""\
                Email: {email!s}
                Valid: {check_email["echo.valid"]}
                Workflow ID: {workflow.id}
            """,
        },
    },
    depends_on=[
        Dependency.on(store_valid_user, check_email["echo.valid"] == True),  # noqa: E712
        Dependency.on(store_invalid_user, check_email["echo.valid"] != True),  # noqa: E712
    ],
)

# Build the workflow
# Note: Description and input schemas are documentation only - the workflow definition
# only includes name and activities. Inputs are provided at runtime when submitting the workflow.
validation_workflow = Workflow(
    name="validate_user",
    activities=[check_email, store_valid_user, store_invalid_user, send_notification],
)

if __name__ == "__main__":
    # Print the compiled YAML to verify
    print(validation_workflow)
