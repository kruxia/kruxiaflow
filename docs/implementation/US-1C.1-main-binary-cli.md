# US-1C.1: Main Binary and CLI Framework

**Epic**: 1C - Kruxia Flow Binary and CLI
**Status**: ✅ **COMPLETE**
**Foundation**: ✅ Complete (via US-1A.1.5)
**Current Binary Size**: 4.5MB (well under 15MB target ✅)

---

## User Story

**As** a platform engineering lead
**I want** a single binary with subcommands to launch different services
**So that** I can deploy Kruxia Flow with minimal dependencies

---

## Acceptance Criteria

- [x] Main crate `kruxiaflow` that depends on `core`, `api`, and `worker` crates
- [x] CLI framework (clap) with subcommands for different services
- [x] Subcommands implemented for US-1C.1:
  - [x] `api` - Launch API server only (✅ US-1A.1.5)
  - [x] `version` - Show version information (✅ US-1C.1)
  - [ ] `serve` - Launch all services together (US-1C.2 - next)
  - [ ] `orchestrator` - Launch orchestrator only (US-1C.3 - Post-Epic 2)
  - [ ] `worker` - Launch worker only (US-1C.3 - Post-Epic 2)
  - [ ] `migrate` - Database migration management (US-1C.5 - Post-Epic 2)
- [x] Global flags: `--database-url`, `--log-level`, `--log-format`
- [x] Help text and examples for each subcommand
- [x] Binary size: <15MB release build (actual: 4MB ✅)
- [x] Version information: `kruxiaflow version` shows semantic version and build info

---

## Implementation Status

### ✅ Completed (via US-1A.1.5)

The foundation of the CLI framework is already in place:

1. **Main binary crate** (`kruxiaflow/`)
   - Location: `kruxiaflow/src/main.rs`
   - Dependencies: `core`, `api`, `oauth` crates
   - CLI framework using `clap` with derive macros

2. **Global flags** working correctly:
   - `--database-url` (env: `DATABASE_URL`) - Required for most commands
   - `--log-level` (env: `KRUXIAFLOW_LOG_LEVEL`) - Default: `info`
   - `--log-format` (env: `KRUXIAFLOW_LOG_FORMAT`) - Default: `text`, options: `json`

3. **Infrastructure modules**:
   - `kruxiaflow/src/config.rs` - Configuration management with precedence
   - `kruxiaflow/src/logging.rs` - Structured logging setup
   - `kruxiaflow/src/signals.rs` - SIGTERM/SIGINT handling
   - `kruxiaflow/src/commands/mod.rs` - Command module structure
   - `kruxiaflow/src/commands/api.rs` - API server command implementation

4. **Working command**: `kruxiaflow api`
   - Fully functional with configuration precedence (CLI > Env > Defaults)
   - Database connection pooling
   - OAuth 2.0 JWT authentication setup
   - Graceful shutdown on SIGTERM/SIGINT
   - Health endpoints
   - Comprehensive test coverage

### ✅ Completed Work for US-1C.1

All four tasks completed:

1. **Build Script for Metadata** ✅
   - Created `kruxiaflow/build.rs` using `api/build.rs` pattern
   - Uses `BUILD_*` environment variables
   - Captures timestamp, git commit, branch

2. **Version Subcommand** ✅
   - Implemented `kruxiaflow version` command
   - Supports text and JSON output formats
   - Displays version, build info, platform details

3. **Enhanced Help Text** ✅
   - Added examples to main CLI help
   - Documented environment variables
   - Comprehensive help for all commands

4. **Binary Size Validation** ✅
   - Created `scripts/check-binary-size.sh`
   - Verified <15MB target (actual: 4MB)
   - Documented optimization settings

### 📋 Deferred to Later Stories

- **`serve`** - US-1C.2 (Pre-Epic 2, ~8 hours)
- **`orchestrator`** - US-1C.3 (Post-Epic 2, ~2 hours)
- **`worker`** - US-1C.3 (Post-Epic 2, ~2 hours)
- **`migrate`** - US-1C.5 (Post-Epic 2, ~3 hours)

---

## Implementation Plan

### Task 1: Build Script for Metadata Capture (~1 hour)

**File**: `kruxiaflow/build.rs` (new)

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

**File**: `kruxiaflow/src/commands/version.rs` (new)

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
            println!("Kruxia Flow {}", version_info.version);
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

**Changes to kruxiaflow/Cargo.toml**:
```toml
[dependencies]
# Add serde for JSON serialization (may already be in workspace)
serde = { workspace = true }
serde_json = { workspace = true }
```

**Changes to kruxiaflow/src/commands/mod.rs**:
```rust
pub mod api;
pub mod version;  // NEW
```

**Changes to kruxiaflow/src/main.rs**:

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
Kruxia Flow 0.2.0
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
./target/release/kruxiaflow version

# Test JSON format
./target/release/kruxiaflow version --format json

# Test short version flag
./target/release/kruxiaflow --version
```

---

### Task 3: Enhanced Help Text (~2 hours)

**Files to Update**:
- `kruxiaflow/src/main.rs` - Main CLI help
- `kruxiaflow/src/commands/api.rs` - API command help
- `kruxiaflow/src/commands/version.rs` - Version command help

**Update main.rs Cli struct**:

```rust
/// Kruxia Flow - High-performance workflow orchestration
#[derive(Parser)]
#[command(
    name = "kruxiaflow",
    version,
    about = "Kruxia Flow workflow orchestration platform",
    long_about = "Kruxia Flow is a lightweight, high-performance workflow orchestration \
platform designed for edge-to-cloud deployment. Built as a single binary \
with PostgreSQL as the only required dependency.\n\n\
EXAMPLES:\n  \
  kruxiaflow api --port 8080\n  \
  kruxiaflow version --format json\n  \
  kruxiaflow --help\n\n\
ENVIRONMENT VARIABLES:\n  \
  DATABASE_URL               PostgreSQL connection string (required for most commands)\n  \
  KRUXIAFLOW_LOG_LEVEL       Logging verbosity (default: info)\n  \
  KRUXIAFLOW_LOG_FORMAT      Log output format (default: text)\n  \
  KRUXIAFLOW_API_PORT        API server port (default: 8080)\n  \
  KRUXIAFLOW_API_BIND        API server bind address (default: 0.0.0.0)"
)]
struct Cli {
    /// Database connection URL
    #[arg(
        long,
        env = "DATABASE_URL",
        global = true,
        help = "PostgreSQL connection URL (postgres://user:pass@host:port/db)",
        long_help = "PostgreSQL connection URL\n\n\
Example: postgres://user:pass@localhost:5432/kruxiaflow\n\
Required for all commands except 'version'"
    )]
    database_url: Option<String>,

    /// Log level
    #[arg(
        long,
        env = "KRUXIAFLOW_LOG_LEVEL",
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
        env = "KRUXIAFLOW_LOG_FORMAT",
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
  kruxiaflow api\n  \
  kruxiaflow api --port 9090 --bind 127.0.0.1\n  \
  DATABASE_URL=postgres://localhost/db kruxiaflow api\n\n\
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
Shows Kruxia Flow version, build timestamp, git commit, and platform details.\n\n\
EXAMPLES:\n  \
  kruxiaflow version\n  \
  kruxiaflow version --format json"
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
        env = "KRUXIAFLOW_API_PORT",
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
        env = "KRUXIAFLOW_API_BIND",
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
./target/release/kruxiaflow --help

# Test api command help
./target/release/kruxiaflow api --help

# Test version command help
./target/release/kruxiaflow version --help

# Verify examples and environment variables are documented
```

---

### Task 4: Binary Size Validation (~1 hour)

**File**: `scripts/check-binary-size.sh` (new)

**Implementation**:

```bash
#!/bin/bash
# Check Kruxia Flow binary size against 15MB target

set -e

BINARY_PATH="${1:-target/release/kruxiaflow}"
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
# Binary: target/release/kruxiaflow
# Size: 4MB (4608KB)
# Target: <15MB
# ✅ PASS: Binary size is within target
```

---

## Module Structure

**Current (after US-1A.1.5)**:
```
kruxiaflow/
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
kruxiaflow/
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
kruxiaflow/
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

**Version Command** (`kruxiaflow/src/commands/version.rs`):
- Test version info creation
- Test JSON serialization
- Test text output format
- Test JSON output format

**Run Tests**:
```bash
# Test version command
cargo test -p kruxiaflow version

# All kruxiaflow tests
cargo test -p kruxiaflow
```

### Integration Tests

**Binary Invocation**:
```bash
# Build release binary
cargo build --release

# Test version commands
./target/release/kruxiaflow --version
./target/release/kruxiaflow version
./target/release/kruxiaflow version --format json

# Test help commands
./target/release/kruxiaflow --help
./target/release/kruxiaflow api --help
./target/release/kruxiaflow version --help

# Test binary size
./scripts/check-binary-size.sh
```

### Manual Verification

1. **Version Output (Text)**:
   ```bash
   ./target/release/kruxiaflow version
   ```
   Expected:
   ```
   Kruxia Flow 0.2.0
   Build timestamp: 2025-11-07T...
   Git commit: 406bc2d
   Git branch: epic-1A-api
   Rust version: 1.90.0
   Platform: x86_64-darwin
   ```

2. **Version Output (JSON)**:
   ```bash
   ./target/release/kruxiaflow version --format json
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
- `clap` - ✅ Already in kruxiaflow/Cargo.toml
- `serde` / `serde_json` - ✅ Already in workspace
- `build.rs` uses standard library only

---

## Success Criteria

### Functional Requirements

- [x] CLI framework with subcommands (via US-1A.1.5)
- [x] `kruxiaflow version` displays version in text format
- [x] `kruxiaflow version --format json` outputs valid JSON
- [x] `kruxiaflow --version` shows short version string
- [x] `kruxiaflow api` launches API server (via US-1A.1.5)
- [x] Help text includes examples and environment variables
- [x] Binary size <15MB in release mode (actual: 4MB)

### Non-Functional Requirements

- **User Experience**: Clear, helpful CLI interface
- **Documentation**: Comprehensive help at all levels
- **Size**: Binary within 15MB target
- **Performance**: CLI startup <100ms

---

## Implementation Checklist

### Task 1: Build Metadata ✅
- [x] Create `kruxiaflow/build.rs` (based on `api/build.rs`)
- [x] Use `BUILD_*` environment variables (not `KRUXIAFLOW_*`)
- [x] Test build script captures git metadata
- [x] Verify graceful handling of missing git
- [x] Test rebuild triggers on git changes

### Task 2: Version Command ✅
- [x] Create `kruxiaflow/src/commands/version.rs`
- [x] Add `serde`/`serde_json` to dependencies
- [x] Implement `VersionCommand` with text/json
- [x] Add unit tests
- [x] Update `commands/mod.rs`
- [x] Update `main.rs` Commands enum
- [x] Skip logging for version command
- [x] Test text and JSON output

### Task 3: Enhanced Help ✅
- [x] Update `main.rs` Cli with `long_about`
- [x] Add examples to main help
- [x] Add environment variable docs
- [x] Update Commands enum with detailed help
- [x] Update `ApiCommand` with `long_help`
- [x] Update `VersionCommand` with `long_help`
- [x] Test all help outputs

### Task 4: Binary Size ✅
- [x] Create `scripts/check-binary-size.sh`
- [x] Make script executable
- [x] Test with release binary
- [x] Document in README
- [x] Verify 4MB < 15MB target ✅

### Final Verification ✅
- [x] All tests pass: `cargo test` (286 tests passed)
- [x] No warnings: `cargo build --release`
- [x] Binary size verified (4MB)
- [x] Help comprehensive
- [x] Version works correctly
- [x] Mark US-1C.1 complete

---

## Post-Epic 2 Commands

These will be added in future stories:

### `orchestrator` (US-1C.3)
```rust
#[derive(Args)]
pub struct OrchestratorCommand {
    #[arg(long, env = "KRUXIAFLOW_ORCHESTRATOR_CONSUMER_ID")]
    consumer_id: Option<String>,
}
```

### `worker` (US-1C.3)
```rust
#[derive(Args)]
pub struct WorkerCommand {
    #[arg(long, env = "KRUXIAFLOW_WORKER_ACTIVITY_TYPES")]
    activity_types: Vec<String>,

    #[arg(long, env = "KRUXIAFLOW_API_URL")]
    api_url: String,

    #[arg(long, env = "KRUXIAFLOW_WORKER_ID")]
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
