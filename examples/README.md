# StreamFlow Workflow Examples

This directory contains example workflows that demonstrate StreamFlow features progressively from simple to complex.

## Available Examples

| Example                  | Features Demonstrated                                                       | Prerequisites |
|--------------------------|-----------------------------------------------------------------------------|---------------|
| `01-weather-report.yaml` | Sequential workflow, HTTP request (GET/POST), headers, template expressions | Webhook URL   |

## Running Examples

### Example 1: Weather Report Pipeline

This workflow demonstrates a simple sequential workflow that:
1. Fetches weather data from the National Weather Service API (HTTP GET)
2. Sends a notification to a webhook with extracted weather data (HTTP POST)

**Prerequisites:**
- A webhook URL to receive the notification (e.g., webhook.site, requestbin.com)

**Run with StreamFlow CLI:**
```bash
streamflow run examples/01-weather-report.yaml \
  --input webhook_url=https://webhook.site/your-unique-id
```

**Expected behavior:**
1. Workflow fetches forecast data from weather.gov API
2. Extracts temperature and conditions from the response
3. Posts formatted data to your webhook URL
4. Webhook receives JSON with temperature, conditions, and workflow_id

**Features demonstrated:**
- ✅ YAML workflow definition parsing
- ✅ Sequential activity execution via `following` relationships
- ✅ HTTP GET request with custom headers
- ✅ HTTP POST request with JSON body
- ✅ Template expressions for input substitution (`{{INPUT.webhook_url}}`)
- ✅ Template expressions for activity output access (`{{fetch_weather.body.properties...}}`)
- ✅ Workflow context variables (`{{WORKFLOW.id}}`)

## Template Expression Syntax

StreamFlow supports the following template expression formats:

### Input Variables
Access workflow input parameters:
```yaml
url: "{{INPUT.webhook_url}}"  # Where to POST the results
```

### Activity Outputs
Access outputs from previous activities:
```yaml
temperature: "{{fetch_weather.body.properties.periods[0].temperature}}"
```

### Secrets
Access secret values (for API keys, tokens):
```yaml
headers:
  Authorization: "Bearer {{SECRET.api_key}}"
```

### Workflow Variables
Access workflow-level metadata:
```yaml
workflow_id: "{{WORKFLOW.id}}"
```

## Testing Examples

You can test examples using a webhook service:

1. **webhook.site** - Get a unique URL at https://webhook.site
2. **requestbin.com** - Create a bin at https://requestbin.com
3. **Local webhook** - Run a local server: `python -m http.server 8080`

## Next Steps

- Example 2 will demonstrate conditional branching with database operations
- Example 3 will show parallel execution with fan-out/fan-in patterns
- Example 4 will introduce LLM activities with cost tracking

See [docs/implementation/mvp-workflows-implementation-plan.md](../docs/implementation/mvp-workflows-implementation-plan.md) for the complete implementation roadmap.
