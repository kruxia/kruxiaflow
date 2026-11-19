//! Worker-level configuration for `run_code`: the Monty pool and the
//! ceilings every workflow's limits are checked against.

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Monty version this build is pinned to. The `monty` worker binary must
/// report the same version; the wire protocol between the pool and its
/// workers is not guaranteed across versions. Keep in step with the
/// `=x.y.z` pins in `worker/Cargo.toml`.
pub const MONTY_VERSION: &str = "1.6.0";

/// Ceiling on host calls per run (Monty's own default).
pub const MAX_SUSPENSIONS_CEILING: usize = 1000;

/// Slack added on top of the longest allowed run (execution plus sleep) when
/// checking `request_timeout`: the pool's own backstop grace is one second
/// per duration limit.
const REQUEST_TIMEOUT_SLACK: Duration = Duration::from_secs(2);

/// Configuration for the `run_code` activity and its Monty worker pool.
///
/// Built from `KRUXIAFLOW_MONTY_*` environment variables / CLI flags by the
/// `kruxiaflow` binary. A workflow may ask for lower limits than these
/// ceilings, never higher.
#[derive(Debug, Clone)]
pub struct MontyConfig {
    /// Absolute path to the `monty` worker binary (crate `monty-runtime`)
    pub binary: PathBuf,
    /// Prewarmed workers
    pub min_processes: usize,
    /// Upper bound on live workers
    pub max_processes: usize,
    /// Host-side kill switch for any single request to a worker
    pub request_timeout: Duration,
    /// Checkouts per worker before it is replaced (1 = never reused)
    pub recycle_after: u32,
    /// Ceiling on a run's `max_memory`, in bytes
    pub max_memory_bytes: usize,
    /// Ceiling on a run's execution time (`max_feed_duration_secs`). Also the
    /// ceiling on its total sleep.
    pub max_duration: Duration,
    /// Ceiling on a run's `max_recursion_depth`
    pub max_recursion_depth: usize,
}

impl MontyConfig {
    /// Validate the configuration and the worker binary.
    ///
    /// Returns the version line the binary reports (for run records). Fails
    /// with a message naming what to fix, so a misconfigured worker refuses to
    /// start instead of failing every `run_code` activity later.
    pub fn validate(&self) -> Result<String> {
        if self.max_processes == 0 {
            bail!("KRUXIAFLOW_MONTY_MAX_PROCESSES must be at least 1");
        }
        if self.min_processes > self.max_processes {
            bail!(
                "KRUXIAFLOW_MONTY_MIN_PROCESSES ({}) exceeds KRUXIAFLOW_MONTY_MAX_PROCESSES ({})",
                self.min_processes,
                self.max_processes
            );
        }
        if self.recycle_after == 0 {
            bail!("KRUXIAFLOW_MONTY_RECYCLE_AFTER must be at least 1");
        }
        if self.max_memory_bytes == 0 {
            bail!("KRUXIAFLOW_MONTY_MAX_MEMORY_BYTES must be greater than 0");
        }
        if self.max_duration.is_zero() {
            bail!("KRUXIAFLOW_MONTY_MAX_DURATION_SECS must be greater than 0");
        }
        if self.max_recursion_depth == 0 {
            bail!("KRUXIAFLOW_MONTY_MAX_RECURSION_DEPTH must be at least 1");
        }
        // A run may execute for max_duration and sleep for max_duration; the
        // kill switch must not fire before Monty's own limits do.
        let longest_run = self.max_duration * 2 + REQUEST_TIMEOUT_SLACK;
        if self.request_timeout < longest_run {
            bail!(
                "KRUXIAFLOW_MONTY_REQUEST_TIMEOUT_SECS ({}s) must be at least {}s: twice \
                 KRUXIAFLOW_MONTY_MAX_DURATION_SECS (execution plus sleep) plus {}s",
                self.request_timeout.as_secs(),
                longest_run.as_secs(),
                REQUEST_TIMEOUT_SLACK.as_secs()
            );
        }
        check_worker_binary(&self.binary)
    }
}

/// Check that `path` is an absolute path to a `monty` worker binary of the
/// pinned version. Returns its version line.
fn check_worker_binary(path: &Path) -> Result<String> {
    if !path.is_absolute() {
        bail!(
            "KRUXIAFLOW_MONTY_BINARY must be an absolute path (it is never looked up on PATH), got {}",
            path.display()
        );
    }
    if !path.is_file() {
        bail!(
            "KRUXIAFLOW_MONTY_BINARY points to {}, which does not exist. Install the worker with \
             `cargo install monty-runtime --version {MONTY_VERSION} --locked` and point this at \
             the installed `monty` binary",
            path.display()
        );
    }
    let output = std::process::Command::new(path)
        .arg("--version")
        .env_clear()
        .output()
        .with_context(|| format!("failed to run {} --version", path.display()))?;
    let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let expected = format!("monty-runtime {MONTY_VERSION}");
    if !output.status.success() || version != expected {
        bail!(
            "{} reports {:?}, but this build of Kruxia Flow needs `{expected}`. Install the \
             matching worker with `cargo install monty-runtime --version {MONTY_VERSION} --locked`",
            path.display(),
            version
        );
    }
    Ok(version)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(binary: PathBuf) -> MontyConfig {
        MontyConfig {
            binary,
            min_processes: 1,
            max_processes: 2,
            request_timeout: Duration::from_secs(30),
            recycle_after: 1,
            max_memory_bytes: 1024 * 1024,
            max_duration: Duration::from_secs(10),
            max_recursion_depth: 1000,
        }
    }

    /// A fake `monty` worker that prints `version_line` for `--version`
    #[cfg(unix)]
    fn fake_binary(version_line: &str, exit_code: i32) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("run-code-test-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("monty");
        std::fs::write(
            &path,
            format!("#!/bin/sh\necho '{version_line}'\nexit {exit_code}\n"),
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn error_of(config: &MontyConfig) -> String {
        config.validate().unwrap_err().to_string()
    }

    #[test]
    fn test_process_counts_are_validated() {
        let mut c = config(PathBuf::from("/x/monty"));
        c.max_processes = 0;
        assert!(error_of(&c).contains("MAX_PROCESSES"));
        let mut c = config(PathBuf::from("/x/monty"));
        c.min_processes = 3;
        assert!(error_of(&c).contains("MIN_PROCESSES"));
        let mut c = config(PathBuf::from("/x/monty"));
        c.recycle_after = 0;
        assert!(error_of(&c).contains("RECYCLE_AFTER"));
    }

    #[test]
    fn test_ceilings_must_be_positive() {
        let mut c = config(PathBuf::from("/x/monty"));
        c.max_memory_bytes = 0;
        assert!(error_of(&c).contains("MAX_MEMORY_BYTES"));
        let mut c = config(PathBuf::from("/x/monty"));
        c.max_duration = Duration::ZERO;
        assert!(error_of(&c).contains("MAX_DURATION_SECS"));
        let mut c = config(PathBuf::from("/x/monty"));
        c.max_recursion_depth = 0;
        assert!(error_of(&c).contains("MAX_RECURSION_DEPTH"));
    }

    #[test]
    fn test_request_timeout_must_cover_a_full_run() {
        let mut c = config(PathBuf::from("/x/monty"));
        c.max_duration = Duration::from_secs(10);
        c.request_timeout = Duration::from_secs(21); // needs 2 * 10 + 2
        let err = error_of(&c);
        assert!(err.contains("REQUEST_TIMEOUT_SECS"), "{err}");
        assert!(err.contains("at least 22s"), "{err}");
    }

    #[test]
    fn test_binary_path_must_be_absolute() {
        let err = error_of(&config(PathBuf::from("monty")));
        assert!(err.contains("absolute path"), "{err}");
    }

    #[test]
    fn test_missing_binary_says_how_to_install() {
        let err = error_of(&config(PathBuf::from("/nonexistent/dir/monty")));
        assert!(err.contains("does not exist"), "{err}");
        assert!(
            err.contains(&format!("monty-runtime --version {MONTY_VERSION}")),
            "{err}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_matching_version_is_accepted() {
        let binary = fake_binary(&format!("monty-runtime {MONTY_VERSION}"), 0);
        assert_eq!(
            config(binary).validate().unwrap(),
            format!("monty-runtime {MONTY_VERSION}")
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_other_version_is_refused() {
        let binary = fake_binary("monty-runtime 0.0.1", 0);
        let err = error_of(&config(binary));
        assert!(err.contains("reports \"monty-runtime 0.0.1\""), "{err}");
        assert!(
            err.contains(&format!("monty-runtime {MONTY_VERSION}")),
            "{err}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_failing_binary_is_refused() {
        let binary = fake_binary(&format!("monty-runtime {MONTY_VERSION}"), 1);
        assert!(config(binary).validate().is_err());
    }
}
