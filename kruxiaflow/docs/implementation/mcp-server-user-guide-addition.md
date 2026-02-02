# MCP Server User Guide Addition

This document contains content to be added to `docs/implementation/mcp-userguide.md` after the Rust MCP server implementation is complete.

---

## Embedded MCP Server

Kruxia Flow includes an integrated MCP (Model Context Protocol) server that enables AI agents like Claude to interact with your workflow orchestration system through a standardized interface.

### Important: HTTP Transport Only

**The integrated MCP server only supports HTTP transport, not stdio transport.**

**Why no stdio?**
- The `serve` command runs multiple services (API server, orchestrator, workers, MCP server)
- All services log to stdout/stderr for observability
- MCP stdio requires **clean stdin/stdout** with no other output
- Mixing logs with stdio MCP corrupts the JSON-RPC message stream

**For stdio MCP (e.g., Claude Desktop):**
Use a separate process that runs ONLY the MCP server:
- **Python MCP server**: `kruxiaflow-mcp/` (already available in the repository)
- **Standalone Rust MCP server**: Create a dedicated binary with logging to file/syslog

The integrated HTTP MCP server is designed for production deployments where logging, metrics, and multi-client access are essential.

### Enabling the MCP Server

The MCP server is **disabled by default**. Enable it when starting Kruxia Flow:

```bash
# HTTP transport (only supported transport)
export KRUXIAFLOW_MCP_ENABLED=true
export KRUXIAFLOW_MCP_HTTP_PORT=8081
export KRUXIAFLOW_MCP_JWT_SECRET="your-secret-key"
kruxiaflow serve
```

Or using CLI flags:

```bash
kruxiaflow serve \
  --mcp-enabled \
  --mcp-http-port 8081 \
  --mcp-http-bind 0.0.0.0
```

### Transport Configuration

**HTTP Transport** (only supported)
- Network-accessible endpoint
- Multiple concurrent clients
- Authentication required by default
- SSE (Server-Sent Events) based

```bash
# Production configuration
export KRUXIAFLOW_MCP_ENABLED=true
export KRUXIAFLOW_MCP_TRANSPORT=http
export KRUXIAFLOW_MCP_HTTP_PORT=8081
export KRUXIAFLOW_MCP_HTTP_BIND=0.0.0.0
export KRUXIAFLOW_MCP_AUTH_REQUIRED=true
export KRUXIAFLOW_MCP_JWT_SECRET="<secure-secret>"

kruxiaflow serve
```

## Security Configuration

### Security Requirements

**⚠️ CRITICAL: The MCP server exposes workflow orchestration capabilities to AI agents. Proper security configuration is mandatory for production deployments.**

### Authentication (REQUIRED for Production)

**HTTP transport requires authentication by default** for security:

```bash
# JWT authentication (REQUIRED for production)
export KRUXIAFLOW_MCP_ENABLED=true
export KRUXIAFLOW_MCP_AUTH_REQUIRED=true
export KRUXIAFLOW_MCP_JWT_SECRET="your-secure-secret-key"

kruxiaflow serve
```

#### JWT Token Management

**Generating Secure JWT Secrets:**

```bash
# Generate a secure random secret (256-bit minimum)
openssl rand -base64 32

# Store in environment or secrets management system
export KRUXIAFLOW_MCP_JWT_SECRET="$(openssl rand -base64 32)"
```

**JWT Token Structure:**
The MCP server uses the same JWT infrastructure as the Kruxia Flow API server. Tokens must include:
- `sub` (subject): Client identifier
- `iss` (issuer): "kruxiaflow"
- `aud` (audience): "kruxiaflow-api"
- `exp` (expiration): Token expiry timestamp

**Generating Client Tokens:**

```bash
# Use the API's OAuth token endpoint or create tokens with matching secret
# Example using the API:
curl -X POST http://localhost:8080/api/v1/oauth/token \
  -H "Content-Type: application/x-www-form-urlencoded" \
  -d "grant_type=client_credentials" \
  -d "client_id=mcp_client" \
  -d "client_secret=<client_secret>"
```

**Client Configuration (VS Code example):**

```json
{
  "mcpServers": {
    "kruxiaflow": {
      "type": "http",
      "url": "http://localhost:8081",
      "auth": {
        "type": "bearer",
        "token": "${env:KRUXIAFLOW_MCP_TOKEN}"
      }
    }
  }
}
```

#### Disabling Authentication (DEVELOPMENT ONLY)

**⚠️ WARNING: Never disable authentication in production!**

```bash
# ONLY for local development/testing
export KRUXIAFLOW_MCP_ENABLED=true
export KRUXIAFLOW_MCP_AUTH_REQUIRED=false

kruxiaflow serve
# WARNING: MCP HTTP transport without authentication is INSECURE!
# Anyone with network access can execute workflows!
```

### Network Security

#### Bind Address Configuration

**Production (behind reverse proxy):**
```bash
# Bind to localhost - requires reverse proxy for external access
export KRUXIAFLOW_MCP_HTTP_BIND=127.0.0.1
export KRUXIAFLOW_MCP_HTTP_PORT=8081
```

**Internal network (trusted network only):**
```bash
# Bind to all interfaces - use only on isolated networks
export KRUXIAFLOW_MCP_HTTP_BIND=0.0.0.0
export KRUXIAFLOW_MCP_HTTP_PORT=8081
```

**⚠️ CRITICAL: Never expose MCP port directly to the internet!**

#### TLS/HTTPS Requirements

**The MCP server does NOT provide native TLS support.** Use a reverse proxy for production:

**Nginx configuration:**
```nginx
upstream mcp_backend {
    server 127.0.0.1:8081;
}

server {
    listen 443 ssl http2;
    server_name mcp.example.com;

    ssl_certificate /etc/ssl/certs/mcp.example.com.crt;
    ssl_certificate_key /etc/ssl/private/mcp.example.com.key;
    ssl_protocols TLSv1.2 TLSv1.3;
    ssl_ciphers HIGH:!aNULL:!MD5;

    location / {
        proxy_pass http://mcp_backend;
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;

        # Timeouts for long-running operations
        proxy_read_timeout 300s;
        proxy_connect_timeout 75s;
    }
}
```

**Caddy configuration:**
```caddyfile
mcp.example.com {
    reverse_proxy localhost:8081 {
        header_up X-Real-IP {remote_host}
        header_up X-Forwarded-For {remote_host}
    }
}
```

### Access Control

#### IP Allowlisting (Reverse Proxy Level)

```nginx
# Nginx: Restrict to specific IP ranges
server {
    # ... SSL config ...

    # Allow corporate network
    allow 10.0.0.0/8;
    allow 172.16.0.0/12;

    # Allow VPN gateway
    allow 198.51.100.0/24;

    # Deny all others
    deny all;

    location / {
        proxy_pass http://mcp_backend;
    }
}
```

#### Client Authorization

Future enhancement: Per-client tool access control
- Currently: All authenticated clients have full access
- Planned: Role-based access control (RBAC) for tool restrictions

### Secret Management

**❌ NEVER:**
- Commit secrets to git
- Hardcode secrets in configuration files
- Log JWT secrets or tokens
- Share secrets across environments

**✅ RECOMMENDED:**

**Development:**
```bash
# Use .env file (add to .gitignore)
echo "KRUXIAFLOW_MCP_JWT_SECRET=$(openssl rand -base64 32)" >> .env.local
source .env.local
```

**Production (Docker Secrets):**
```yaml
# docker-compose.yml
version: '3.8'
services:
  kruxiaflow:
    image: kruxiaflow:latest
    secrets:
      - mcp_jwt_secret
    environment:
      KRUXIAFLOW_MCP_JWT_SECRET_FILE: /run/secrets/mcp_jwt_secret

secrets:
  mcp_jwt_secret:
    external: true
```

**Production (Kubernetes Secrets):**
```yaml
apiVersion: v1
kind: Secret
metadata:
  name: kruxiaflow-mcp
type: Opaque
data:
  jwt-secret: <base64-encoded-secret>
---
apiVersion: apps/v1
kind: Deployment
metadata:
  name: kruxiaflow
spec:
  template:
    spec:
      containers:
      - name: kruxiaflow
        env:
        - name: KRUXIAFLOW_MCP_JWT_SECRET
          valueFrom:
            secretKeyRef:
              name: kruxiaflow-mcp
              key: jwt-secret
```

**Production (AWS Secrets Manager):**
```bash
# Retrieve secret at runtime
export KRUXIAFLOW_MCP_JWT_SECRET=$(aws secretsmanager get-secret-value \
  --secret-id kruxiaflow/mcp/jwt-secret \
  --query SecretString \
  --output text)
```

### Security Best Practices

#### Production Security Checklist

**Before deploying to production:**

- [ ] **Authentication enabled**: `KRUXIAFLOW_MCP_AUTH_REQUIRED=true`
- [ ] **Strong JWT secret**: Minimum 32 bytes, randomly generated
- [ ] **TLS/HTTPS**: Reverse proxy with valid certificate
- [ ] **Network isolation**: Bind to localhost or private network
- [ ] **Firewall rules**: MCP port not exposed to public internet
- [ ] **IP allowlisting**: Restrict to known client IPs (if applicable)
- [ ] **Secrets management**: Use vault/secrets manager, not environment files
- [ ] **Resource limits**: Set `MAX_CONCURRENT_REQUESTS` and timeouts
- [ ] **Monitoring**: Enable structured logging and alerts
- [ ] **Audit logging**: Track all MCP tool invocations (future enhancement)
- [ ] **Regular updates**: Keep rust-mcp-sdk and dependencies current
- [ ] **Token rotation**: Rotate JWT secrets periodically

#### Development vs Production

**Development Environment:**
```bash
# Acceptable for local development only
export KRUXIAFLOW_MCP_ENABLED=true
export KRUXIAFLOW_MCP_HTTP_PORT=8081
export KRUXIAFLOW_MCP_HTTP_BIND=127.0.0.1
export KRUXIAFLOW_MCP_AUTH_REQUIRED=false  # ⚠️ Dev only!
```

**Staging Environment:**
```bash
# Production-like security
export KRUXIAFLOW_MCP_ENABLED=true
export KRUXIAFLOW_MCP_HTTP_PORT=8081
export KRUXIAFLOW_MCP_HTTP_BIND=127.0.0.1
export KRUXIAFLOW_MCP_AUTH_REQUIRED=true
export KRUXIAFLOW_MCP_JWT_SECRET="<staging-secret>"
export KRUXIAFLOW_MCP_MAX_CONCURRENT_REQUESTS=10
export KRUXIAFLOW_MCP_REQUEST_TIMEOUT_SECS=60
```

**Production Environment:**
```bash
# Maximum security
export KRUXIAFLOW_MCP_ENABLED=true
export KRUXIAFLOW_MCP_HTTP_PORT=8081
export KRUXIAFLOW_MCP_HTTP_BIND=127.0.0.1  # Behind reverse proxy
export KRUXIAFLOW_MCP_AUTH_REQUIRED=true
export KRUXIAFLOW_MCP_JWT_SECRET="<from-secrets-manager>"
export KRUXIAFLOW_MCP_MAX_CONCURRENT_REQUESTS=20
export KRUXIAFLOW_MCP_REQUEST_TIMEOUT_SECS=60
export KRUXIAFLOW_LOG_FORMAT=json
export KRUXIAFLOW_LOG_LEVEL=info
```

### Security Monitoring

#### Logging Security Events

**Enable structured logging for security analysis:**
```bash
export KRUXIAFLOW_LOG_FORMAT=json
export KRUXIAFLOW_LOG_LEVEL=info
```

**Security events to monitor:**
- Authentication failures (401 responses)
- Unauthorized access attempts
- Rate limit hits
- Unusual workflow submission patterns
- High-frequency tool usage

#### Example: Monitoring with Splunk/ELK

```json
// Query for authentication failures
{
  "query": {
    "bool": {
      "must": [
        { "match": { "service": "mcp" } },
        { "match": { "status": 401 } }
      ]
    }
  }
}
```

### Incident Response

**If you suspect unauthorized access:**

1. **Immediately rotate JWT secrets:**
   ```bash
   # Generate new secret
   NEW_SECRET=$(openssl rand -base64 32)

   # Update configuration
   export KRUXIAFLOW_MCP_JWT_SECRET="$NEW_SECRET"

   # Restart service
   systemctl restart kruxiaflow
   ```

2. **Invalidate existing tokens**: Restart the service (clears in-memory token cache)

3. **Review audit logs**: Check for unauthorized workflow submissions

4. **Update firewall rules**: Restrict access if source identified

5. **Regenerate client credentials**: Issue new tokens to legitimate clients

### Resource Limits

Prevent resource exhaustion:

```bash
# Concurrent request limit
export KRUXIAFLOW_MCP_MAX_CONCURRENT_REQUESTS=20

# Request timeout
export KRUXIAFLOW_MCP_REQUEST_TIMEOUT_SECS=60

kruxiaflow serve --mcp-enabled
```

### Complete Configuration Reference

| Environment Variable                      | Default     | Security Impact | Description                                   |
| ----------------------------------------- | ----------- | --------------- | --------------------------------------------- |
| `KRUXIAFLOW_MCP_ENABLED`                  | `false`     | Medium          | Enable MCP server (disabled by default for security) |
| `KRUXIAFLOW_MCP_TRANSPORT`                | `http`      | Low             | Transport type (only 'http' supported)        |
| `KRUXIAFLOW_MCP_HTTP_PORT`                | `8081`      | Low             | HTTP port (use non-standard port)             |
| `KRUXIAFLOW_MCP_HTTP_BIND`                | `0.0.0.0`   | **CRITICAL**    | Bind address - use `127.0.0.1` in production |
| `KRUXIAFLOW_MCP_AUTH_REQUIRED`            | `true`      | **CRITICAL**    | Require authentication (NEVER disable in production) |
| `KRUXIAFLOW_MCP_JWT_SECRET`               | -           | **CRITICAL**    | JWT secret key (min 32 bytes, use secrets manager) |
| `KRUXIAFLOW_MCP_MAX_CONCURRENT_REQUESTS`  | `10`        | Medium          | Max concurrent requests (DoS protection)      |
| `KRUXIAFLOW_MCP_REQUEST_TIMEOUT_SECS`     | `30`        | Low             | Request timeout in seconds (resource protection) |

**Security Notes:**
- **CRITICAL** settings directly impact system security - misconfiguration can expose your infrastructure
- **Medium** settings affect resilience and DoS protection
- **Low** settings have minimal security impact

**Note:** This configuration is simplified for MVP. Advanced security features (role-based access control, per-tool rate limits, audit logging) will be added in Phase 7 (Production Hardening).

## Common Deployment Scenarios

### Local Development with Claude Desktop

**Important:** The integrated MCP server does not support stdio. For Claude Desktop, use the Python MCP server (`kruxiaflow-mcp/`) which runs as a separate process.

See `kruxiaflow-mcp/README.md` for Claude Desktop configuration instructions.

### Local Development (HTTP - Insecure)

**⚠️ WARNING: This configuration is INSECURE - use only on isolated development machines!**

```bash
export DATABASE_URL="postgres://localhost/kruxiaflow"
export KRUXIAFLOW_MCP_ENABLED=true
export KRUXIAFLOW_MCP_HTTP_PORT=8081
export KRUXIAFLOW_MCP_HTTP_BIND=127.0.0.1  # Localhost only
export KRUXIAFLOW_MCP_AUTH_REQUIRED=false  # ⚠️ INSECURE - Dev only!
export KRUXIAFLOW_LOG_LEVEL=debug

kruxiaflow serve --port 8080
```

**Access MCP at:** `http://localhost:8081`

**Security Notes:**
- No authentication - anyone with localhost access can execute workflows
- Bind to 127.0.0.1 prevents network access
- Never use this configuration on shared/cloud systems

### Local Development (HTTP - Secure)

**Recommended for development with proper security:**

```bash
export DATABASE_URL="postgres://localhost/kruxiaflow"
export KRUXIAFLOW_MCP_ENABLED=true
export KRUXIAFLOW_MCP_HTTP_PORT=8081
export KRUXIAFLOW_MCP_HTTP_BIND=127.0.0.1
export KRUXIAFLOW_MCP_AUTH_REQUIRED=true
export KRUXIAFLOW_MCP_JWT_SECRET="$(openssl rand -base64 32)"
export KRUXIAFLOW_LOG_LEVEL=debug

# Generate a test token
export MCP_TOKEN=$(curl -X POST http://localhost:8080/api/v1/oauth/token \
  -H "Content-Type: application/x-www-form-urlencoded" \
  -d "grant_type=client_credentials" \
  -d "client_id=dev_mcp_client" \
  -d "client_secret=dev_secret" | jq -r .access_token)

kruxiaflow serve --port 8080
```

### Production Deployment (Reverse Proxy)

**✅ SECURE: Production-ready configuration**

```bash
export DATABASE_URL="postgres://prod-db:5432/kruxiaflow"
export KRUXIAFLOW_MCP_ENABLED=true
export KRUXIAFLOW_MCP_HTTP_PORT=8081
export KRUXIAFLOW_MCP_HTTP_BIND=127.0.0.1  # Behind nginx/caddy
export KRUXIAFLOW_MCP_AUTH_REQUIRED=true
export KRUXIAFLOW_MCP_JWT_SECRET="<from-secrets-manager>"
export KRUXIAFLOW_MCP_MAX_CONCURRENT_REQUESTS=20
export KRUXIAFLOW_MCP_REQUEST_TIMEOUT_SECS=60
export KRUXIAFLOW_LOG_FORMAT=json
export KRUXIAFLOW_LOG_LEVEL=info

kruxiaflow serve
```

**Required reverse proxy (Nginx/Caddy) for:**
- TLS/HTTPS termination
- IP allowlisting
- Rate limiting (layer 7)
- Request logging
- DDoS protection

**External access:** `https://mcp.example.com` (via reverse proxy on port 443)

### Production Deployment (Container - Docker)

```yaml
version: '3.8'
services:
  kruxiaflow:
    image: kruxiaflow:latest
    networks:
      - internal
    environment:
      DATABASE_URL: postgres://db:5432/kruxiaflow
      KRUXIAFLOW_MCP_ENABLED: "true"
      KRUXIAFLOW_MCP_HTTP_PORT: "8081"
      KRUXIAFLOW_MCP_HTTP_BIND: "0.0.0.0"  # OK in isolated network
      KRUXIAFLOW_MCP_AUTH_REQUIRED: "true"
      KRUXIAFLOW_LOG_FORMAT: json
    secrets:
      - mcp_jwt_secret
    # No port exposure - accessed via reverse proxy only

  nginx:
    image: nginx:alpine
    ports:
      - "443:443"
    networks:
      - internal
      - external
    volumes:
      - ./nginx.conf:/etc/nginx/nginx.conf:ro
      - ./ssl:/etc/ssl:ro

networks:
  internal:
    internal: true  # No external access
  external:

secrets:
  mcp_jwt_secret:
    external: true
```

### Production Deployment (Kubernetes)

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: kruxiaflow
spec:
  replicas: 3
  template:
    spec:
      containers:
      - name: kruxiaflow
        image: kruxiaflow:latest
        env:
        - name: KRUXIAFLOW_MCP_ENABLED
          value: "true"
        - name: KRUXIAFLOW_MCP_HTTP_PORT
          value: "8081"
        - name: KRUXIAFLOW_MCP_HTTP_BIND
          value: "0.0.0.0"  # OK - Service provides isolation
        - name: KRUXIAFLOW_MCP_AUTH_REQUIRED
          value: "true"
        - name: KRUXIAFLOW_MCP_JWT_SECRET
          valueFrom:
            secretKeyRef:
              name: kruxiaflow-mcp
              key: jwt-secret
        ports:
        - containerPort: 8081
          name: mcp
          protocol: TCP
        securityContext:
          runAsNonRoot: true
          runAsUser: 1000
          readOnlyRootFilesystem: true
          allowPrivilegeEscalation: false
---
apiVersion: v1
kind: Service
metadata:
  name: kruxiaflow-mcp
spec:
  type: ClusterIP  # Internal only
  selector:
    app: kruxiaflow
  ports:
  - port: 8081
    targetPort: mcp
---
apiVersion: networking.k8s.io/v1
kind: Ingress
metadata:
  name: kruxiaflow-mcp
  annotations:
    cert-manager.io/cluster-issuer: letsencrypt-prod
    nginx.ingress.kubernetes.io/whitelist-source-range: "10.0.0.0/8,172.16.0.0/12"
spec:
  tls:
  - hosts:
    - mcp.example.com
    secretName: mcp-tls
  rules:
  - host: mcp.example.com
    http:
      paths:
      - path: /
        pathType: Prefix
        backend:
          service:
            name: kruxiaflow-mcp
            port:
              number: 8081
```

## Adding Custom MCP Tools

If you build custom activities or extend Kruxia Flow, you can add custom MCP tools to expose them to AI agents.

### Step 1: Define the Tool

Create a new tool in the appropriate category module (e.g., `kruxiaflow/src/mcp/tools/custom.rs`):

```rust
use rust_mcp_sdk::prelude::*;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

/// Execute custom analytics query
pub async fn run_analytics_query(
    pool: &PgPool,
    params: CallToolRequestParams,
) -> Result<CallToolResult, CallToolError> {
    // Parse parameters
    let args: AnalyticsQueryArgs = serde_json::from_value(params.arguments)
        .map_err(|e| CallToolError::invalid_params(e.to_string()))?;

    // Execute query
    let results = execute_analytics_query(pool, &args).await
        .map_err(|e| CallToolError::internal_error(e.to_string()))?;

    // Format response
    let response = serde_json::json!({
        "query_id": args.query_id,
        "results": results,
        "row_count": results.len(),
    });

    Ok(CallToolResult::text_content(vec![
        serde_json::to_string_pretty(&response).unwrap()
    ]))
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct AnalyticsQueryArgs {
    query_id: String,
    parameters: serde_json::Value,
}
```

### Step 2: Register the Tool

Add to the tool listing function:

```rust
// In kruxiaflow/src/mcp/tools/custom.rs

pub fn list_tools(config: &McpConfig) -> Vec<Tool> {
    let mut tools = Vec::new();

    if config.is_tool_enabled("run_analytics_query") {
        tools.push(Tool {
            name: "run_analytics_query".into(),
            description: Some("Execute a predefined analytics query".into()),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query_id": {
                        "type": "string",
                        "description": "Analytics query identifier"
                    },
                    "parameters": {
                        "type": "object",
                        "description": "Query parameters"
                    }
                },
                "required": ["query_id"]
            }),
        });
    }

    tools
}
```

### Step 3: Route Tool Calls

Add routing in the handler:

```rust
// In kruxiaflow/src/mcp/handler.rs

async fn handle_call_tool_request(
    &self,
    params: CallToolRequestParams,
    _runtime: Arc<dyn McpServer>,
) -> Result<CallToolResult, CallToolError> {
    // ... existing tools ...

    // Custom tools
    match params.name.as_str() {
        "run_analytics_query" => {
            tools::custom::run_analytics_query(&self.pool, params).await
        }
        _ => Err(CallToolError::unknown_tool(params.name)),
    }
}
```

### Step 4: Add to Tool Lists

Update the list_tools handler:

```rust
// In kruxiaflow/src/mcp/handler.rs

async fn handle_list_tools_request(
    &self,
    _request: Option<PaginatedRequestParams>,
    _runtime: Arc<dyn McpServer>,
) -> Result<ListToolsResult, RpcError> {
    let mut tools = Vec::new();

    // ... existing features ...

    // Custom tools (always enabled if MCP is enabled)
    tools.extend(tools::custom::list_tools(&self.config));

    Ok(ListToolsResult {
        tools,
        meta: None,
        next_cursor: None,
    })
}
```

### Best Practices for Custom Tools

1. **Use descriptive names**: `run_analytics_query` not `query`
2. **Document parameters**: Include JSON schema with descriptions
3. **Error handling**: Return helpful error messages via `CallToolError`
4. **Validate inputs**: Check parameters before database access
5. **Rate limit**: Add to `KRUXIAFLOW_MCP_TOOL_RATE_LIMITS` if expensive
6. **Audit log**: Sensitive operations should be logged
7. **Test thoroughly**: Unit tests + integration tests with MCP client

## Troubleshooting

### MCP Server Won't Start

Check logs for configuration errors:
```bash
kruxiaflow serve --mcp-enabled --log-level debug 2>&1 | grep MCP
```

Common issues:
- HTTP transport without port: Set `KRUXIAFLOW_MCP_HTTP_PORT`
- Missing authentication credentials
- Invalid feature names in `KRUXIAFLOW_MCP_FEATURES`

### Tool Not Available

Check tool is enabled:
```bash
export RUST_LOG=debug
kruxiaflow serve --mcp-enabled
# Check logs for tool listing
```

### Authentication Failures

Verify JWT secret is set:
```bash
export KRUXIAFLOW_MCP_JWT_SECRET="$(cat /run/secrets/jwt_secret)"
kruxiaflow serve --mcp-enabled
```

### Connection Refused

Verify the MCP server is listening:
```bash
# Check logs
kruxiaflow serve --mcp-enabled | grep "MCP"

# Verify port is listening
curl http://localhost:8081/health  # If health endpoint exists
```

---

**End of User Guide Addition**
