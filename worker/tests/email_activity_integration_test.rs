//! Integration tests for the email_send activity
//!
//! These tests require the mailpit SMTP server to be running locally.
//! Start it with: `docker compose up mailpit`
//!
//! Mailpit captures all emails and provides:
//! - SMTP server on port 1025
//! - Web UI on port 8025
//! - REST API on port 8025 for verification

use kruxiaflow_worker::ActivityImpl;
use kruxiaflow_worker::EmailSendActivity;
use serde::Deserialize;
use serde_json::json;
use serial_test::serial;

/// Mailpit SMTP URL for testing
fn mailpit_smtp_url() -> String {
    std::env::var("MAILPIT_SMTP_URL").unwrap_or_else(|_| "smtp://127.0.0.1:1025".to_string())
}

/// Mailpit API URL for verification
fn mailpit_api_url() -> String {
    std::env::var("MAILPIT_API_URL").unwrap_or_else(|_| "http://127.0.0.1:8025".to_string())
}

/// Check if mailpit is available
async fn mailpit_available() -> bool {
    let client = reqwest::Client::new();
    match client
        .get(format!("{}/api/v1/messages", mailpit_api_url()))
        .send()
        .await
    {
        Ok(resp) => resp.status().is_success(),
        Err(_) => false,
    }
}

/// Clear all messages from mailpit
async fn clear_mailpit() {
    let client = reqwest::Client::new();
    let _ = client
        .delete(format!("{}/api/v1/messages", mailpit_api_url()))
        .send()
        .await;
}

/// Mailpit message list response
#[derive(Debug, Deserialize)]
struct MailpitMessages {
    messages: Vec<MailpitMessageSummary>,
}

/// Mailpit message summary (from list endpoint)
#[derive(Debug, Deserialize)]
struct MailpitMessageSummary {
    #[serde(rename = "ID")]
    id: String,
    #[serde(rename = "Subject")]
    subject: String,
    #[serde(rename = "From")]
    from: MailpitAddress,
    #[serde(rename = "To")]
    to: Vec<MailpitAddress>,
    #[allow(dead_code)]
    #[serde(rename = "Snippet")]
    snippet: String,
}

/// Mailpit message detail (from single message endpoint)
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct MailpitMessageDetail {
    #[serde(rename = "Subject")]
    subject: String,
    #[serde(rename = "From")]
    from: MailpitAddress,
    #[serde(rename = "To")]
    to: Vec<MailpitAddress>,
    #[serde(rename = "Cc")]
    cc: Vec<MailpitAddress>,
    #[serde(rename = "Bcc")]
    bcc: Vec<MailpitAddress>,
    #[serde(rename = "Text")]
    text: String,
    #[serde(rename = "HTML")]
    html: String,
}

/// Mailpit email address
#[derive(Debug, Deserialize)]
struct MailpitAddress {
    #[serde(rename = "Address")]
    address: String,
}

/// Mailpit message headers
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct MailpitHeaders {
    #[serde(rename = "Content-Type")]
    content_type: Option<Vec<String>>,
    #[serde(rename = "Subject")]
    subject: Option<Vec<String>>,
}

/// Get message list from mailpit
async fn get_mailpit_messages() -> Option<MailpitMessages> {
    let client = reqwest::Client::new();
    let response = client
        .get(format!("{}/api/v1/messages", mailpit_api_url()))
        .send()
        .await
        .ok()?;

    if !response.status().is_success() {
        return None;
    }

    response.json::<MailpitMessages>().await.ok()
}

/// Get detailed message from mailpit by ID
async fn get_mailpit_message_detail(id: &str) -> Option<MailpitMessageDetail> {
    let client = reqwest::Client::new();
    let response = client
        .get(format!("{}/api/v1/message/{}", mailpit_api_url(), id))
        .send()
        .await
        .ok()?;

    if !response.status().is_success() {
        return None;
    }

    response.json::<MailpitMessageDetail>().await.ok()
}

/// Get message headers from mailpit by ID
async fn get_mailpit_message_headers(id: &str) -> Option<MailpitHeaders> {
    let client = reqwest::Client::new();
    let response = client
        .get(format!(
            "{}/api/v1/message/{}/headers",
            mailpit_api_url(),
            id
        ))
        .send()
        .await
        .ok()?;

    if !response.status().is_success() {
        return None;
    }

    response.json::<MailpitHeaders>().await.ok()
}

/// Wait for a message to arrive in mailpit (with timeout), returns (summary, detail)
async fn wait_for_message(
    timeout_ms: u64,
) -> Option<(MailpitMessageSummary, MailpitMessageDetail)> {
    let start = std::time::Instant::now();
    while start.elapsed().as_millis() < timeout_ms as u128 {
        if let Some(messages) = get_mailpit_messages().await
            && let Some(summary) = messages.messages.into_iter().next()
            && let Some(detail) = get_mailpit_message_detail(&summary.id).await
        {
            return Some((summary, detail));
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    None
}

#[tokio::test]
#[serial]
async fn test_email_send_plain_text() {
    if !mailpit_available().await {
        eprintln!("Skipping test: mailpit not available. Run: docker compose up mailpit");
        return;
    }

    clear_mailpit().await;

    let activity = EmailSendActivity::new();

    let params = json!({
        "smtp_url": mailpit_smtp_url(),
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

    // Verify email was received by mailpit
    let (summary, detail) = wait_for_message(5000).await.expect("Email not received");
    assert_eq!(summary.subject, "Test Plain Text Email");
    assert_eq!(summary.from.address, "sender@example.com");
    assert_eq!(summary.to[0].address, "recipient@example.com");
    assert!(detail.text.contains("This is a plain text email body"));
}

#[tokio::test]
#[serial]
async fn test_email_send_html() {
    if !mailpit_available().await {
        eprintln!("Skipping test: mailpit not available. Run: docker compose up mailpit");
        return;
    }

    clear_mailpit().await;

    let activity = EmailSendActivity::new();

    let params = json!({
        "smtp_url": mailpit_smtp_url(),
        "from": "sender@example.com",
        "to": ["recipient@example.com"],
        "subject": "Test HTML Email",
        "html_body": "<html><body><h1>Hello</h1><p>This is an HTML email.</p></body></html>"
    });

    let result = activity.execute(params).await.unwrap();

    let output_value = result.to_json_value();
    let email_result = output_value.get("result").unwrap();
    assert_eq!(email_result.get("success").unwrap(), true);

    // Verify email was received by mailpit
    let (summary, detail) = wait_for_message(5000).await.expect("Email not received");
    assert_eq!(summary.subject, "Test HTML Email");

    // Check content type header via headers endpoint
    let headers = get_mailpit_message_headers(&summary.id).await;
    if let Some(headers) = headers
        && let Some(content_types) = &headers.content_type
    {
        assert!(content_types[0].contains("text/html"));
    }
    assert!(detail.html.contains("<h1>Hello</h1>"));
}

#[tokio::test]
#[serial]
async fn test_email_send_multiple_recipients() {
    if !mailpit_available().await {
        eprintln!("Skipping test: mailpit not available. Run: docker compose up mailpit");
        return;
    }

    clear_mailpit().await;

    let activity = EmailSendActivity::new();

    let params = json!({
        "smtp_url": mailpit_smtp_url(),
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
    let (summary, _detail) = wait_for_message(5000).await.expect("Email not received");
    assert_eq!(summary.subject, "Test Multiple Recipients");
    assert_eq!(summary.to.len(), 2);
}

#[tokio::test]
#[serial]
async fn test_email_send_with_cc_and_bcc() {
    if !mailpit_available().await {
        eprintln!("Skipping test: mailpit not available. Run: docker compose up mailpit");
        return;
    }

    clear_mailpit().await;

    let activity = EmailSendActivity::new();

    let params = json!({
        "smtp_url": mailpit_smtp_url(),
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
    let (summary, detail) = wait_for_message(5000).await.expect("Email not received");
    assert_eq!(summary.subject, "Test CC and BCC");
    // Check cc and bcc in detail
    assert_eq!(detail.cc.len(), 1);
    assert_eq!(detail.cc[0].address, "cc@example.com");
    // Note: bcc may or may not appear in mailpit's detail depending on version
}

#[tokio::test]
#[serial]
async fn test_email_send_with_reply_to() {
    if !mailpit_available().await {
        eprintln!("Skipping test: mailpit not available. Run: docker compose up mailpit");
        return;
    }

    clear_mailpit().await;

    let activity = EmailSendActivity::new();

    let params = json!({
        "smtp_url": mailpit_smtp_url(),
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
    let (summary, _detail) = wait_for_message(5000).await.expect("Email not received");
    assert_eq!(summary.subject, "Test Reply-To");
}

#[tokio::test]
#[serial]
async fn test_email_send_empty_recipients_fails() {
    let activity = EmailSendActivity::new();

    let params = json!({
        "smtp_url": mailpit_smtp_url(),
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
        "smtp_url": mailpit_smtp_url(),
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
        "smtp_url": mailpit_smtp_url(),
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
    if !mailpit_available().await {
        eprintln!("Skipping test: mailpit not available. Run: docker compose up mailpit");
        return;
    }

    clear_mailpit().await;

    let activity = EmailSendActivity::new();

    // Use text_body - should be sent as text/plain
    let params = json!({
        "smtp_url": mailpit_smtp_url(),
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
    let (summary, _detail) = wait_for_message(5000).await.expect("Email not received");
    assert_eq!(summary.subject, "Test Default Content Type");

    // Check content type defaults to text/plain via headers endpoint
    let headers = get_mailpit_message_headers(&summary.id).await;
    if let Some(headers) = headers
        && let Some(content_types) = &headers.content_type
    {
        assert!(content_types[0].contains("text/plain"));
    }
}

#[tokio::test]
#[serial]
async fn test_email_activity_name_and_worker() {
    let activity = EmailSendActivity::new();

    assert_eq!(activity.name(), "email_send");
    assert_eq!(activity.worker(), "std");
}
