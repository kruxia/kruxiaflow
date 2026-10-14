use anyhow::{Context, Result};
use clap::Args;
use sqlx::PgPool;
use std::io::IsTerminal;

/// Seed user command - Create a user for password login, or reset a password
#[derive(Args)]
pub struct SeedUserCommand {
    /// Username for password login
    #[arg(long, help = "Username for password login")]
    pub username: String,

    /// Email address (required when creating a user)
    #[arg(long, help = "Email address (required when creating a user)")]
    pub email: Option<String>,

    /// Read the password from the first line of stdin instead of prompting
    #[arg(
        long,
        help = "Read the password from stdin instead of prompting",
        long_help = "Read the password from the first line of stdin instead of\n\
prompting on the terminal. For scripts and automation.\n\n\
Example: printf '%s\\n' \"$PASSWORD\" | kruxiaflow seed-user --username admin --password-stdin"
    )]
    pub password_stdin: bool,
}

/// Execute seed-user command
pub async fn execute(cmd: SeedUserCommand, database_url: String) -> Result<()> {
    let password = read_new_password(cmd.password_stdin)?;

    let pool = PgPool::connect(&database_url)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to connect to database: {}", e))?;

    seed_user(&pool, &cmd.username, cmd.email.as_deref(), &password).await
}

/// Read a new password from stdin or an interactive prompt (with confirmation)
pub fn read_new_password(from_stdin: bool) -> Result<String> {
    let password = if from_stdin {
        read_stdin_line()?
    } else {
        if !std::io::stdin().is_terminal() {
            anyhow::bail!("No terminal for the password prompt; use --password-stdin");
        }
        let password = rpassword::prompt_password("New password: ")?;
        if rpassword::prompt_password("Confirm password: ")? != password {
            anyhow::bail!("Passwords do not match");
        }
        password
    };
    if password.is_empty() {
        anyhow::bail!("Password must not be empty");
    }
    Ok(password)
}

/// Read the first line of stdin, without its line ending
pub fn read_stdin_line() -> Result<String> {
    let mut line = String::new();
    std::io::stdin()
        .read_line(&mut line)
        .context("Failed to read password from stdin")?;
    Ok(line.trim_end_matches(['\r', '\n']).to_string())
}

/// Create a user, or set the password (and email, if given) of an existing one
///
/// Unlike `POST /api/v1/oauth/users`, which never changes an existing user's
/// password, this is an operator command with database access, so it doubles
/// as a password reset. The user is (re)activated either way.
pub async fn seed_user(
    pool: &PgPool,
    username: &str,
    email: Option<&str>,
    password: &str,
) -> Result<()> {
    let exists = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM oauth_users WHERE username = $1)",
    )
    .bind(username)
    .fetch_one(pool)
    .await
    .context("Failed to check for existing user")?;

    if !exists && email.is_none() {
        anyhow::bail!("--email is required when creating a new user");
    }

    let password_hash = bcrypt::hash(password, bcrypt::DEFAULT_COST)
        .map_err(|e| anyhow::anyhow!("Failed to hash password: {}", e))?;

    if exists {
        sqlx::query(
            r#"
            UPDATE oauth_users
            SET password_hash = $2, email = COALESCE($3, email), is_active = true
            WHERE username = $1
            "#,
        )
        .bind(username)
        .bind(&password_hash)
        .bind(email)
        .execute(pool)
        .await
        .context("Failed to update user")?;

        // A reset signs the user out everywhere: revoke outstanding refresh
        // tokens (access tokens still expire on their own TTL)
        let revoked = sqlx::query(
            r#"
            UPDATE oauth_refresh_tokens SET revoked_at = NOW()
            WHERE revoked_at IS NULL
              AND user_id = (SELECT id FROM oauth_users WHERE username = $1)
            "#,
        )
        .bind(username)
        .execute(pool)
        .await
        .context("Failed to revoke refresh tokens")?
        .rows_affected();
        tracing::info!(
            username = username,
            revoked_refresh_tokens = revoked,
            "User password reset"
        );
    } else {
        sqlx::query("INSERT INTO oauth_users (username, email, password_hash) VALUES ($1, $2, $3)")
            .bind(username)
            .bind(email)
            .bind(&password_hash)
            .execute(pool)
            .await
            .context("Failed to create user")?;
        tracing::info!(username = username, "User created");
    }

    Ok(())
}
