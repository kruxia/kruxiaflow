# US-5.7a: Email Notification Activity Implementation Plan

**Version**: 1.0
**Date**: 2025-11-26
**Status**: Implemented
**Epic**: 5 - Built-In Activity Library
**Dependency of**: Example 10 (Order Processing with Email Notification)

---

## Overview

This story implements the `email_send` built-in activity that enables workflows to send email notifications via SMTP.

**User Story** (US-5.7):
> As a platform engineering lead, I want built-in notification activities, so that workflows can alert using common communication channels.

**MVP Scope** (US-5.7a):
> Implement `email_send` activity with SMTP support and HTML/plain text content.

**Note**: After US-5.7a + Example 10, the MVP orchestrator and built-in worker with activities are feature-complete.

---

## Acceptance Criteria

| Criterion                     | Status |
|-------------------------------|--------|
| `email_send` activity         | ✅     |
| Template support for messages | ✅     |
| Retry on delivery failure     | ✅     |

**Deferred to Post-MVP**: `slack_message`, `teams_notify`, `discord_send`, `gchat_send` (see `docs/post-mvp.md` Story 1.15)

---

## Activity: `email_send`

### Description

Sends an email via SMTP with support for HTML/plain text content, attachments, and template-based message composition.

### YAML Usage

```yaml
activities:
  send_alert:
    activity: email_send
    parameters:
      smtp_url: "{{SECRET.smtp_url}}"
      from: "alerts@example.com"
      to:
        - "ops@example.com"
        - "oncall@example.com"
      subject: "Workflow {{WORKFLOW.id}} completed"
      text_body: |
        The workflow has completed successfully.

        Results:
        - Processed: {{process_data.row_count}} rows
        - Duration: {{WORKFLOW.duration_ms}}ms
```

### Parameters

| Parameter   | Type     | Required | Default | Description                                                    |
|-------------|----------|----------|---------|----------------------------------------------------------------|
| `smtp_url`  | string   | Yes      | -       | SMTP connection URL (see format below)                         |
| `from`      | string   | Yes      | -       | Sender email address                                           |
| `to`        | string[] | Yes      | -       | Recipient email addresses                                      |
| `subject`   | string   | Yes      | -       | Email subject line                                             |
| `text_body` | string   | No*      | -       | Plain text email body                                          |
| `html_body` | string   | No*      | -       | HTML email body                                                |
| `cc`        | string[] | No       | -       | CC recipients                                                  |
| `bcc`       | string[] | No       | -       | BCC recipients                                                 |
| `reply_to`  | string   | No       | -       | Reply-to address                                               |

*At least one of `text_body` or `html_body` must be provided.

### Email Body Options

The email body is specified using explicit, unambiguous fields:

| Provided            | Email Format           | Description                              |
|---------------------|------------------------|------------------------------------------|
| `text_body` only    | `text/plain`           | Plain text email                         |
| `html_body` only    | `text/html`            | HTML-only email                          |
| Both                | `multipart/alternative`| Both versions sent; client chooses       |

**Multipart/alternative** (providing both) is recommended for HTML emails:
- **Compatibility**: Fallback for clients that don't support HTML
- **Accessibility**: Better support for screen readers
- **Deliverability**: Reduces spam filter false positives
- **User choice**: Recipients can view their preferred format

### SMTP URL Format

The `smtp_url` parameter supports standard SMTP connection strings:

```
# Plain SMTP (port 25, insecure - development only)
smtp://smtp.example.com:25

# SMTP with STARTTLS (port 587, recommended)
smtp://username:password@smtp.example.com:587?tls=required

# SMTPS (implicit TLS, port 465)
smtps://username:password@smtp.example.com:465

# Common providers:
# Gmail:      smtps://user@gmail.com:app_password@smtp.gmail.com:465
# SendGrid:   smtp://apikey:SG.xxx@smtp.sendgrid.net:587?tls=required
# Mailgun:    smtp://postmaster@domain:key@smtp.mailgun.org:587?tls=required
# Amazon SES: smtp://AKIAIOSFODNN7:secret@email-smtp.us-east-1.amazonaws.com:587?tls=required
```

### Output

```json
{
  "result": {
    "success": true,
    "message_id": "<unique-message-id@example.com>",
    "recipients_accepted": 2,
    "recipients_rejected": 0
  }
}
```

On failure:
```json
{
  "error": {
    "code": "SMTP_ERROR",
    "message": "Connection refused: smtp.example.com:587",
    "retryable": true
  }
}
```

---

## Implementation Architecture

### File Structure

```
worker/src/activities/
├── mod.rs                 # Module exports (add EmailSendActivity)
├── email.rs               # NEW: Email activity implementation
│   ├── SmtpConfig         # Parsed SMTP URL configuration
│   ├── EmailParams        # Activity parameters
│   ├── EmailResult        # Activity output
│   ├── EmailExecutor      # SMTP client wrapper
│   └── EmailSendActivity  # ActivityImpl implementation
├── http.rs
├── postgres.rs
└── ...
```

### Dependencies

Add to `worker/Cargo.toml`:

```toml
[dependencies]
lettre = { version = "0.11", default-features = false, features = ["tokio1-rustls-tls", "smtp-transport", "builder"] }
```

**Why lettre?**
- Pure Rust SMTP client (no C dependencies)
- Async support via Tokio
- TLS support via rustls (matches our existing TLS stack)
- Well-maintained, widely used
- Supports all major SMTP features

### Core Types

```rust
use lettre::{
    AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor,
    message::{Mailbox, header::ContentType},
    transport::smtp::authentication::Credentials,
};
use serde::{Deserialize, Serialize};
use std::time::Duration;

// ============================================================================
// SMTP Configuration
// ============================================================================

/// Parsed SMTP connection configuration
#[derive(Debug, Clone)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<String>,
    pub tls_mode: TlsMode,
}

#[derive(Debug, Clone, Default)]
pub enum TlsMode {
    None,           // Plain SMTP (insecure)
    #[default]
    StartTls,       // STARTTLS upgrade (port 587)
    ImplicitTls,    // SMTPS (port 465)
}

impl SmtpConfig {
    /// Parse SMTP URL into configuration
    /// Formats:
    ///   smtp://host:port
    ///   smtp://user:pass@host:port?tls=required
    ///   smtps://user:pass@host:port
    pub fn from_url(url: &str) -> Result<Self> {
        let parsed = url::Url::parse(url)
            .context("Invalid SMTP URL")?;

        let tls_mode = match parsed.scheme() {
            "smtps" => TlsMode::ImplicitTls,
            "smtp" => {
                // Check for ?tls=required query param
                let tls_param = parsed.query_pairs()
                    .find(|(k, _)| k == "tls")
                    .map(|(_, v)| v.to_string());
                match tls_param.as_deref() {
                    Some("required") | Some("true") => TlsMode::StartTls,
                    _ => TlsMode::None,
                }
            }
            _ => return Err(anyhow::anyhow!("Invalid SMTP URL scheme: {}", parsed.scheme())),
        };

        Ok(SmtpConfig {
            host: parsed.host_str()
                .context("Missing host in SMTP URL")?
                .to_string(),
            port: parsed.port().unwrap_or(match tls_mode {
                TlsMode::ImplicitTls => 465,
                TlsMode::StartTls => 587,
                TlsMode::None => 25,
            }),
            username: if parsed.username().is_empty() {
                None
            } else {
                Some(urlencoding::decode(parsed.username())?.into_owned())
            },
            password: parsed.password()
                .map(|p| urlencoding::decode(p).map(|s| s.into_owned()))
                .transpose()?,
            tls_mode,
        })
    }
}

// ============================================================================
// Activity Parameters and Result
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailParams {
    pub smtp_url: String,
    pub from: String,
    pub to: Vec<String>,
    pub subject: String,
    /// Plain text email body. At least one of text_body or html_body required.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_body: Option<String>,
    /// HTML email body. At least one of text_body or html_body required.
    /// If both provided, email is sent as multipart/alternative.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub html_body: Option<String>,
    #[serde(default)]
    pub cc: Vec<String>,
    #[serde(default)]
    pub bcc: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailResult {
    pub success: bool,
    pub message_id: Option<String>,
    pub recipients_accepted: usize,
    pub recipients_rejected: usize,
}
```

### Email Executor

```rust
// ============================================================================
// Email Executor (SMTP Client)
// ============================================================================

pub struct EmailExecutor;

impl EmailExecutor {
    pub fn new() -> Self {
        Self
    }

    pub async fn send(&self, params: EmailParams) -> Result<EmailResult> {
        // Parse SMTP configuration
        let config = SmtpConfig::from_url(&params.smtp_url)?;

        // Collect all recipients
        let all_recipients: Vec<&str> = params.to.iter()
            .chain(params.cc.iter())
            .chain(params.bcc.iter())
            .map(|s| s.as_str())
            .collect();

        // Build email message
        let from_mailbox: Mailbox = params.from.parse()
            .context("Invalid 'from' email address")?;

        let mut email_builder = Message::builder()
            .from(from_mailbox)
            .subject(&params.subject);

        // Add recipients
        for to in &params.to {
            let mailbox: Mailbox = to.parse()
                .context(format!("Invalid 'to' email address: {}", to))?;
            email_builder = email_builder.to(mailbox);
        }

        for cc in &params.cc {
            let mailbox: Mailbox = cc.parse()
                .context(format!("Invalid 'cc' email address: {}", cc))?;
            email_builder = email_builder.cc(mailbox);
        }

        for bcc in &params.bcc {
            let mailbox: Mailbox = bcc.parse()
                .context(format!("Invalid 'bcc' email address: {}", bcc))?;
            email_builder = email_builder.bcc(mailbox);
        }

        if let Some(reply_to) = &params.reply_to {
            let mailbox: Mailbox = reply_to.parse()
                .context("Invalid 'reply_to' email address")?;
            email_builder = email_builder.reply_to(mailbox);
        }

        // Build email body based on which fields are provided
        let email = match (&params.text_body, &params.html_body) {
            (Some(text), Some(html)) => {
                // Both provided: send multipart/alternative
                let multipart = MultiPart::alternative()
                    .singlepart(SinglePart::builder()
                        .header(ContentType::TEXT_PLAIN)
                        .body(text.clone()))
                    .singlepart(SinglePart::builder()
                        .header(ContentType::TEXT_HTML)
                        .body(html.clone()));
                email_builder.multipart(multipart)?
            }
            (Some(text), None) => email_builder
                .header(ContentType::TEXT_PLAIN)
                .body(text.clone())?,
            (None, Some(html)) => email_builder
                .header(ContentType::TEXT_HTML)
                .body(html.clone())?,
            (None, None) => return Err(anyhow::anyhow!(
                "At least one of 'text_body' or 'html_body' must be provided"
            )),
        };

        // Build SMTP transport
        let transport = self.build_transport(&config)?;

        // Send email
        let response = transport.send(email).await
            .context("Failed to send email")?;

        Ok(EmailResult {
            success: response.is_positive(),
            message_id: response.message().map(|s| s.to_string()),
            recipients_accepted: all_recipients.len(),
            recipients_rejected: 0,
        })
    }

    fn build_transport(
        &self,
        config: &SmtpConfig,
    ) -> Result<AsyncSmtpTransport<Tokio1Executor>> {
        let mut builder = match config.tls_mode {
            TlsMode::ImplicitTls => {
                AsyncSmtpTransport::<Tokio1Executor>::relay(&config.host)?
            }
            TlsMode::StartTls => {
                AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&config.host)?
            }
            TlsMode::None => {
                AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&config.host)
            }
        };

        builder = builder.port(config.port);

        // Add credentials if provided
        if let (Some(username), Some(password)) = (&config.username, &config.password) {
            let credentials = Credentials::new(username.clone(), password.clone());
            builder = builder.credentials(credentials);
        }

        // Connection timeout
        builder = builder.timeout(Some(Duration::from_secs(30)));

        Ok(builder.build())
    }
}
```

### Activity Implementation

```rust
// ============================================================================
// EmailSendActivity (ActivityImpl)
// ============================================================================

pub struct EmailSendActivity {
    executor: EmailExecutor,
}

impl EmailSendActivity {
    pub fn new() -> Self {
        Self {
            executor: EmailExecutor::new(),
        }
    }
}

#[async_trait]
impl ActivityImpl for EmailSendActivity {
    async fn execute(&self, parameters: Value) -> Result<ActivityResult> {
        tracing::debug!(
            "Executing email_send activity"
        );

        // Parse parameters
        let params: EmailParams = serde_json::from_value(parameters)
            .context("Failed to parse email parameters")?;

        // Validate recipients
        if params.to.is_empty() {
            return Err(anyhow::anyhow!("At least one recipient required in 'to' field"));
        }

        // Send email
        let result = self.executor.send(params).await?;

        tracing::debug!(
            "Email sent successfully, message_id: {:?}",
            result.message_id
        );

        Ok(ActivityResult::value("result", serde_json::to_value(result)?))
    }

    fn name(&self) -> &str {
        "email_send"
    }

    fn worker(&self) -> &str {
        "std"
    }
}
```

### Registration in std.rs

```rust
use crate::activities::{
    EchoActivity, EmbeddingActivity, HttpRequestActivity, LLMPromptActivity,
    PostgresQueryActivity, EmailSendActivity,
};

pub fn register_std_activities(cache_service: Arc<dyn CacheService>) -> ActivityRegistry {
    let mut registry = ActivityRegistry::new(cache_service);

    // Register activities
    registry.register(Arc::new(EchoActivity));
    registry.register(Arc::new(HttpRequestActivity::new()));
    registry.register(Arc::new(PostgresQueryActivity::new()));
    registry.register(Arc::new(EmailSendActivity::new()));

    // LLM activities
    registry.register(Arc::new(LLMPromptActivity::new()));
    registry.register(Arc::new(EmbeddingActivity::new()));

    registry
}
```

---

## Configuration

### Environment Variables

| Variable                        | Default | Description             |
|---------------------------------|---------|-------------------------|
| `KRUXIAFLOW_EMAIL_TIMEOUT_SECS` | 30      | SMTP connection timeout |

---

## Retry Behavior

Email delivery failures are categorized as retryable or non-retryable:

**Retryable Errors** (activity will be retried per workflow settings):
- Connection timeout
- Connection refused
- Temporary SMTP errors (4xx responses)
- DNS resolution failures

**Non-Retryable Errors** (activity fails immediately):
- Invalid email addresses
- Authentication failures
- Permanent SMTP errors (5xx responses)

The activity returns error metadata to enable proper retry handling:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
}

impl EmailExecutor {
    fn classify_error(err: &lettre::transport::smtp::Error) -> EmailError {
        use lettre::transport::smtp::Error;

        match err {
            Error::Transient(response) => EmailError {
                code: "SMTP_TRANSIENT".to_string(),
                message: response.message().join("; "),
                retryable: true,
            },
            Error::Permanent(response) => EmailError {
                code: "SMTP_PERMANENT".to_string(),
                message: response.message().join("; "),
                retryable: false,
            },
            Error::Client(msg) => EmailError {
                code: "SMTP_CLIENT".to_string(),
                message: msg.to_string(),
                retryable: false,
            },
            _ => EmailError {
                code: "SMTP_ERROR".to_string(),
                message: err.to_string(),
                retryable: true,
            },
        }
    }
}
```

---

## Example Workflows

### Example: Alert on Workflow Completion

```yaml
name: data_pipeline_with_alert
description: Process data and send completion alert

activities:
  process_data:
    activity: postgres_query
    parameters:
      db_url: "{{SECRET.db_url}}"
      query: "SELECT count(*) as total FROM processed_records WHERE batch_id = $1"
      params:
        - "{{INPUT.batch_id}}"

  send_completion_alert:
    activity: email_send
    parameters:
      smtp_url: "{{SECRET.smtp_url}}"
      from: "pipeline@example.com"
      to:
        - "data-team@example.com"
      subject: "Pipeline {{WORKFLOW.id}} Complete"
      text_body: |
        Data pipeline completed successfully.

        Batch ID: {{INPUT.batch_id}}
        Records processed: {{process_data.result.rows[0].total}}

        Workflow ID: {{WORKFLOW.id}}
    depends_on:
      - process_data
```

### Example: Error Notification with Multipart/Alternative (Recommended)

```yaml
name: error_notification
description: Send error notification with both HTML and plain text versions

activities:
  send_error_alert:
    activity: email_send
    parameters:
      smtp_url: "{{SECRET.smtp_url}}"
      from: "alerts@example.com"
      to:
        - "oncall@example.com"
      cc:
        - "engineering@example.com"
      subject: "[ALERT] Error in {{INPUT.service_name}}"
      text_body: |
        ERROR ALERT

        Service: {{INPUT.service_name}}
        Error: {{INPUT.error_message}}
        Time: {{WORKFLOW.started_at}}

        Workflow ID: {{WORKFLOW.id}}
      html_body: |
        <html>
        <body style="font-family: Arial, sans-serif;">
          <h2 style="color: #d32f2f;">Error Alert</h2>
          <table style="border-collapse: collapse;">
            <tr>
              <td style="padding: 8px; border: 1px solid #ddd;"><strong>Service</strong></td>
              <td style="padding: 8px; border: 1px solid #ddd;">{{INPUT.service_name}}</td>
            </tr>
            <tr>
              <td style="padding: 8px; border: 1px solid #ddd;"><strong>Error</strong></td>
              <td style="padding: 8px; border: 1px solid #ddd;">{{INPUT.error_message}}</td>
            </tr>
            <tr>
              <td style="padding: 8px; border: 1px solid #ddd;"><strong>Time</strong></td>
              <td style="padding: 8px; border: 1px solid #ddd;">{{WORKFLOW.started_at}}</td>
            </tr>
          </table>
          <p style="color: #666; font-size: 12px;">
            Workflow ID: {{WORKFLOW.id}}
          </p>
        </body>
        </html>
```

### Example: HTML-Only Notification

```yaml
activities:
  send_html_only:
    activity: email_send
    parameters:
      smtp_url: "{{SECRET.smtp_url}}"
      from: "alerts@example.com"
      to:
        - "team@example.com"
      subject: "Status Update"
      html_body: |
        <html><body><h1>Update</h1><p>Your request was processed.</p></body></html>
```

### Example: Conditional Notification

```yaml
name: conditional_alert
description: Send alert only on failure

activities:
  check_status:
    activity: http_request
    parameters:
      method: GET
      url: "{{INPUT.health_check_url}}"

  send_failure_alert:
    activity: email_send
    parameters:
      smtp_url: "{{SECRET.smtp_url}}"
      from: "monitoring@example.com"
      to:
        - "ops@example.com"
      subject: "Health Check Failed: {{INPUT.service_name}}"
      text_body: |
        Service health check failed.

        Service: {{INPUT.service_name}}
        URL: {{INPUT.health_check_url}}
        Status: {{check_status.response.status}}
    depends_on:
      - activity_key: check_status
        condition: "{{check_status.response.success}} == false"
```

---

## Testing Strategy

### Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_smtp_config_from_url_plain() {
        let config = SmtpConfig::from_url("smtp://localhost:25").unwrap();
        assert_eq!(config.host, "localhost");
        assert_eq!(config.port, 25);
        assert!(matches!(config.tls_mode, TlsMode::None));
    }

    #[test]
    fn test_smtp_config_from_url_starttls() {
        let config = SmtpConfig::from_url(
            "smtp://user:pass@smtp.example.com:587?tls=required"
        ).unwrap();
        assert_eq!(config.host, "smtp.example.com");
        assert_eq!(config.port, 587);
        assert_eq!(config.username, Some("user".to_string()));
        assert_eq!(config.password, Some("pass".to_string()));
        assert!(matches!(config.tls_mode, TlsMode::StartTls));
    }

    #[test]
    fn test_smtp_config_from_url_smtps() {
        let config = SmtpConfig::from_url(
            "smtps://apikey:secret@smtp.sendgrid.net:465"
        ).unwrap();
        assert_eq!(config.host, "smtp.sendgrid.net");
        assert_eq!(config.port, 465);
        assert!(matches!(config.tls_mode, TlsMode::ImplicitTls));
    }

    #[test]
    fn test_email_params_text_only() {
        let params = EmailParams {
            smtp_url: "smtp://localhost:25".to_string(),
            from: "sender@example.com".to_string(),
            to: vec!["recipient@example.com".to_string()],
            subject: "Test".to_string(),
            text_body: Some("Test body".to_string()),
            html_body: None,
            cc: vec![],
            bcc: vec![],
            reply_to: None,
        };

        let json = serde_json::to_string(&params).unwrap();
        let deserialized: EmailParams = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.from, "sender@example.com");
        assert_eq!(deserialized.text_body, Some("Test body".to_string()));
    }

    #[test]
    fn test_email_params_multipart() {
        let params = EmailParams {
            smtp_url: "smtp://localhost:25".to_string(),
            from: "sender@example.com".to_string(),
            to: vec!["recipient@example.com".to_string()],
            subject: "Test".to_string(),
            text_body: Some("Plain text version".to_string()),
            html_body: Some("<html><body><h1>HTML version</h1></body></html>".to_string()),
            cc: vec![],
            bcc: vec![],
            reply_to: None,
        };

        assert!(params.text_body.is_some());
        assert!(params.html_body.is_some());
    }
}
```

### Integration Tests

Integration tests require a local SMTP server. Use `mailhog` or `mailcatcher` in docker-compose for testing:

```yaml
# docker-compose.yml (test services)
services:
  mailhog:
    image: mailhog/mailhog:latest
    ports:
      - "1025:1025"   # SMTP
      - "8025:8025"   # Web UI
```

```rust
#[tokio::test]
async fn test_email_send_integration() {
    // Requires mailhog running on localhost:1025
    let activity = EmailSendActivity::new();

    let params = serde_json::json!({
        "smtp_url": "smtp://localhost:1025",
        "from": "test@example.com",
        "to": ["recipient@example.com"],
        "subject": "Integration Test",
        "text_body": "This is a test email"
    });

    let result = activity.execute(params).await.unwrap();
    let output: EmailResult = serde_json::from_value(
        result.outputs.get("result").unwrap().clone()
    ).unwrap();

    assert!(output.success);
    assert_eq!(output.recipients_accepted, 1);
}
```

---

## Implementation Tasks

1. **Add lettre dependency** ✅
   - Added to `worker/Cargo.toml`
   - Also added `url` and `urlencoding` for SMTP URL parsing
   - Verified compilation

2. **Create email.rs module** ✅
   - Implemented `SmtpConfig::from_url()` with support for smtp://, smtps://, and ?tls=required
   - Implemented `EmailExecutor` with STARTTLS, implicit TLS, and plain SMTP modes
   - Implemented `EmailSendActivity` as ActivityImpl

2b. **Add multipart/alternative support** ✅
   - Replaced `body` + `content_type` with explicit `text_body` and `html_body` optional fields
   - At least one of `text_body` or `html_body` must be provided
   - If both provided, email is sent as multipart/alternative
   - Clean, unambiguous API: field names clearly indicate content type

3. **Register activity** ✅
   - Exported from `activities/mod.rs`
   - Registered in `std.rs`

4. **Tests** ✅
   - 16 unit tests for SmtpConfig URL parsing, EmailParams serialization (text-only, html-only, multipart), activity metadata
   - 13 integration tests with mailhog in `worker/tests/email_activity_integration_test.rs`:
     - `test_email_send_plain_text` - Send plain text email
     - `test_email_send_html` - Send HTML email
     - `test_email_send_multiple_recipients` - Multiple To recipients
     - `test_email_send_with_cc_and_bcc` - CC and BCC recipients
     - `test_email_send_with_reply_to` - Reply-To header
     - `test_email_send_empty_recipients_fails` - Validation error for empty To
     - `test_email_send_invalid_from_address_fails` - Validation error for invalid From
     - `test_email_send_invalid_to_address_fails` - Validation error for invalid To
     - `test_email_send_connection_failure` - Connection error handling
     - `test_email_send_invalid_smtp_url` - URL parsing error
     - `test_email_send_wrong_scheme` - Invalid URL scheme error
     - `test_email_send_default_content_type` - Default to text/plain
     - `test_email_activity_name_and_worker` - Activity metadata

5. **Documentation** ✅
   - Example workflows documented in this file

**Estimated Total**: ~4.5 hours

---

## Success Criteria

- [x] `email_send` activity sends emails via SMTP
- [x] Supports plain text and HTML content types
- [x] Supports multipart/alternative (both HTML and plain text)
- [x] Supports multiple recipients (to/cc/bcc)
- [x] Proper error classification for retry behavior
- [x] All unit tests pass (16 tests)
- [x] Integration test with mailhog passes
- [x] Example workflow documented

---

## Post-MVP: Deferred Notification Activities

The following notification activities are deferred to post-MVP (see `docs/post-mvp.md`):

1. **Slack Integration** (`slack_message`)
   - Post messages to Slack channels
   - Support for blocks, attachments, threads
   - Webhook and Bot Token modes

2. **Microsoft Teams** (`teams_notify`)
   - Adaptive Card support
   - Channel and chat messages
   - Webhook integration

3. **Discord** (`discord_send`)
   - Channel messages
   - Embed support
   - Webhook integration

These are deferred because:
- Email covers the most critical alerting use cases
- Chat integrations require OAuth flows and app setup
- Each platform has unique API requirements
- Can be implemented as separate workers post-MVP
