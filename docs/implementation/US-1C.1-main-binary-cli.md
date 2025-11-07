# US-1C.1: Main Binary and CLI Framework

**Epic**: 1C - StreamFlow Binary and CLI
**Status**: Partial - Core Framework Complete, Remaining Commands Needed
**Estimated Remaining Effort**: ~6 hours
**Dependencies**: None (foundation already in place via US-1A.1.5)

---

## User Story

**As** a platform engineering lead
**I want** a single binary with subcommands to launch different services
**So that** I can deploy StreamFlow with minimal dependencies

---

## Acceptance Criteria

- [x] Main crate `streamflow` that depends on `core`, `api`, and `worker` crates
- [x] CLI framework (clap) with subcommands for different services
- [ ] Subcommands: `serve`, `orchestrator`, `api`, `worker`, `version`, `migrate`
  - [x] `api` - Launch API server only (implemented in US-1A.1.5)
  - [ ] `serve` - Launch all services together (US-1C.2)
  - [ ] `orchestrator` - Launch orchestrator only (US-1C.3 - Post-Epic 2)
  - [ ] `worker` - Launch worker only (US-1C.3 - Post-Epic 2)
  - [ ] `version` - Show version information
  - [ ] `migrate` - Database migration management (US-1C.5 - Post-Epic 2)
- [x] Global flags: `--database-url`, `--log-level`, `--log-format`
- [ ] Help text and examples for each subcommand
- [ ] Binary size: <15MB release build
- [ ] Version information: `streamflow --version` shows semantic version and build info

---

## Current Implementation Status

### ✅ Completed (via US-1A.1.5)

The foundation of the CLI framework is already in place:

1. **Main binary crate** (`streamflow/`)
   - Location: `streamflow/src/main.rs`
   - Dependencies: `core`, `api`, `oauth` crates
   - CLI framework using `clap` with derive macros

2. **Global flags** working correctly:
   - `--database-url` (env: `DATABASE_URL`) - Required for most commands
   - `--log-level` (env: `STREAMFLOW_LOG_LEVEL`) - Default: `info`
   - `--log-format` (env: `STREAMFLOW_LOG_FORMAT`) - Default: `text`, options: `json`

3. **Infrastructure modules**:
   - `streamflow/src/config.rs` - Configuration management with precedence
   - `streamflow/src/logging.rs` - Structured logging setup
   - `streamflow/src/signals.rs` - SIGTERM/SIGINT handling
   - `streamflow/src/commands/mod.rs` - Command module structure
   - `streamflow/src/commands/api.rs` - API server command implementation

4. **Working command**: `streamflow api`
   - Fully functional with configuration precedence (CLI > Env > Defaults)
   - Database connection pooling
   - OAuth 2.0 JWT authentication setup
   - Graceful shutdown on SIGTERM/SIGINT
   - Health endpoints
   - Comprehensive test coverage

### 📋 Remaining Work for Pre-Epic 2

For US-1C.1 specifically, we need to add:

1. **`version` subcommand** (~2 hours)
   - Display semantic version from Cargo.toml
   - Show build timestamp
   - Show git commit hash (if available)
   - Show build platform/architecture
   - Machine-readable output option (`--format json`)

2. **Enhanced help text** (~2 hours)
   - Update main CLI help with better descriptions
   - Add examples to subcommand help
   - Document environment variables in help
   - Add "See also" cross-references between commands

3. **Binary size validation** (~1 hour)
   - Measure release build size
   - Verify <15MB target
   - Document size optimization settings
   - Add CI check for binary size regression

4. **Build metadata** (~1 hour)
   - Add build timestamp to version output
   - Add git commit hash using build.rs
   - Add build platform information

**Total Pre-Epic 2 Work: ~6 hours**

### 📋 Deferred to Later Phases

The following subcommands will be implemented in later user stories:

- **`serve`** - US-1C.2 (Pre-Epic 2, ~8 hours)
  - All-in-one mode launching orchestrator + API + workers
  - Will be the next story to implement after US-1C.1

- **`orchestrator`** - US-1C.3 (Post-Epic 2, ~2 hours)
  - Launch orchestrator service independently
  - Deferred until distributed deployment validation

- **`worker`** - US-1C.3 (Post-Epic 2, ~2 hours)
  - Launch worker service independently
  - Deferred until distributed deployment validation

- **`migrate`** - US-1C.5 (Post-Epic 2, ~3 hours)
  - Database migration management
  - Can use `sqlx migrate` directly until then

---

## Implementation Plan

### Task 1: Add Version Subcommand (~2 hours)

**File**: `streamflow/src/commands/version.rs`

Create new version command that displays:
- Semantic version from Cargo.toml
- Build timestamp (from build.rs)
- Git commit hash (from build.rs)
- Rust version used for build
- Platform/architecture
- Support for JSON output format

**Example output (text format)**:
```
StreamFlow 0.2.0
Build timestamp: 2025-11-06T10:30:00Z
Git commit: 8a6b8d2
Rust version: 1.90.0
Platform: x86_64-unknown-linux-gnu
```

**Example output (json format)**:
```json
{
  "version": "0.2.0",
  "build_timestamp": "2025-11-06T10:30:00Z",
  "git_commit": "8a6b8d2",
  "git_commit_full": "8a6b8d2abcd1234...",
  "rust_version": "1.90.0",
  "platform": "x86_64-unknown-linux-gnu"
}
```

**Implementation steps**:
1. Create `streamflow/src/commands/version.rs`
2. Add `Version` variant to `Commands` enum in `main.rs`
3. Create build.rs to capture build metadata
4. Implement version display logic with text/json formats
5. Add tests for version output formatting

### Task 2: Create Build Script for Metadata (~1 hour)

**File**: `streamflow/build.rs`

Use Rust's build script mechanism to capture:
- Build timestamp (using `chrono::Utc::now()`)
- Git commit hash (using `std::process::Command` to run `git rev-parse`)
- Git branch (using `git rev-parse --abbrev-ref HEAD`)
- Generate constants that can be used in version command

**Environment variables to set**:
- `STREAMFLOW_BUILD_TIMESTAMP`
- `STREAMFLOW_GIT_COMMIT_HASH`
- `STREAMFLOW_GIT_COMMIT_FULL`
- `STREAMFLOW_GIT_BRANCH`

### Task 3: Enhance Help Text and Documentation (~2 hours)

**Files to update**:
- `streamflow/src/main.rs` - Main CLI help
- `streamflow/src/commands/api.rs` - API command help
- Future commands as they're added

**Enhancements**:
1. Add comprehensive descriptions to each command
2. Add examples section to help text
3. Document environment variables
4. Add "See also" references
5. Improve flag descriptions with examples

**Example enhanced help**:
```
StreamFlow 0.2.0
High-performance workflow orchestration platform

USAGE:
    streamflow [OPTIONS] <COMMAND>

COMMANDS:
    api       Launch API server
    version   Show version information
    help      Print this message or the help of the given subcommand(s)

OPTIONS:
    --database-url <URL>         PostgreSQL connection URL [env: DATABASE_URL]
    --log-level <LEVEL>          Log level: trace, debug, info, warn, error [env: STREAMFLOW_LOG_LEVEL] [default: info]
    --log-format <FORMAT>        Log format: text, json [env: STREAMFLOW_LOG_FORMAT] [default: text]
    -h, --help                   Print help information
    -V, --version                Print version information

EXAMPLES:
    # Launch API server with custom port
    streamflow api --port 9090

    # Show detailed version information
    streamflow version --format json

    # View API server help
    streamflow api --help

ENVIRONMENT VARIABLES:
    DATABASE_URL               PostgreSQL connection string (required for most commands)
    STREAMFLOW_LOG_LEVEL       Logging verbosity (default: info)
    STREAMFLOW_LOG_FORMAT      Log output format (default: text)
    STREAMFLOW_API_PORT        API server port (default: 8080)
    STREAMFLOW_API_BIND        API server bind address (default: 0.0.0.0)

See 'streamflow <command> --help' for more information on a specific command.
```

### Task 4: Binary Size Validation (~1 hour)

**Goals**:
- Measure current release build size
- Verify <15MB target is met
- Document optimization settings
- Add CI check for size regression

**Implementation**:
1. Add size measurement to CI/CD pipeline
2. Document current binary size in README
3. Add build profile documentation
4. Create script to measure and report binary size

**Current optimization settings** (already in `Cargo.toml`):
```toml
[profile.release]
opt-level = 'z'     # Optimize for size
lto = true          # Link-time optimization
codegen-units = 1   # Better optimization
strip = true        # Strip symbols
```

**Validation script** (`scr/check-binary-size.sh`):
```bash
#!/bin/bash
cargo build --release
SIZE=$(stat -f%z target/release/streamflow 2>/dev/null || stat -c%s target/release/streamflow)
SIZE_MB=$((SIZE / 1024 / 1024))
echo "Binary size: ${SIZE_MB}MB"
if [ $SIZE_MB -gt 15 ]; then
    echo "ERROR: Binary size exceeds 15MB target"
    exit 1
fi
```

---

## Module Structure

Current structure after US-1A.1.5:

```
streamflow/
├── Cargo.toml
├── build.rs                    # NEW: Build metadata capture
└── src/
    ├── main.rs                 # MODIFY: Add Version command
    ├── config.rs               # Existing: ApiConfig
    ├── logging.rs              # Existing: Log setup
    ├── signals.rs              # Existing: Signal handling
    └── commands/
        ├── mod.rs              # MODIFY: Export version module
        ├── api.rs              # Existing: API server command
        └── version.rs          # NEW: Version command
```

Future structure after US-1C.2 and US-1C.3:

```
streamflow/
└── src/
    └── commands/
        ├── mod.rs
        ├── api.rs          # Existing
        ├── version.rs      # This story
        ├── serve.rs        # US-1C.2 (Pre-Epic 2)
        ├── orchestrator.rs # US-1C.3 (Post-Epic 2)
        ├── worker.rs       # US-1C.3 (Post-Epic 2)
        └── migrate.rs      # US-1C.5 (Post-Epic 2)
```

---

## Testing Strategy

### Unit Tests

1. **Version command tests** (`streamflow/src/commands/version.rs`):
   - Test version output format (text)
   - Test version output format (json)
   - Test JSON parsing and validation
   - Test missing git information (CI environments)

2. **Help text tests**:
   - Verify help messages contain expected sections
   - Test command discovery
   - Validate environment variable documentation

### Integration Tests

1. **Binary invocation tests**:
   ```bash
   # Test version output
   ./target/release/streamflow --version
   ./target/release/streamflow version
   ./target/release/streamflow version --format json

   # Test help output
   ./target/release/streamflow --help
   ./target/release/streamflow help
   ./target/release/streamflow api --help
   ```

2. **Build size validation**:
   - Automated check in CI for binary size <15MB
   - Track size over time to detect bloat

### Manual Testing

Test CLI user experience:
```bash
# Version information
cargo build --release
./target/release/streamflow --version
./target/release/streamflow version
./target/release/streamflow version --format json

# Help text
./target/release/streamflow --help
./target/release/streamflow help
./target/release/streamflow api --help

# Global flags
./target/release/streamflow --log-level debug api
./target/release/streamflow --log-format json api
```

---

## Dependencies

**Crate dependencies** (already in place):
- `clap = { version = "4", features = ["derive", "env"] }` ✅
- `tokio = { workspace = true }` ✅
- `tracing` and `tracing-subscriber` ✅
- `anyhow` and `thiserror` ✅
- `streamflow-api`, `streamflow-core`, `streamflow-oauth` ✅

**Build dependencies** (new):
- `chrono` (workspace) - for build timestamp

**No new runtime dependencies needed.**

---

## Build Metadata Implementation

### build.rs

```rust
use std::process::Command;
use chrono::Utc;

fn main() {
    // Capture build timestamp
    println!("cargo:rustc-env=STREAMFLOW_BUILD_TIMESTAMP={}", Utc::now().to_rfc3339());

    // Capture git commit hash (short)
    if let Ok(output) = Command::new("git").args(["rev-parse", "--short", "HEAD"]).output() {
        if output.status.success() {
            let hash = String::from_utf8_lossy(&output.stdout).trim().to_string();
            println!("cargo:rustc-env=STREAMFLOW_GIT_COMMIT_HASH={}", hash);
        }
    }

    // Capture git commit hash (full)
    if let Ok(output) = Command::new("git").args(["rev-parse", "HEAD"]).output() {
        if output.status.success() {
            let hash = String::from_utf8_lossy(&output.stdout).trim().to_string();
            println!("cargo:rustc-env=STREAMFLOW_GIT_COMMIT_FULL={}", hash);
        }
    }

    // Capture git branch
    if let Ok(output) = Command::new("git").args(["rev-parse", "--abbrev-ref", "HEAD"]).output() {
        if output.status.success() {
            let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
            println!("cargo:rustc-env=STREAMFLOW_GIT_BRANCH={}", branch);
        }
    }

    // Rerun if .git/HEAD changes (detects new commits)
    println!("cargo:rerun-if-changed=../.git/HEAD");
}
```

---

## Version Command Implementation

### commands/version.rs

```rust
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
            build_timestamp: option_env!("STREAMFLOW_BUILD_TIMESTAMP")
                .unwrap_or("unknown")
                .to_string(),
            git_commit: option_env!("STREAMFLOW_GIT_COMMIT_HASH")
                .unwrap_or("unknown")
                .to_string(),
            git_commit_full: option_env!("STREAMFLOW_GIT_COMMIT_FULL")
                .unwrap_or("unknown")
                .to_string(),
            git_branch: option_env!("STREAMFLOW_GIT_BRANCH")
                .unwrap_or("unknown")
                .to_string(),
            rust_version: env!("CARGO_PKG_RUST_VERSION")
                .to_string(),
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
        _ => {
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
    fn test_execute_text_format() {
        let cmd = VersionCommand {
            format: "text".to_string(),
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
```

---

## Updated main.rs

```rust
use anyhow::Result;
use clap::{Parser, Subcommand};

mod commands;
mod config;
mod logging;
mod signals;

/// StreamFlow - High-performance workflow orchestration
#[derive(Parser)]
#[command(
    name = "streamflow",
    version,
    about = "StreamFlow workflow orchestration platform",
    long_about = "StreamFlow is a lightweight, high-performance workflow orchestration \
                  platform designed for edge-to-cloud deployment. Built as a single binary \
                  with PostgreSQL as the only required dependency.\n\n\
                  Examples:\n  \
                    streamflow api --port 8080\n  \
                    streamflow version --format json\n  \
                    streamflow --help"
)]
struct Cli {
    /// Database connection URL
    #[arg(
        long,
        env = "DATABASE_URL",
        global = true,
        help = "PostgreSQL connection URL (postgres://user:pass@host:port/db)"
    )]
    database_url: Option<String>,

    /// Log level
    #[arg(
        long,
        env = "STREAMFLOW_LOG_LEVEL",
        default_value = "info",
        global = true,
        help = "Log level (trace, debug, info, warn, error)"
    )]
    log_level: String,

    /// Log format
    #[arg(
        long,
        env = "STREAMFLOW_LOG_FORMAT",
        default_value = "text",
        global = true,
        help = "Log format (text, json)"
    )]
    log_format: String,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Launch API server
    #[command(about = "Launch the API server on the specified port")]
    Api(commands::api::ApiCommand),

    /// Show version information
    #[command(about = "Display version and build information")]
    Version(commands::version::VersionCommand),

    // Future commands (US-1C.2, US-1C.3, US-1C.5):
    // /// Launch all services (orchestrator + API + workers)
    // Serve(commands::serve::ServeCommand),

    // /// Launch orchestrator only
    // Orchestrator(commands::orchestrator::OrchestratorCommand),

    // /// Launch worker only
    // Worker(commands::worker::WorkerCommand),

    // /// Manage database migrations
    // Migrate(commands::migrate::MigrateCommand),
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Initialize logging (not needed for version command)
    if !matches!(cli.command, Commands::Version(_)) {
        logging::init(&cli.log_level, &cli.log_format)?;
    }

    // Route to command handler
    match cli.command {
        Commands::Api(cmd) => commands::api::execute(cmd, cli.database_url).await,
        Commands::Version(cmd) => commands::version::execute(cmd),
    }
}
```

---

## Success Criteria

### Functional Requirements

- [x] CLI framework with subcommands works (via US-1A.1.5)
- [ ] `streamflow version` displays version information in text format
- [ ] `streamflow version --format json` outputs valid JSON
- [ ] `streamflow --version` shows short version string
- [x] `streamflow api` launches API server (via US-1A.1.5)
- [ ] Help text includes examples and environment variables
- [ ] Binary size is <15MB in release mode

### Non-Functional Requirements

- **User Experience**: Clear, helpful command-line interface
- **Documentation**: Comprehensive help text at all levels
- **Size**: Binary stays within 15MB target
- **Performance**: CLI startup time <100ms

---

## Post-Epic 2 Command Additions

The following commands will be added after Epic 2 performance validation:

### `orchestrator` command (US-1C.3)

Launch orchestrator independently for distributed deployment:
```rust
#[derive(Args)]
pub struct OrchestratorCommand {
    #[arg(long, env = "STREAMFLOW_ORCHESTRATOR_CONSUMER_ID")]
    consumer_id: Option<String>,
}
```

### `worker` command (US-1C.3)

Launch worker independently:
```rust
#[derive(Args)]
pub struct WorkerCommand {
    #[arg(long, env = "STREAMFLOW_WORKER_ACTIVITY_TYPES")]
    activity_types: Vec<String>,

    #[arg(long, env = "STREAMFLOW_API_URL")]
    api_url: String,

    #[arg(long, env = "STREAMFLOW_WORKER_ID")]
    worker_id: Option<String>,
}
```

### `migrate` command (US-1C.5)

Database migration management:
```rust
#[derive(Args)]
pub struct MigrateCommand {
    #[command(subcommand)]
    action: MigrateAction,
}

#[derive(Subcommand)]
enum MigrateAction {
    /// Run pending migrations
    Run,
    /// Show migration status
    Status,
    /// Revert last migration
    Revert,
}
```

---

## References

- **US-1A.1.5**: API Server CLI Launcher (foundation for this story)
- **US-1C.2**: All-in-One Service Launcher (`serve` command)
- **US-1C.3**: Individual Service Launchers (`orchestrator`, `worker` commands)
- **US-1C.5**: Database Migration Management (`migrate` command)
- **docs/architecture.md**: Overall system architecture
- **docs/mvp-requirements.md**: Epic 1C user stories and sequencing

---

## Implementation Checklist

### Pre-Epic 2 (This Story - ~6 hours)

- [ ] Create `build.rs` for build metadata capture
- [ ] Implement `commands/version.rs` with text/json output
- [ ] Add `Version` command to main.rs Commands enum
- [ ] Enhance help text in main.rs with examples
- [ ] Update API command help with better descriptions
- [ ] Add comprehensive tests for version command
- [ ] Create binary size validation script
- [ ] Verify binary size <15MB target
- [ ] Update README with version command documentation
- [ ] Update CLAUDE.md if needed

### Post-Epic 2 (Future Stories)

- [ ] Implement `serve` command (US-1C.2)
- [ ] Implement `orchestrator` command (US-1C.3)
- [ ] Implement `worker` command (US-1C.3)
- [ ] Implement `migrate` command (US-1C.5)

---

## Notes

**Design Decisions**:
1. **Build metadata via build.rs**: Standard Rust approach, metadata available at compile time
2. **Separate version command**: More flexible than just `--version` flag, allows JSON output
3. **Graceful degradation**: Version command works even without git information (CI environments)
4. **Size optimization**: Already using `opt-level='z'` and LTO for minimal binary size

**Known Limitations**:
- Git metadata not available in some CI environments (handled gracefully with "unknown")
- Binary size measurement varies by platform (both BSD and GNU stat supported)

**Future Enhancements** (Post-MVP):
- Add plugin/extension version information
- Show active configuration snapshot
- Add system information (memory, CPU)
- Check for updates mechanism
