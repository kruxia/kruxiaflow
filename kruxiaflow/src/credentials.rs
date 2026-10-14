//! Local login store for client commands (`kruxiaflow login`, `cost`)
//!
//! Holds the tokens from a password login, per API server URL, in
//! `$XDG_CONFIG_HOME/kruxiaflow/credentials.json` (default
//! `~/.config/kruxiaflow/credentials.json`). The file is created with mode
//! 0600 inside a 0700 directory: it contains bearer and refresh tokens.
//! This is a credentials cache, not configuration - settings still come only
//! from flags and environment variables.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;

/// Tokens from a password login to one API server
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredLogin {
    pub username: String,
    pub access_token: String,
    pub access_token_expires_at: DateTime<Utc>,
    /// Rotates on every refresh: the server revokes the previous one
    pub refresh_token: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct CredentialsFile {
    #[serde(default)]
    servers: BTreeMap<String, StoredLogin>,
}

/// Normalize an API URL into the key logins are stored under
pub fn server_key(api_url: &str) -> String {
    api_url.trim_end_matches('/').to_string()
}

/// Path of the credentials file
pub fn credentials_path() -> Result<PathBuf> {
    let config_dir = match std::env::var_os("XDG_CONFIG_HOME") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => PathBuf::from(
            std::env::var_os("HOME").context("HOME is not set; cannot locate credentials file")?,
        )
        .join(".config"),
    };
    Ok(config_dir.join("kruxiaflow").join("credentials.json"))
}

fn load_file() -> Result<CredentialsFile> {
    let path = credentials_path()?;
    match std::fs::read_to_string(&path) {
        Ok(contents) => serde_json::from_str(&contents)
            .with_context(|| format!("Invalid credentials file {}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(CredentialsFile::default()),
        Err(e) => Err(e).with_context(|| format!("Failed to read {}", path.display())),
    }
}

fn save_file(file: &CredentialsFile) -> Result<()> {
    let path = credentials_path()?;
    let dir = path.parent().context("Credentials path has no parent")?;
    std::fs::create_dir_all(dir).with_context(|| format!("Failed to create {}", dir.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }

    // Write a 0600 temp file, then rename over the old one: readers never see
    // a partial file, and the tokens are never world-readable
    let tmp = path.with_extension("json.tmp");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut out = options
        .open(&tmp)
        .with_context(|| format!("Failed to write {}", tmp.display()))?;
    out.write_all(serde_json::to_string_pretty(file)?.as_bytes())?;
    out.sync_all()?;
    std::fs::rename(&tmp, &path)
        .with_context(|| format!("Failed to replace {}", path.display()))?;
    Ok(())
}

/// The stored login for an API server, if any
pub fn load_login(api_url: &str) -> Result<Option<StoredLogin>> {
    Ok(load_file()?.servers.remove(&server_key(api_url)))
}

/// Store (or replace) the login for an API server
pub fn save_login(api_url: &str, login: StoredLogin) -> Result<()> {
    let mut file = load_file()?;
    file.servers.insert(server_key(api_url), login);
    save_file(&file)
}

/// Remove the login for an API server; returns whether one existed
pub fn remove_login(api_url: &str) -> Result<bool> {
    let mut file = load_file()?;
    let existed = file.servers.remove(&server_key(api_url)).is_some();
    if existed {
        save_file(&file)?;
    }
    Ok(existed)
}
