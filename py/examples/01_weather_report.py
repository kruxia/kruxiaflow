"""Weather Report Workflow - Simple sequential workflow example.

This example demonstrates:
- Declarative activity definition with HTTP requests
- Activity dependencies (sequential execution)
- Referencing activity outputs in subsequent activities
- Using workflow metadata (workflow.id)
- Sending email notifications via Mailpit
"""

from kruxiaflow import Activity, Workflow, workflow

# Step 1: Fetch weather data from the weather API
fetch_weather = Activity(
    key="fetch_weather",
    worker="std",
    activity_name="http_request",
    parameters={
        "method": "GET",
        "url": "https://api.weather.gov/gridpoints/LOT/76,73/forecast",
    },
    outputs=["response"],  # String auto-converted to ActivityOutputDefinition
)

# Step 2: Send notification with weather data via email
# Depends on fetch_weather completing first
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
                    "Name": "Weather Subscriber",
                    "Email": "weather@example.com",
                }
            ],
            "Subject": f"Weather Report - {workflow.id}",
            "Text": f"""
                Temperature: {fetch_weather["response.body.properties.periods[0].temperature"]}
                Forecast: {fetch_weather["response.body.properties.periods[0].detailedForecast"]}
            """,
        },
    },
    depends_on=["fetch_weather"],
)

# Build the workflow
weather_workflow = Workflow(
    name="weather_report",
    activities=[fetch_weather, send_notification],
)

if __name__ == "__main__":
    # Print the compiled YAML to verify
    print(weather_workflow)
