//! Integration tests for the email_send activity
//!
//! These tests require the mailhog SMTP server to be running locally.
//! Start it with: `docker compose up mailhog`
//!
//! Mailhog captures all emails and provides:
//! - SMTP server on port 1025
//! - Web UI on port 8025
//! - REST API on port 8025 for verification

use kruxiaflow_worker::ActivityImpl;
use kruxiaflow_worker::EmailSendActivity;
use serde::Deserialize;
use serde_json::json;
use serial_test::serial;

/// Mailhog SMTP URL for testing
fn mailhog_smtp_url() -> String {
    std::env::var("MAILHOG_SMTP_URL").unwrap_or_else(|_| "smtp://127.0.0.1:1025".to_string())
}

/// Mailhog API URL for verification
fn mailhog_api_url() -> String {
    std::env::var("MAILHOG_API_URL").unwrap_or_else(|_| "http://127.0.0.1:8025".to_string())
}

/// Check if mailhog is available
async fn mailhog_available() -> bool {
    let client = reqwest::Client::new();
    client
        .get(format!("{}/api/v2/messages", mailhog_api_url()))
        .send()
        .await
        .is_ok()
}

/// Clear all messages from mailhog
async fn clear_mailhog() {
    let client = reqwest::Client::new();
    let _ = client
        .delete(format!("{}/api/v1/messages", mailhog_api_url()))
        .send()
        .await;
}

/// Mailhog message response structure
#[derive(Debug, Deserialize)]
struct MailhogMessages {
    items: Vec<MailhogMessage>,
}

#[derive(Debug, Deserialize)]
struct MailhogMessage {
    #[serde(rename = "Content")]
    content: MailhogContent,
    #[serde(rename = "Raw")]
    raw: MailhogRaw,
}

#[derive(Debug, Deserialize)]
struct MailhogContent {
    #[serde(rename = "Headers")]
    headers: MailhogHeaders,
    #[serde(rename = "Body")]
    body: String,
}

#[derive(Debug, Deserialize)]
struct MailhogHeaders {
    #[serde(rename = "Subject")]
    subject: Vec<String>,
    #[serde(rename = "From")]
    from: Vec<String>,
    #[serde(rename = "To")]
    to: Vec<String>,
    #[serde(rename = "Content-Type")]
    content_type: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct MailhogRaw {
    #[allow(dead_code)]
    #[serde(rename = "From")]
    from: String,
    #[serde(rename = "To")]
    to: Vec<String>,
}

/// Get all messages from mailhog
async fn get_mailhog_messages() -> Option<MailhogMessages> {
    let client = reqwest::Client::new();
    let response = client
        .get(format!("{}/api/v2/messages", mailhog_api_url()))
        .send()
        .await
        .ok()?;

    response.json::<MailhogMessages>().await.ok()
}

/// Wait for a message to arrive in mailhog (with timeout)
async fn wait_for_message(timeout_ms: u64) -> Option<MailhogMessage> {
    let start = std::time::Instant::now();
    while start.elapsed().as_millis() < timeout_ms as u128 {
        if let Some(messages) = get_mailhog_messages().await {
            if !messages.items.is_empty() {
                return Some(messages.items.into_iter().next().unwrap());
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    None
}

#[tokio::test]
#[serial]
async fn test_email_send_plain_text() {
    if !mailhog_available().await {
        eprintln!("Skipping test: mailhog not available. Run: docker compose up mailhog");
        return;
    }

    clear_mailhog().await;

    let activity = EmailSendActivity::new();

    let params = json!({
        "smtp_url": mailhog_smtp_url(),
        "from": "sender@example.com",
        "to": ["recipient@example.com"],
        "subject": "Test Plain Text Email",
        "text_body": "This is a plain text email body."
    });

    let result = activity.execute(params).await.unwrap();

    let output_value = result.to_json_value();
    let email_result = output_value.get("result").unwrap();
    assert_eq!(email_result.get("success").unwrap(), true);
    assert_eq!(email_result.get("recipients_accepted").unwrap(), 1);
    assert_eq!(email_result.get("recipients_rejected").unwrap(), 0);

    // Verify email was received by mailhog
    let message = wait_for_message(5000).await.expect("Email not received");
    assert_eq!(message.content.headers.subject[0], "Test Plain Text Email");
    assert!(message.content.headers.from[0].contains("sender@example.com"));
    assert!(message.content.headers.to[0].contains("recipient@example.com"));
    assert!(
        message
            .content
            .body
            .contains("This is a plain text email body")
    );
}

#[tokio::test]
#[serial]
async fn test_email_send_html() {
    if !mailhog_available().await {
        eprintln!("Skipping test: mailhog not available. Run: docker compose up mailhog");
        return;
    }

    clear_mailhog().await;

    let activity = EmailSendActivity::new();

    let params = json!({
        "smtp_url": mailhog_smtp_url(),
        "from": "sender@example.com",
        "to": ["recipient@example.com"],
        "subject": "Test HTML Email",
        "html_body": "<html><body><h1>Hello</h1><p>This is an HTML email.</p></body></html>"
    });

    let result = activity.execute(params).await.unwrap();

    let output_value = result.to_json_value();
    let email_result = output_value.get("result").unwrap();
    assert_eq!(email_result.get("success").unwrap(), true);

    // Verify email was received by mailhog
    let message = wait_for_message(5000).await.expect("Email not received");
    assert_eq!(message.content.headers.subject[0], "Test HTML Email");

    // Check content type header
    if let Some(content_types) = &message.content.headers.content_type {
        assert!(content_types[0].contains("text/html"));
    }
    assert!(message.content.body.contains("<h1>Hello</h1>"));
}

#[tokio::test]
#[serial]
async fn test_email_send_multiple_recipients() {
    if !mailhog_available().await {
        eprintln!("Skipping test: mailhog not available. Run: docker compose up mailhog");
        return;
    }

    clear_mailhog().await;

    let activity = EmailSendActivity::new();

    let params = json!({
        "smtp_url": mailhog_smtp_url(),
        "from": "sender@example.com",
        "to": ["recipient1@example.com", "recipient2@example.com"],
        "subject": "Test Multiple Recipients",
        "text_body": "Email to multiple recipients."
    });

    let result = activity.execute(params).await.unwrap();

    let output_value = result.to_json_value();
    let email_result = output_value.get("result").unwrap();
    assert_eq!(email_result.get("success").unwrap(), true);
    assert_eq!(email_result.get("recipients_accepted").unwrap(), 2);

    // Verify email was received
    let message = wait_for_message(5000).await.expect("Email not received");
    assert_eq!(
        message.content.headers.subject[0],
        "Test Multiple Recipients"
    );
    assert_eq!(message.raw.to.len(), 2);
}

#[tokio::test]
#[serial]
async fn test_email_send_with_cc_and_bcc() {
    if !mailhog_available().await {
        eprintln!("Skipping test: mailhog not available. Run: docker compose up mailhog");
        return;
    }

    clear_mailhog().await;

    let activity = EmailSendActivity::new();

    let params = json!({
        "smtp_url": mailhog_smtp_url(),
        "from": "sender@example.com",
        "to": ["recipient@example.com"],
        "cc": ["cc@example.com"],
        "bcc": ["bcc@example.com"],
        "subject": "Test CC and BCC",
        "text_body": "Email with CC and BCC recipients."
    });

    let result = activity.execute(params).await.unwrap();

    let output_value = result.to_json_value();
    let email_result = output_value.get("result").unwrap();
    assert_eq!(email_result.get("success").unwrap(), true);
    // 3 recipients total: to + cc + bcc
    assert_eq!(email_result.get("recipients_accepted").unwrap(), 3);

    // Verify email was received
    let message = wait_for_message(5000).await.expect("Email not received");
    assert_eq!(message.content.headers.subject[0], "Test CC and BCC");
    // Raw recipients include all recipients (to + cc + bcc)
    assert_eq!(message.raw.to.len(), 3);
}

#[tokio::test]
#[serial]
async fn test_email_send_with_reply_to() {
    if !mailhog_available().await {
        eprintln!("Skipping test: mailhog not available. Run: docker compose up mailhog");
        return;
    }

    clear_mailhog().await;

    let activity = EmailSendActivity::new();

    let params = json!({
        "smtp_url": mailhog_smtp_url(),
        "from": "sender@example.com",
        "to": ["recipient@example.com"],
        "reply_to": "reply@example.com",
        "subject": "Test Reply-To",
        "text_body": "Email with Reply-To header."
    });

    let result = activity.execute(params).await.unwrap();

    let output_value = result.to_json_value();
    let email_result = output_value.get("result").unwrap();
    assert_eq!(email_result.get("success").unwrap(), true);

    // Verify email was received
    let message = wait_for_message(5000).await.expect("Email not received");
    assert_eq!(message.content.headers.subject[0], "Test Reply-To");
}

#[tokio::test]
#[serial]
async fn test_email_send_empty_recipients_fails() {
    let activity = EmailSendActivity::new();

    let params = json!({
        "smtp_url": mailhog_smtp_url(),
        "from": "sender@example.com",
        "to": [],
        "subject": "Test Empty Recipients",
        "text_body": "This should fail."
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
#[serial]
async fn test_email_send_invalid_from_address_fails() {
    let activity = EmailSendActivity::new();

    let params = json!({
        "smtp_url": mailhog_smtp_url(),
        "from": "not-an-email",
        "to": ["recipient@example.com"],
        "subject": "Test Invalid From",
        "text_body": "This should fail."
    });

    let result = activity.execute(params).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("from"));
}

#[tokio::test]
#[serial]
async fn test_email_send_invalid_to_address_fails() {
    let activity = EmailSendActivity::new();

    let params = json!({
        "smtp_url": mailhog_smtp_url(),
        "from": "sender@example.com",
        "to": ["not-an-email"],
        "subject": "Test Invalid To",
        "text_body": "This should fail."
    });

    let result = activity.execute(params).await;
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    // Error should mention the invalid email address
    assert!(
        err_msg.contains("email")
            || err_msg.contains("address")
            || err_msg.contains("not-an-email"),
        "Error message should mention invalid email: {}",
        err_msg
    );
}

#[tokio::test]
#[serial]
async fn test_email_send_connection_failure() {
    let activity = EmailSendActivity::new();

    // Try to connect to a non-existent SMTP server
    let params = json!({
        "smtp_url": "smtp://127.0.0.1:65535",
        "from": "sender@example.com",
        "to": ["recipient@example.com"],
        "subject": "Test Connection Failure",
        "text_body": "This should fail due to connection error."
    });

    let result = activity.execute(params).await;
    assert!(result.is_err());
}

#[tokio::test]
#[serial]
async fn test_email_send_invalid_smtp_url() {
    let activity = EmailSendActivity::new();

    let params = json!({
        "smtp_url": "not-a-url",
        "from": "sender@example.com",
        "to": ["recipient@example.com"],
        "subject": "Test Invalid URL",
        "text_body": "This should fail."
    });

    let result = activity.execute(params).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("Invalid SMTP URL"));
}

#[tokio::test]
#[serial]
async fn test_email_send_wrong_scheme() {
    let activity = EmailSendActivity::new();

    let params = json!({
        "smtp_url": "http://localhost:1025",
        "from": "sender@example.com",
        "to": ["recipient@example.com"],
        "subject": "Test Wrong Scheme",
        "text_body": "This should fail."
    });

    let result = activity.execute(params).await;
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Invalid SMTP URL scheme")
    );
}

#[tokio::test]
#[serial]
async fn test_email_send_default_content_type() {
    if !mailhog_available().await {
        eprintln!("Skipping test: mailhog not available. Run: docker compose up mailhog");
        return;
    }

    clear_mailhog().await;

    let activity = EmailSendActivity::new();

    // Use text_body - should be sent as text/plain
    let params = json!({
        "smtp_url": mailhog_smtp_url(),
        "from": "sender@example.com",
        "to": ["recipient@example.com"],
        "subject": "Test Default Content Type",
        "text_body": "This should be plain text by default."
    });

    let result = activity.execute(params).await.unwrap();

    let output_value = result.to_json_value();
    let email_result = output_value.get("result").unwrap();
    assert_eq!(email_result.get("success").unwrap(), true);

    // Verify email was received
    let message = wait_for_message(5000).await.expect("Email not received");
    assert_eq!(
        message.content.headers.subject[0],
        "Test Default Content Type"
    );

    // Check content type defaults to text/plain
    if let Some(content_types) = &message.content.headers.content_type {
        assert!(content_types[0].contains("text/plain"));
    }
}

#[tokio::test]
#[serial]
async fn test_email_activity_name_and_worker() {
    let activity = EmailSendActivity::new();

    assert_eq!(activity.name(), "email_send");
    assert_eq!(activity.worker(), "builtin");
}
