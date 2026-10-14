// Client CLI: password login for client commands (`cost`).
//
// `login` exchanges a username/password for tokens (OAuth2 password grant)
// and stores them per API server (see crate::credentials). Client commands
// then reuse the access token until it expires and renew it with the refresh
// token, so the slow bcrypt password check runs only at login.

use crate::credentials::{self, StoredLogin};
use anyhow::{Context, Result, anyhow};
use chrono::Utc;
use clap::Args;
use reqwest::Client;
use serde::Deserialize;
use std::io::{IsTerminal, Write};
use std::time::Duration;

/// Renew access tokens this long before they expire
const EXPIRY_MARGIN_SECS: i64 = 60;

#[derive(Args)]
pub struct LoginCommand {
    /// API server URL
    #[arg(
        long,
        env = "KRUXIAFLOW_API_URL",
        default_value = "http://127.0.0.1:8080"
    )]
    pub api_url: String,

    /// Username (prompted for if omitted)
    #[arg(long, env = "KRUXIAFLOW_USERNAME")]
    pub username: Option<String>,

    /// Read the password from the first line of stdin instead of prompting
    #[arg(long)]
    pub password_stdin: bool,

    /// Request timeout in seconds (password checks are slow on small hosts)
    #[arg(long, default_value = "60")]
    pub timeout: u64,
}

#[derive(Args)]
pub struct LogoutCommand {
    /// API server URL
    #[arg(
        long,
        env = "KRUXIAFLOW_API_URL",
        default_value = "http://127.0.0.1:8080"
    )]
    pub api_url: String,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: u64,
    refresh_token: Option<String>,
}

/// Execute login command
pub async fn execute_login(cmd: LoginCommand) -> Result<()> {
    let api_url = credentials::server_key(&cmd.api_url);
    let interactive = std::io::stdin().is_terminal();

    let username = match cmd.username {
        Some(username) => username,
        None if interactive && !cmd.password_stdin => {
            eprint!("Username: ");
            std::io::stderr().flush()?;
            let mut line = String::new();
            std::io::stdin().read_line(&mut line)?;
            line.trim().to_string()
        }
        None => anyhow::bail!("--username (or KRUXIAFLOW_USERNAME) is required"),
    };
    if username.is_empty() {
        anyhow::bail!("Username must not be empty");
    }

    let password = if cmd.password_stdin {
        super::seed_user::read_stdin_line()?
    } else if interactive {
        rpassword::prompt_password("Password: ")?
    } else {
        anyhow::bail!("No terminal for the password prompt; use --password-stdin");
    };

    let http = http_client(cmd.timeout)?;
    let tokens = request_token(
        &http,
        &api_url,
        &serde_json::json!({
            "grant_type": "password",
            "username": username,
            "password": password,
        }),
    )
    .await?;
    let login = stored_login(username, tokens)?;
    let username = login.username.clone();
    credentials::save_login(&api_url, login)?;

    eprintln!(
        "Logged in to {} as {} (credentials saved in {})",
        api_url,
        username,
        credentials::credentials_path()?.display()
    );
    Ok(())
}

/// Execute logout command
pub fn execute_logout(cmd: LogoutCommand) -> Result<()> {
    let api_url = credentials::server_key(&cmd.api_url);
    if credentials::remove_login(&api_url)? {
        eprintln!("Logged out of {}", api_url);
    } else {
        eprintln!("Not logged in to {}", api_url);
    }
    Ok(())
}

/// Access token from the stored login for `api_url`, renewed if expired
///
/// Returns `None` when there is no stored login for this server.
pub async fn stored_login_token(http: &Client, api_url: &str) -> Result<Option<String>> {
    let Some(login) = credentials::load_login(api_url)? else {
        return Ok(None);
    };

    let renew_at = login.access_token_expires_at - chrono::Duration::seconds(EXPIRY_MARGIN_SECS);
    if Utc::now() < renew_at {
        return Ok(Some(login.access_token));
    }

    let tokens = request_token(
        http,
        api_url,
        &serde_json::json!({
            "grant_type": "refresh_token",
            "refresh_token": login.refresh_token,
        }),
    )
    .await
    .map_err(|e| {
        anyhow!(
            "{}\nThe saved login for {} is no longer valid; run `kruxiaflow login`.",
            e,
            api_url
        )
    })?;
    let renewed = stored_login(login.username, tokens)?;
    let access_token = renewed.access_token.clone();
    credentials::save_login(api_url, renewed)?;
    Ok(Some(access_token))
}

fn http_client(timeout_secs: u64) -> Result<Client> {
    Client::builder()
        .timeout(Duration::from_secs(timeout_secs))
        .build()
        .context("Failed to build HTTP client")
}

async fn request_token(
    http: &Client,
    api_url: &str,
    body: &serde_json::Value,
) -> Result<TokenResponse> {
    let response = http
        .post(format!("{}/api/v1/oauth/token", api_url))
        .json(body)
        .send()
        .await
        .with_context(|| format!("Failed to reach API server at {}", api_url))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(anyhow!("Token request failed ({}): {}", status, body));
    }
    Ok(response.json::<TokenResponse>().await?)
}

fn stored_login(username: String, tokens: TokenResponse) -> Result<StoredLogin> {
    Ok(StoredLogin {
        username,
        access_token_expires_at: Utc::now() + chrono::Duration::seconds(tokens.expires_in as i64),
        access_token: tokens.access_token,
        refresh_token: tokens
            .refresh_token
            .context("Server did not return a refresh token")?,
    })
}
