use crate::activity_result::ActivityResult;
use crate::registry::ActivityImpl;
use anyhow::{Context, Result};
use async_trait::async_trait;
use lettre::{
    AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor,
    message::{Mailbox, MultiPart, SinglePart, header::ContentType},
    transport::smtp::authentication::Credentials,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::Duration;

// ============================================================================
// SMTP Configuration
// ============================================================================

/// TLS mode for SMTP connections
#[derive(Debug, Clone, Default)]
pub enum TlsMode {
    /// Plain SMTP (insecure - development only)
    None,
    /// STARTTLS upgrade (port 587, recommended)
    #[default]
    StartTls,
    /// SMTPS/implicit TLS (port 465)
    ImplicitTls,
}

/// Parsed SMTP connection configuration
#[derive(Debug, Clone)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<String>,
    pub tls_mode: TlsMode,
}

impl SmtpConfig {
    /// Parse SMTP URL into configuration
    ///
    /// Supported formats:
    /// - `smtp://host:port` (plain, insecure)
    /// - `smtp://user:pass@host:port?tls=required` (STARTTLS)
    /// - `smtps://user:pass@host:port` (implicit TLS)
    ///
    /// # Examples
    /// ```
    /// # use kruxiaflow_worker::activities::email::SmtpConfig;
    /// let config = SmtpConfig::from_url("smtp://localhost:25").unwrap();
    /// assert_eq!(config.host, "localhost");
    /// assert_eq!(config.port, 25);
    /// ```
    pub fn from_url(url: &str) -> Result<Self> {
        let parsed = url::Url::parse(url).context("Invalid SMTP URL")?;

        let tls_mode = match parsed.scheme() {
            "smtps" => TlsMode::ImplicitTls,
            "smtp" => {
                // Check for ?tls=required query param
                let tls_param = parsed
                    .query_pairs()
                    .find(|(k, _)| k == "tls")
                    .map(|(_, v)| v.to_string());
                match tls_param.as_deref() {
                    Some("required") | Some("true") => TlsMode::StartTls,
                    _ => TlsMode::None,
                }
            }
            scheme => return Err(anyhow::anyhow!("Invalid SMTP URL scheme: {}", scheme)),
        };

        let port = parsed.port().unwrap_or(match tls_mode {
            TlsMode::ImplicitTls => 465,
            TlsMode::StartTls => 587,
            TlsMode::None => 25,
        });

        let username = if parsed.username().is_empty() {
            None
        } else {
            Some(
                urlencoding::decode(parsed.username())
                    .context("Invalid URL encoding in username")?
                    .into_owned(),
            )
        };

        let password = parsed
            .password()
            .map(|p| urlencoding::decode(p).map(|s| s.into_owned()))
            .transpose()
            .context("Invalid URL encoding in password")?;

        Ok(SmtpConfig {
            host: parsed
                .host_str()
                .context("Missing host in SMTP URL")?
                .to_string(),
            port,
            username,
            password,
            tls_mode,
        })
    }
}

// ============================================================================
// Activity Parameters and Result
// ============================================================================

/// Email send activity parameters
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailParams {
    /// SMTP connection URL (see SmtpConfig::from_url for format)
    pub smtp_url: String,
    /// Sender email address
    pub from: String,
    /// Recipient email addresses
    pub to: Vec<String>,
    /// Email subject line
    pub subject: String,
    /// Plain text email body. At least one of text_body or html_body must be provided.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_body: Option<String>,
    /// HTML email body. At least one of text_body or html_body must be provided.
    /// If both are provided, the email is sent as multipart/alternative.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub html_body: Option<String>,
    /// CC recipients
    #[serde(default)]
    pub cc: Vec<String>,
    /// BCC recipients
    #[serde(default)]
    pub bcc: Vec<String>,
    /// Reply-to address
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<String>,
}

/// Email send result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailResult {
    /// Whether the email was sent successfully
    pub success: bool,
    /// SMTP message ID (if available)
    pub message_id: Option<String>,
    /// Number of recipients that accepted the email
    pub recipients_accepted: usize,
    /// Number of recipients that rejected the email
    pub recipients_rejected: usize,
}

// ============================================================================
// Email Executor (SMTP Client)
// ============================================================================

/// Email executor that handles SMTP communication
pub struct EmailExecutor;

impl EmailExecutor {
    /// Create a new email executor
    pub fn new() -> Self {
        Self
    }

    /// Send an email
    pub async fn send(&self, params: EmailParams) -> Result<EmailResult> {
        // Parse SMTP configuration
        let config = SmtpConfig::from_url(&params.smtp_url)?;

        // Collect all recipients
        let all_recipients: Vec<&str> = params
            .to
            .iter()
            .chain(params.cc.iter())
            .chain(params.bcc.iter())
            .map(|s| s.as_str())
            .collect();

        // Build email message
        let from_mailbox: Mailbox = params
            .from
            .parse()
            .context("Invalid 'from' email address")?;

        let mut email_builder = Message::builder()
            .from(from_mailbox)
            .subject(&params.subject);

        // Add recipients
        for to in &params.to {
            let mailbox: Mailbox = to
                .parse()
                .context(format!("Invalid 'to' email address: {}", to))?;
            email_builder = email_builder.to(mailbox);
        }

        for cc in &params.cc {
            let mailbox: Mailbox = cc
                .parse()
                .context(format!("Invalid 'cc' email address: {}", cc))?;
            email_builder = email_builder.cc(mailbox);
        }

        for bcc in &params.bcc {
            let mailbox: Mailbox = bcc
                .parse()
                .context(format!("Invalid 'bcc' email address: {}", bcc))?;
            email_builder = email_builder.bcc(mailbox);
        }

        if let Some(reply_to) = &params.reply_to {
            let mailbox: Mailbox = reply_to
                .parse()
                .context("Invalid 'reply_to' email address")?;
            email_builder = email_builder.reply_to(mailbox);
        }

        // Build email body based on which fields are provided
        let email = match (&params.text_body, &params.html_body) {
            (Some(text), Some(html)) => {
                // Both provided: send multipart/alternative
                let multipart = MultiPart::alternative()
                    .singlepart(
                        SinglePart::builder()
                            .header(ContentType::TEXT_PLAIN)
                            .body(text.clone()),
                    )
                    .singlepart(
                        SinglePart::builder()
                            .header(ContentType::TEXT_HTML)
                            .body(html.clone()),
                    );
                email_builder.multipart(multipart)?
            }
            (Some(text), None) => {
                // Plain text only
                email_builder
                    .header(ContentType::TEXT_PLAIN)
                    .body(text.clone())?
            }
            (None, Some(html)) => {
                // HTML only
                email_builder
                    .header(ContentType::TEXT_HTML)
                    .body(html.clone())?
            }
            (None, None) => {
                return Err(anyhow::anyhow!(
                    "At least one of 'text_body' or 'html_body' must be provided"
                ));
            }
        };

        // Build SMTP transport
        let transport = self.build_transport(&config)?;

        // Send email
        let response = transport
            .send(email)
            .await
            .context("Failed to send email")?;

        // Collect response messages (first line typically contains message ID)
        let message_lines: Vec<&str> = response.message().collect();
        let message_id = message_lines.first().map(|s| s.to_string());

        Ok(EmailResult {
            success: response.is_positive(),
            message_id,
            recipients_accepted: all_recipients.len(),
            recipients_rejected: 0,
        })
    }

    fn build_transport(&self, config: &SmtpConfig) -> Result<AsyncSmtpTransport<Tokio1Executor>> {
        let mut builder = match config.tls_mode {
            TlsMode::ImplicitTls => AsyncSmtpTransport::<Tokio1Executor>::relay(&config.host)
                .context("Failed to create TLS relay")?,
            TlsMode::StartTls => AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&config.host)
                .context("Failed to create STARTTLS relay")?,
            TlsMode::None => AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&config.host),
        };

        builder = builder.port(config.port);

        // Add credentials if provided
        if let (Some(username), Some(password)) = (&config.username, &config.password) {
            let credentials = Credentials::new(username.clone(), password.clone());
            builder = builder.credentials(credentials);
        }

        // Connection timeout (from env or default 30s)
        let timeout_secs = std::env::var("KRUXIAFLOW_EMAIL_TIMEOUT_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(30);
        builder = builder.timeout(Some(Duration::from_secs(timeout_secs)));

        Ok(builder.build())
    }
}

impl Default for EmailExecutor {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// EmailSendActivity (ActivityImpl)
// ============================================================================

/// Email send activity for the built-in worker
///
/// Sends emails via SMTP with support for:
/// - Plain text and HTML content
/// - Multiple recipients (to/cc/bcc)
/// - STARTTLS and implicit TLS
pub struct EmailSendActivity {
    executor: EmailExecutor,
}

impl EmailSendActivity {
    /// Create a new email send activity
    pub fn new() -> Self {
        Self {
            executor: EmailExecutor::new(),
        }
    }
}

impl Default for EmailSendActivity {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ActivityImpl for EmailSendActivity {
    async fn execute(&self, parameters: Value) -> Result<ActivityResult> {
        tracing::debug!("Executing email_send activity");

        // Parse parameters
        let params: EmailParams =
            serde_json::from_value(parameters).context("Failed to parse email parameters")?;

        // Validate recipients
        if params.to.is_empty() {
            return Err(anyhow::anyhow!(
                "At least one recipient required in 'to' field"
            ));
        }

        // Send email
        let result = self.executor.send(params).await?;

        tracing::debug!(
            "Email sent successfully, message_id: {:?}",
            result.message_id
        );

        Ok(ActivityResult::value("result", json!(result)))
    }

    fn name(&self) -> &str {
        "email_send"
    }

    fn worker(&self) -> &str {
        "std"
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_smtp_config_from_url_plain() {
        let config = SmtpConfig::from_url("smtp://localhost:25").unwrap();
        assert_eq!(config.host, "localhost");
        assert_eq!(config.port, 25);
        assert!(matches!(config.tls_mode, TlsMode::None));
        assert!(config.username.is_none());
        assert!(config.password.is_none());
    }

    #[test]
    fn test_smtp_config_from_url_plain_default_port() {
        let config = SmtpConfig::from_url("smtp://localhost").unwrap();
        assert_eq!(config.host, "localhost");
        assert_eq!(config.port, 25);
    }

    #[test]
    fn test_smtp_config_from_url_starttls() {
        let config =
            SmtpConfig::from_url("smtp://user:pass@smtp.example.com:587?tls=required").unwrap();
        assert_eq!(config.host, "smtp.example.com");
        assert_eq!(config.port, 587);
        assert_eq!(config.username, Some("user".to_string()));
        assert_eq!(config.password, Some("pass".to_string()));
        assert!(matches!(config.tls_mode, TlsMode::StartTls));
    }

    #[test]
    fn test_smtp_config_from_url_starttls_default_port() {
        let config =
            SmtpConfig::from_url("smtp://user:pass@smtp.example.com?tls=required").unwrap();
        assert_eq!(config.port, 587);
    }

    #[test]
    fn test_smtp_config_from_url_smtps() {
        let config = SmtpConfig::from_url("smtps://apikey:secret@smtp.sendgrid.net:465").unwrap();
        assert_eq!(config.host, "smtp.sendgrid.net");
        assert_eq!(config.port, 465);
        assert_eq!(config.username, Some("apikey".to_string()));
        assert_eq!(config.password, Some("secret".to_string()));
        assert!(matches!(config.tls_mode, TlsMode::ImplicitTls));
    }

    #[test]
    fn test_smtp_config_from_url_smtps_default_port() {
        let config = SmtpConfig::from_url("smtps://user:pass@smtp.gmail.com").unwrap();
        assert_eq!(config.port, 465);
    }

    #[test]
    fn test_smtp_config_from_url_url_encoded_credentials() {
        // Test URL-encoded special characters in username/password
        let config =
            SmtpConfig::from_url("smtp://user%40domain:pass%3Dword@smtp.example.com").unwrap();
        assert_eq!(config.username, Some("user@domain".to_string()));
        assert_eq!(config.password, Some("pass=word".to_string()));
    }

    #[test]
    fn test_smtp_config_from_url_invalid_scheme() {
        let result = SmtpConfig::from_url("http://localhost:25");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Invalid SMTP URL scheme")
        );
    }

    #[test]
    fn test_smtp_config_from_url_invalid_url() {
        let result = SmtpConfig::from_url("not a url");
        assert!(result.is_err());
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
        assert_eq!(deserialized.to.len(), 1);
        assert_eq!(deserialized.text_body, Some("Test body".to_string()));
        assert!(deserialized.html_body.is_none());
    }

    #[test]
    fn test_email_params_html_only() {
        let params = EmailParams {
            smtp_url: "smtp://localhost:25".to_string(),
            from: "sender@example.com".to_string(),
            to: vec!["recipient@example.com".to_string()],
            subject: "Test".to_string(),
            text_body: None,
            html_body: Some("<html><body><h1>HTML only</h1></body></html>".to_string()),
            cc: vec![],
            bcc: vec![],
            reply_to: None,
        };

        let json = serde_json::to_string(&params).unwrap();
        let deserialized: EmailParams = serde_json::from_str(&json).unwrap();

        assert!(deserialized.text_body.is_none());
        assert_eq!(
            deserialized.html_body,
            Some("<html><body><h1>HTML only</h1></body></html>".to_string())
        );
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

        let json = serde_json::to_string(&params).unwrap();
        let deserialized: EmailParams = serde_json::from_str(&json).unwrap();

        assert_eq!(
            deserialized.text_body,
            Some("Plain text version".to_string())
        );
        assert_eq!(
            deserialized.html_body,
            Some("<html><body><h1>HTML version</h1></body></html>".to_string())
        );
    }

    #[test]
    fn test_email_params_from_json() {
        let json = r#"{
            "smtp_url": "smtp://localhost:25",
            "from": "sender@example.com",
            "to": ["recipient@example.com"],
            "subject": "Test",
            "text_body": "Plain text",
            "html_body": "<html><body>HTML</body></html>"
        }"#;

        let params: EmailParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.text_body, Some("Plain text".to_string()));
        assert_eq!(
            params.html_body,
            Some("<html><body>HTML</body></html>".to_string())
        );
    }

    #[test]
    fn test_email_params_text_only_from_json() {
        let json = r#"{
            "smtp_url": "smtp://localhost:25",
            "from": "sender@example.com",
            "to": ["recipient@example.com"],
            "subject": "Test",
            "text_body": "Test body"
        }"#;

        let params: EmailParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.text_body, Some("Test body".to_string()));
        assert!(params.html_body.is_none());
    }

    #[test]
    fn test_email_result_serialization() {
        let result = EmailResult {
            success: true,
            message_id: Some("abc123@example.com".to_string()),
            recipients_accepted: 2,
            recipients_rejected: 0,
        };

        let json = serde_json::to_string(&result).unwrap();
        let deserialized: EmailResult = serde_json::from_str(&json).unwrap();

        assert!(deserialized.success);
        assert_eq!(
            deserialized.message_id,
            Some("abc123@example.com".to_string())
        );
        assert_eq!(deserialized.recipients_accepted, 2);
    }

    #[test]
    fn test_email_send_activity_name_and_worker() {
        let activity = EmailSendActivity::new();

        assert_eq!(activity.name(), "email_send");
        assert_eq!(activity.worker(), "std");
    }

    #[test]
    fn test_email_send_activity_default() {
        let activity = EmailSendActivity::default();
        assert_eq!(activity.name(), "email_send");
    }

    #[test]
    fn test_email_executor_default() {
        let executor = EmailExecutor;
        // Just verify it can be created
        let _ = executor;
    }

    #[tokio::test]
    async fn test_email_send_activity_empty_recipients() {
        use crate::registry::ActivityImpl;
        let activity = EmailSendActivity::new();

        let params = serde_json::json!({
            "smtp_url": "smtp://localhost:25",
            "from": "sender@example.com",
            "to": [],
            "subject": "Test",
            "text_body": "Hello"
        });

        let result = activity.execute(params).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("At least one recipient")
        );
    }

    #[tokio::test]
    async fn test_email_send_activity_invalid_params() {
        use crate::registry::ActivityImpl;
        let activity = EmailSendActivity::new();

        let result = activity.execute(serde_json::json!({"bad": "params"})).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Failed to parse email parameters")
        );
    }

    #[test]
    fn test_email_params_with_cc_bcc_reply_to() {
        let json = r#"{
            "smtp_url": "smtp://localhost:25",
            "from": "sender@example.com",
            "to": ["recipient@example.com"],
            "subject": "Test",
            "text_body": "Hello",
            "cc": ["cc1@example.com", "cc2@example.com"],
            "bcc": ["bcc@example.com"],
            "reply_to": "reply@example.com"
        }"#;

        let params: EmailParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.cc.len(), 2);
        assert_eq!(params.bcc.len(), 1);
        assert_eq!(params.reply_to, Some("reply@example.com".to_string()));
    }

    #[test]
    fn test_email_params_defaults_for_optional_lists() {
        let json = r#"{
            "smtp_url": "smtp://localhost:25",
            "from": "sender@example.com",
            "to": ["recipient@example.com"],
            "subject": "Test",
            "text_body": "Hello"
        }"#;

        let params: EmailParams = serde_json::from_str(json).unwrap();
        assert!(params.cc.is_empty());
        assert!(params.bcc.is_empty());
        assert!(params.reply_to.is_none());
    }

    #[test]
    fn test_smtp_config_from_url_tls_true() {
        let config =
            SmtpConfig::from_url("smtp://user:pass@smtp.example.com:587?tls=true").unwrap();
        assert!(matches!(config.tls_mode, TlsMode::StartTls));
    }

    #[test]
    fn test_tls_mode_default() {
        let mode = TlsMode::default();
        assert!(matches!(mode, TlsMode::StartTls));
    }

    #[test]
    fn test_email_result_failed() {
        let result = EmailResult {
            success: false,
            message_id: None,
            recipients_accepted: 0,
            recipients_rejected: 1,
        };

        let json = serde_json::to_string(&result).unwrap();
        let deserialized: EmailResult = serde_json::from_str(&json).unwrap();

        assert!(!deserialized.success);
        assert!(deserialized.message_id.is_none());
        assert_eq!(deserialized.recipients_rejected, 1);
    }
}
