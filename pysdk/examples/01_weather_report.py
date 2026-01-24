"""Weather Report Workflow - Simple sequential workflow example.

This example demonstrates:
- Basic activity definition with HTTP requests
- Activity dependencies (sequential execution)
- Referencing activity outputs in subsequent activities
- Referencing workflow inputs
"""

from kruxiaflow import Activity, Input, Workflow

# Define workflow inputs
webhook_url = Input("webhook_url", type=str, required=True)

# Step 1: Fetch weather data from the weather API
fetch_weather = (
    Activity(key="fetch_weather")
    .with_worker("builtin", "http_request")
    .with_params(
        method="GET",
        url="https://api.weather.gov/gridpoints/LOT/76,73/forecast",
    )
)

# Step 2: Send notification with weather data to webhook
# Depends on fetch_weather completing first
send_notification = (
    Activity(key="send_notification")
    .with_worker("builtin", "http_request")
    .with_params(
        method="POST",
        url=webhook_url,
        headers={"Content-Type": "application/json"},
        body={
            "temperature": f"{fetch_weather['response.json.properties.periods[0].temperature']}",
            "conditions": f"{fetch_weather['response.json.properties.periods[0].shortForecast']}",
            "workflow_id": "{{WORKFLOW.id}}",
        },
    )
    .with_dependencies(fetch_weather)
)

# Build the workflow
workflow = (
    Workflow(name="weather_report")
    .with_inputs(webhook_url)
    .with_activities(fetch_weather, send_notification)
)

if __name__ == "__main__":
    # Print the compiled YAML to verify
    print(workflow.to_yaml())
