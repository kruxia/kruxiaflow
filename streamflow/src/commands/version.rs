use anyhow::Result;
use clap::Args;
use serde::Serialize;

#[derive(Args)]
pub struct VersionCommand {
    /// Output format: text or json
    #[arg(long, value_name = "FORMAT", default_value = "text")]
    format: String,
}

#[derive(Serialize)]
struct VersionInfo {
    version: String,
    build_timestamp: String,
    git_commit: String,
    git_commit_full: String,
    git_branch: String,
    rust_version: String,
    platform: String,
}

impl VersionInfo {
    fn new() -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION").to_string(),
            build_timestamp: option_env!("BUILD_TIMESTAMP")
                .unwrap_or("unknown")
                .to_string(),
            git_commit: option_env!("BUILD_GIT_HASH")
                .unwrap_or("unknown")
                .to_string(),
            git_commit_full: option_env!("BUILD_GIT_HASH_FULL")
                .unwrap_or("unknown")
                .to_string(),
            git_branch: option_env!("BUILD_GIT_BRANCH")
                .unwrap_or("unknown")
                .to_string(),
            rust_version: env!("CARGO_PKG_RUST_VERSION").to_string(),
            platform: format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS),
        }
    }
}

pub fn execute(cmd: VersionCommand) -> Result<()> {
    let version_info = VersionInfo::new();

    match cmd.format.as_str() {
        "json" => {
            println!("{}", serde_json::to_string_pretty(&version_info)?);
        }
        "detail" => {
            // Text format
            println!("StreamFlow {}", version_info.version);
            println!("Build timestamp: {}", version_info.build_timestamp);
            println!("Git commit: {}", version_info.git_commit);
            if version_info.git_branch != "unknown" {
                println!("Git branch: {}", version_info.git_branch);
            }
            println!("Rust version: {}", version_info.rust_version);
            println!("Platform: {}", version_info.platform);
        }
        _ => {
            // Default to simple text format
            println!(
                "StreamFlow {} ({})",
                version_info.version, version_info.build_timestamp
            );
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_info_creation() {
        let info = VersionInfo::new();
        assert!(!info.version.is_empty());
        assert!(!info.platform.is_empty());
    }

    #[test]
    fn test_version_json_serialization() {
        let info = VersionInfo::new();
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("version"));
        assert!(json.contains("platform"));
    }

    #[test]
    fn test_execute_default_format() {
        let cmd = VersionCommand {
            format: "text".to_string(), // Any non-json/detail format uses simple default
        };
        assert!(execute(cmd).is_ok());
    }

    #[test]
    fn test_execute_detail_format() {
        let cmd = VersionCommand {
            format: "detail".to_string(),
        };
        assert!(execute(cmd).is_ok());
    }

    #[test]
    fn test_execute_json_format() {
        let cmd = VersionCommand {
            format: "json".to_string(),
        };
        assert!(execute(cmd).is_ok());
    }
}
