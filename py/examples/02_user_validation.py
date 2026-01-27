"""User Validation Workflow - Conditional branching example.

This example demonstrates:
- Conditional dependencies with Dependency.on()
- Branching based on activity output values
- Using secrets for sensitive configuration
- Database operations (postgres_query)
- Fan-out to parallel conditional branches
"""

from kruxiaflow import Activity, Dependency, Input, SecretRef, Workflow, workflow

# Define workflow inputs
email = Input("email", type=str, required=True)
notification_webhook_url = Input("notification_webhook_url", type=str, required=True)

# Secret for database connection
db_url = SecretRef("db_url")

# Step 1: Check if email is valid using a validation service
check_email = (
    Activity(key="check_email")
    .with_worker("builtin", "http_request")
    .with_params(
        method="GET",
        url="https://httpbin.org/json",  # Mock validation endpoint
    )
)

# Step 2a: Store valid user (only runs if validation succeeded)
store_valid_user = (
    Activity(key="store_valid_user")
    .with_worker("builtin", "postgres_query")
    .with_params(
        db_url=db_url,
        query="""INSERT INTO valid_users (email, validated_at) VALUES ($1, NOW())
            ON CONFLICT (email) DO NOTHING""",
        params=[email],
    )
    .with_dependencies(
        Dependency.on(check_email, check_email["response.success"] == True)  # noqa: E712
    )
)

# Step 2b: Store invalid user (only runs if validation failed)
store_invalid_user = (
    Activity(key="store_invalid_user")
    .with_worker("builtin", "postgres_query")
    .with_params(
        db_url=db_url,
        query="""INSERT INTO invalid_users (email, reason, checked_at)
            VALUES ($1, $2, NOW()) ON CONFLICT (email) DO NOTHING""",
        params=[email, "Email validation failed"],
    )
    .with_dependencies(
        Dependency.on(check_email, check_email["response.success"] != True)  # noqa: E712
    )
)

# Step 3: Send notification (runs after either store activity completes)
# Note: Both dependencies are listed; one will be skipped based on its condition
send_notification = (
    Activity(key="send_notification")
    .with_worker("builtin", "http_request")
    .with_params(
        method="POST",
        url=notification_webhook_url,
        headers={"Content-Type": "application/json"},
        body={
            "email": email,
            "status": check_email["response.success"],
            "workflow_id": workflow.id,
        },
    )
    .with_dependencies(store_valid_user, store_invalid_user)
)

# Build the workflow
validation_workflow = (
    Workflow(name="validate_user")
    .with_inputs(email, notification_webhook_url)
    .with_activities(
        check_email, store_valid_user, store_invalid_user, send_notification
    )
)

if __name__ == "__main__":
    # Print the compiled YAML to verify
    print(validation_workflow.to_yaml())
