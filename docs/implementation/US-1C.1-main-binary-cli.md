# US-1C.1: Main Binary and CLI Framework

**Epic**: 1C - StreamFlow Binary and CLI
**Status**: 📋 Ready for Implementation (~6 hours remaining)
**Foundation**: ✅ Complete (via US-1A.1.5)
**Current Binary Size**: 4.5MB (well under 15MB target ✅)

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
  - [x] `api` - Launch API server only (✅ US-1A.1.5)
  - [ ] `version` - Show version information (this story - ~2 hours)
  - [ ] `serve` - Launch all services together (US-1C.2 - next)
  - [ ] `orchestrator` - Launch orchestrator only (US-1C.3 - Post-Epic 2)
  - [ ] `worker` - Launch worker only (US-1C.3 - Post-Epic 2)
  - [ ] `migrate` - Database migration management (US-1C.5 - Post-Epic 2)
- [x] Global flags: `--database-url`, `--log-level`, `--log-format`
- [ ] Help text and examples for each subcommand (~2 hours)
- [ ] Binary size: <15MB release build (~1 hour validation)
- [ ] Version information: `streamflow --version` shows semantic version and build info (~1 hour)

---

## Implementation Status

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

### 📋 Remaining Work for US-1C.1 (~6 hours)

Four tasks to complete this user story:

1. **Build Script for Metadata** (~1 hour)
   - Create `streamflow/build.rs` using `api/build.rs` pattern
   - Use same `BUILD_*` environment variables
   - Capture timestamp, git commit, branch

2. **Version Subcommand** (~2 hours)
   - Implement `streamflow version` command
   - Support text and JSON output formats
   - Display version, build info, platform details

3. **Enhanced Help Text** (~2 hours)
   - Add examples to main CLI help
   - Document environment variables
   - Comprehensive help for all commands

4. **Binary Size Validation** (~1 hour)
   - Create validation script
   - Verify <15MB target (currently 4.5MB)
   - Document optimization settings

### 📋 Deferred to Later Stories

- **`serve`** - US-1C.2 (Pre-Epic 2, ~8 hours)
- **`orchestrator`** - US-1C.3 (Post-Epic 2, ~2 hours)
- **`worker`** - US-1C.3 (Post-Epic 2, ~2 hours)
- **`migrate`** - US-1C.5 (Post-Epic 2, ~3 hours)

---

## Implementation Plan

### Task 1: Build Script for Metadata Capture (~1 hour)

**File**: `streamflow/build.rs` (new)

**Objective**: Capture build-time metadata (git commit, timestamp, branch) using the same pattern as `api/build.rs`.

**Implementation**:

```rust
use std::process::Command;

fn main() {
    // Capture build timestamp in ISO 8601 format
    // Use UTC time for consistency across different build environments
    let output = Command::new("date")
        .arg("-u")
        .arg("+%Y-%m-%dT%H:%M:%SZ")
        .output();

    if let Ok(output) = output {
        if output.status.success() {
            let build_date = String::from_utf8_lossy(&output.stdout);
            println!("cargo:rustc-env=BUILD_TIMESTAMP={}", build_date.trim());
        } else {
            // Fallback if date command fails
            println!("cargo:rustc-env=BUILD_TIMESTAMP=unknown");
        }
    } else {
        // Fallback if date command is not available
        println!("cargo:rustc-env=BUILD_TIMESTAMP=unknown");
    }

    // Capture git commit short hash
    let git_output = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output();

    if let Ok(output) = git_output {
        if output.status.success() {
            let git_hash = String::from_utf8_lossy(&output.stdout);
            println!("cargo:rustc-env=BUILD_GIT_HASH={}", git_hash.trim());
        } else {
            println!("cargo:rustc-env=BUILD_GIT_HASH=unknown");
        }
    } else {
        println!("cargo:rustc-env=BUILD_GIT_HASH=unknown");
    }

    // Capture git commit full hash
    let git_full_output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output();

    if let Ok(output) = git_full_output {
        if output.status.success() {
            let git_hash = String::from_utf8_lossy(&output.stdout);
            println!("cargo:rustc-env=BUILD_GIT_HASH_FULL={}", git_hash.trim());
        } else {
            println!("cargo:rustc-env=BUILD_GIT_HASH_FULL=unknown");
        }
    } else {
        println!("cargo:rustc-env=BUILD_GIT_HASH_FULL=unknown");
    }

    // Capture git branch
    let git_branch_output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output();

    if let Ok(output) = git_branch_output {
        if output.status.success() {
            let git_branch = String::from_utf8_lossy(&output.stdout);
            println!("cargo:rustc-env=BUILD_GIT_BRANCH={}", git_branch.trim());
        } else {
            println!("cargo:rustc-env=BUILD_GIT_BRANCH=unknown");
        }
    } else {
        println!("cargo:rustc-env=BUILD_GIT_BRANCH=unknown");
    }

    // Rerun build script if git HEAD changes
    println!("cargo:rerun-if-changed=../.git/HEAD");
}
```

**Environment Variables Set**:
- `BUILD_TIMESTAMP` - ISO 8601 timestamp (matches `api/build.rs`)
- `BUILD_GIT_HASH` - Short git commit hash
- `BUILD_GIT_HASH_FULL` - Full git commit hash
- `BUILD_GIT_BRANCH` - Git branch name

**Dependencies**: None (uses standard library only, matches `api/build.rs` pattern)

**Testing**:
```bash
# Build and verify environment variables are set
cargo build --release

# Test in CI environment (gracefully handles missing git)
# The script will output "unknown" for missing values
```

---

### Task 2: Version Subcommand (~2 hours)

**File**: `streamflow/src/commands/version.rs` (new)

**Objective**: Implement version command with text and JSON output formats.

**Implementation**:

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

**Changes to streamflow/Cargo.toml**:
```toml
[dependencies]
# Add serde for JSON serialization (may already be in workspace)
serde = { workspace = true }
serde_json = { workspace = true }
```

**Changes to streamflow/src/commands/mod.rs**:
```rust
pub mod api;
pub mod version;  // NEW
```

**Changes to streamflow/src/main.rs**:

1. Update Commands enum:
```rust
#[derive(Subcommand)]
enum Commands {
    /// Launch API server
    Api(commands::api::ApiCommand),

    /// Show version information
    #[command(about = "Display version and build information")]
    Version(commands::version::VersionCommand),  // NEW
}
```

2. Update main() to skip logging for version command:
```rust
#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Initialize logging (skip for version command)
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

**Example Output (Text)**:
```
StreamFlow 0.2.0
Build timestamp: 2025-11-07T10:30:00Z
Git commit: 406bc2d
Git branch: epic-1A-api
Rust version: 1.90.0
Platform: x86_64-darwin
```

**Example Output (JSON)**:
```json
{
  "version": "0.2.0",
  "build_timestamp": "2025-11-07T10:30:00Z",
  "git_commit": "406bc2d",
  "git_commit_full": "406bc2d1234...",
  "git_branch": "epic-1A-api",
  "rust_version": "1.90.0",
  "platform": "x86_64-darwin"
}
```

**Testing**:
```bash
cargo build --release

# Test text format (default)
./target/release/streamflow version

# Test JSON format
./target/release/streamflow version --format json

# Test short version flag
./target/release/streamflow --version
```

---

### Task 3: Enhanced Help Text (~2 hours)

**Files to Update**:
- `streamflow/src/main.rs` - Main CLI help
- `streamflow/src/commands/api.rs` - API command help
- `streamflow/src/commands/version.rs` - Version command help

**Update main.rs Cli struct**:

```rust
/// StreamFlow - High-performance workflow orchestration
#[derive(Parser)]
#[command(
    name = "streamflow",
    version,
    about = "StreamFlow workflow orchestration platform",
    long_about = "StreamFlow is a lightweight, high-performance workflow orchestration \
platform designed for edge-to-cloud deployment. Built as a single binary \
with PostgreSQL as the only required dependency.\n\n\
EXAMPLES:\n  \
  streamflow api --port 8080\n  \
  streamflow version --format json\n  \
  streamflow --help\n\n\
ENVIRONMENT VARIABLES:\n  \
  DATABASE_URL               PostgreSQL connection string (required for most commands)\n  \
  STREAMFLOW_LOG_LEVEL       Logging verbosity (default: info)\n  \
  STREAMFLOW_LOG_FORMAT      Log output format (default: text)\n  \
  STREAMFLOW_API_PORT        API server port (default: 8080)\n  \
  STREAMFLOW_API_BIND        API server bind address (default: 0.0.0.0)"
)]
struct Cli {
    /// Database connection URL
    #[arg(
        long,
        env = "DATABASE_URL",
        global = true,
        help = "PostgreSQL connection URL (postgres://user:pass@host:port/db)",
        long_help = "PostgreSQL connection URL\n\n\
Example: postgres://user:pass@localhost:5432/streamflow\n\
Required for all commands except 'version'"
    )]
    database_url: Option<String>,

    /// Log level
    #[arg(
        long,
        env = "STREAMFLOW_LOG_LEVEL",
        default_value = "info",
        global = true,
        help = "Log level (trace, debug, info, warn, error)",
        long_help = "Log level for structured logging\n\n\
Options: trace, debug, info, warn, error\n\
Default: info\n\
Example: --log-level debug"
    )]
    log_level: String,

    /// Log format
    #[arg(
        long,
        env = "STREAMFLOW_LOG_FORMAT",
        default_value = "text",
        global = true,
        help = "Log format (text, json)",
        long_help = "Log output format\n\n\
Options: text (human-readable), json (machine-readable)\n\
Default: text\n\
Example: --log-format json for production logging"
    )]
    log_format: String,

    #[command(subcommand)]
    command: Commands,
}
```

**Update Commands enum**:

```rust
#[derive(Subcommand)]
enum Commands {
    /// Launch API server
    #[command(
        about = "Launch the API server on the specified port",
        long_about = "Launch the HTTP API server\n\n\
The API server provides RESTful endpoints for workflow management, \
authentication, and monitoring.\n\n\
EXAMPLES:\n  \
  streamflow api\n  \
  streamflow api --port 9090 --bind 127.0.0.1\n  \
  DATABASE_URL=postgres://localhost/db streamflow api\n\n\
ENDPOINTS:\n  \
  GET  /health              - Liveness probe\n  \
  GET  /health/ready        - Readiness probe\n  \
  GET  /api/v1/info         - Service information\n  \
  POST /api/v1/auth/token   - Authentication\n  \
  See /api/v1/openapi.json for full API documentation"
    )]
    Api(commands::api::ApiCommand),

    /// Show version information
    #[command(
        about = "Display version and build information",
        long_about = "Display version and build information\n\n\
Shows StreamFlow version, build timestamp, git commit, and platform details.\n\n\
EXAMPLES:\n  \
  streamflow version\n  \
  streamflow version --format json"
    )]
    Version(commands::version::VersionCommand),
}
```

**Update commands/api.rs ApiCommand**:

```rust
#[derive(Args)]
pub struct ApiCommand {
    /// Port to bind to
    #[arg(
        short,
        long,
        env = "STREAMFLOW_API_PORT",
        help = "Port to bind API server to",
        long_help = "Port to bind API server to\n\n\
Default: 8080\n\
Example: --port 9090"
    )]
    port: Option<u16>,

    /// Address to bind to
    #[arg(
        short,
        long,
        env = "STREAMFLOW_API_BIND",
        help = "Address to bind API server to (e.g., 0.0.0.0, 127.0.0.1)",
        long_help = "Address to bind API server to\n\n\
Options:\n  \
  0.0.0.0    - All network interfaces (default)\n  \
  127.0.0.1  - Localhost only (development)\n\
Example: --bind 127.0.0.1"
    )]
    bind: Option<String>,
}
```

**Testing**:
```bash
# Test main help
./target/release/streamflow --help

# Test api command help
./target/release/streamflow api --help

# Test version command help
./target/release/streamflow version --help

# Verify examples and environment variables are documented
```

---

### Task 4: Binary Size Validation (~1 hour)

**File**: `scripts/check-binary-size.sh` (new)

**Implementation**:

```bash
#!/bin/bash
# Check StreamFlow binary size against 15MB target

set -e

BINARY_PATH="${1:-target/release/streamflow}"
TARGET_SIZE_MB=15

if [ ! -f "$BINARY_PATH" ]; then
    echo "ERROR: Binary not found at $BINARY_PATH"
    echo "Run: cargo build --release"
    exit 1
fi

# Get file size (works on both macOS and Linux)
if [[ "$OSTYPE" == "darwin"* ]]; then
    SIZE_BYTES=$(stat -f%z "$BINARY_PATH")
else
    SIZE_BYTES=$(stat -c%s "$BINARY_PATH")
fi

SIZE_MB=$((SIZE_BYTES / 1024 / 1024))
SIZE_KB=$((SIZE_BYTES / 1024))

echo "Binary: $BINARY_PATH"
echo "Size: ${SIZE_MB}MB (${SIZE_KB}KB)"
echo "Target: <${TARGET_SIZE_MB}MB"

if [ $SIZE_MB -ge $TARGET_SIZE_MB ]; then
    echo "❌ FAIL: Binary size (${SIZE_MB}MB) exceeds ${TARGET_SIZE_MB}MB target"
    exit 1
else
    echo "✅ PASS: Binary size is within target"
    exit 0
fi
```

**Make Executable**:
```bash
chmod +x scripts/check-binary-size.sh
```

**Current Optimization Settings** (already in root `Cargo.toml`):
```toml
[profile.release]
opt-level = 'z'     # Optimize for size
lto = true          # Link-time optimization
codegen-units = 1   # Better optimization
strip = true        # Strip symbols
```

**Testing**:
```bash
# Build release binary
cargo build --release

# Run size check
./scripts/check-binary-size.sh

# Expected output:
# Binary: target/release/streamflow
# Size: 4MB (4608KB)
# Target: <15MB
# ✅ PASS: Binary size is within target
```

---

## Module Structure

**Current (after US-1A.1.5)**:
```
streamflow/
├── Cargo.toml
└── src/
    ├── main.rs
    ├── config.rs
    ├── logging.rs
    ├── signals.rs
    └── commands/
        ├── mod.rs
        └── api.rs
```

**After This Story (US-1C.1)**:
```
streamflow/
├── Cargo.toml
├── build.rs                    # NEW: Build metadata
└── src/
    ├── main.rs                 # MODIFIED: Version command
    ├── config.rs
    ├── logging.rs
    ├── signals.rs
    └── commands/
        ├── mod.rs              # MODIFIED: Export version
        ├── api.rs              # MODIFIED: Enhanced help
        └── version.rs          # NEW: Version command
```

**Future (US-1C.2, US-1C.3)**:
```
streamflow/
└── src/
    └── commands/
        ├── mod.rs
        ├── api.rs          # Existing
        ├── version.rs      # This story
        ├── serve.rs        # US-1C.2
        ├── orchestrator.rs # US-1C.3
        ├── worker.rs       # US-1C.3
        └── migrate.rs      # US-1C.5
```

---

## Testing Strategy

### Unit Tests

**Version Command** (`streamflow/src/commands/version.rs`):
- Test version info creation
- Test JSON serialization
- Test text output format
- Test JSON output format

**Run Tests**:
```bash
# Test version command
cargo test -p streamflow version

# All streamflow tests
cargo test -p streamflow
```

### Integration Tests

**Binary Invocation**:
```bash
# Build release binary
cargo build --release

# Test version commands
./target/release/streamflow --version
./target/release/streamflow version
./target/release/streamflow version --format json

# Test help commands
./target/release/streamflow --help
./target/release/streamflow api --help
./target/release/streamflow version --help

# Test binary size
./scripts/check-binary-size.sh
```

### Manual Verification

1. **Version Output (Text)**:
   ```bash
   ./target/release/streamflow version
   ```
   Expected:
   ```
   StreamFlow 0.2.0
   Build timestamp: 2025-11-07T...
   Git commit: 406bc2d
   Git branch: epic-1A-api
   Rust version: 1.90.0
   Platform: x86_64-darwin
   ```

2. **Version Output (JSON)**:
   ```bash
   ./target/release/streamflow version --format json
   ```
   Expected: Valid JSON with all fields

3. **Help Quality**:
   - Examples are clear and accurate
   - Environment variables documented
   - Long help more detailed than short help

4. **Binary Size**:
   - Verify <15MB (currently 4.5MB ✅)

---

## Dependencies

**No new external dependencies needed**:
- `clap` - ✅ Already in streamflow/Cargo.toml
- `serde` / `serde_json` - ✅ Already in workspace
- `build.rs` uses standard library only

---

## Success Criteria

### Functional Requirements

- [x] CLI framework with subcommands (via US-1A.1.5)
- [ ] `streamflow version` displays version in text format
- [ ] `streamflow version --format json` outputs valid JSON
- [ ] `streamflow --version` shows short version string
- [x] `streamflow api` launches API server (via US-1A.1.5)
- [ ] Help text includes examples and environment variables
- [ ] Binary size <15MB in release mode

### Non-Functional Requirements

- **User Experience**: Clear, helpful CLI interface
- **Documentation**: Comprehensive help at all levels
- **Size**: Binary within 15MB target
- **Performance**: CLI startup <100ms

---

## Implementation Checklist

### Task 1: Build Metadata (~1 hour)
- [ ] Create `streamflow/build.rs` (based on `api/build.rs`)
- [ ] Use `BUILD_*` environment variables (not `STREAMFLOW_*`)
- [ ] Test build script captures git metadata
- [ ] Verify graceful handling of missing git
- [ ] Test rebuild triggers on git changes

### Task 2: Version Command (~2 hours)
- [ ] Create `streamflow/src/commands/version.rs`
- [ ] Add `serde`/`serde_json` to dependencies
- [ ] Implement `VersionCommand` with text/json
- [ ] Add unit tests
- [ ] Update `commands/mod.rs`
- [ ] Update `main.rs` Commands enum
- [ ] Skip logging for version command
- [ ] Test text and JSON output

### Task 3: Enhanced Help (~2 hours)
- [ ] Update `main.rs` Cli with `long_about`
- [ ] Add examples to main help
- [ ] Add environment variable docs
- [ ] Update Commands enum with detailed help
- [ ] Update `ApiCommand` with `long_help`
- [ ] Update `VersionCommand` with `long_help`
- [ ] Test all help outputs

### Task 4: Binary Size (~1 hour)
- [ ] Create `scripts/check-binary-size.sh`
- [ ] Make script executable
- [ ] Test with release binary
- [ ] Document in README
- [ ] Verify 4.5MB < 15MB target ✅

### Final Verification
- [ ] All tests pass: `cargo test`
- [ ] No warnings: `cargo build --release`
- [ ] Binary size verified
- [ ] Help comprehensive
- [ ] Version works correctly
- [ ] Mark US-1C.1 complete

---

## Post-Epic 2 Commands

These will be added in future stories:

### `orchestrator` (US-1C.3)
```rust
#[derive(Args)]
pub struct OrchestratorCommand {
    #[arg(long, env = "STREAMFLOW_ORCHESTRATOR_CONSUMER_ID")]
    consumer_id: Option<String>,
}
```

### `worker` (US-1C.3)
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

### `migrate` (US-1C.5)
```rust
#[derive(Args)]
pub struct MigrateCommand {
    #[command(subcommand)]
    action: MigrateAction,
}

#[derive(Subcommand)]
enum MigrateAction {
    Run,
    Status,
    Revert,
}
```

---

## References

- **US-1A.1.5**: API Server CLI Launcher (foundation)
- **US-1C.2**: All-in-One Service Launcher (next story)
- **US-1C.3**: Individual Service Launchers (Post-Epic 2)
- **US-1C.5**: Database Migration Management (Post-Epic 2)
- **api/build.rs**: Pattern for build metadata capture
- **docs/architecture.md**: System architecture
- **docs/mvp-requirements.md**: Epic 1C requirements

---

## Notes

**Design Decisions**:
1. **BUILD_* variables**: Match `api/build.rs` pattern for consistency
2. **Separate version command**: More flexible than just `--version` flag
3. **Graceful degradation**: Works without git (CI environments)
4. **Size optimization**: Already using optimal settings

**Known Limitations**:
- Git metadata unavailable in some CI environments (handled gracefully)
- Binary size measurement varies by platform (script handles both)

**Timeline**: ~6 hours total
1. Build script: 1 hour
2. Version command: 2 hours
3. Enhanced help: 2 hours
4. Size validation: 1 hour

**Next**: US-1C.2 (All-in-One Service Launcher)
