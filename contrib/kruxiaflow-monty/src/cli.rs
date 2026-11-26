//! Command-line settings for the add-on worker. Every setting is also an
//! environment variable; the worker's connection to the server (API URL,
//! client credentials, concurrency) comes from the worker SDK's
//! `KRUXIAFLOW_*` variables.

use crate::run_code::MontyConfig;
use anyhow::{Context, Result, anyhow};
use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;
use std::time::Duration;

/// Sandboxed Python (run_code) for Kruxia Flow
#[derive(Parser, Debug)]
#[command(name = "kruxiaflow-monty", version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

    #[command(flatten)]
    pub monty: MontyArgs,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Run the worker (the default)
    Run,
    /// Print the run_code contract for these settings, as JSON, for the
    /// engine's MCP server (`--mcp-run-code-contract`)
    Contract,
}

/// The sandbox's settings: the `monty` worker binary, its process pool and
/// the limit ceilings every run is held to
#[derive(Args, Debug, Clone)]
pub struct MontyArgs {
    /// Worker type that run_code steps name
    #[arg(
        long,
        env = "KRUXIAFLOW_WORKER",
        default_value = crate::WORKER,
        help = "Worker type run_code steps name (the definition's `worker:`)"
    )]
    pub worker: String,

    /// Path to the `monty` worker binary
    #[arg(
        long,
        env = "KRUXIAFLOW_MONTY_BINARY",
        help = "Absolute path to the monty worker binary (required to run)",
        long_help = "Absolute path to the `monty` worker binary (crate monty-runtime, the\n\
same version as this build). It runs untrusted Python in pooled Monty sandbox\n\
subprocesses. The path is never looked up on PATH, and the binary's version is\n\
checked at start-up.\n\n\
Install: cargo install monty-runtime --version 1.7.0 --locked\n\
Example: --monty-binary /usr/local/bin/monty"
    )]
    pub monty_binary: Option<PathBuf>,

    /// Prewarmed Monty workers
    #[arg(
        long,
        env = "KRUXIAFLOW_MONTY_MIN_PROCESSES",
        default_value_t = 1,
        help = "Monty worker processes kept warm"
    )]
    pub monty_min_processes: usize,

    /// Upper bound on live Monty workers
    #[arg(
        long,
        env = "KRUXIAFLOW_MONTY_MAX_PROCESSES",
        help = "Most Monty worker processes alive at once (default: the worker's activity concurrency)"
    )]
    pub monty_max_processes: Option<usize>,

    /// Host-side kill switch for any single request to a Monty worker
    #[arg(
        long,
        env = "KRUXIAFLOW_MONTY_REQUEST_TIMEOUT_SECS",
        default_value_t = 30,
        help = "Seconds before a Monty worker that has not answered is killed",
        long_help = "Seconds before a Monty worker that has not answered a request is\n\
killed. A backstop behind the per-run limits: it must be at least twice\n\
--monty-max-duration-secs plus 2 seconds.\n\n\
Default: 30"
    )]
    pub monty_request_timeout_secs: u64,

    /// Checkouts per Monty worker before it is replaced
    #[arg(
        long,
        env = "KRUXIAFLOW_MONTY_RECYCLE_AFTER",
        default_value_t = 1,
        help = "Runs per Monty worker before it is replaced (1 = never reused)",
        long_help = "How many run_code runs a Monty worker serves before it is replaced.\n\
1 (the default) gives every run a fresh process, so nothing can carry over\n\
between workflows or tenants; it costs a few milliseconds per run.\n\n\
Default: 1"
    )]
    pub monty_recycle_after: u32,

    /// Memory ceiling per run, in bytes
    #[arg(
        long,
        env = "KRUXIAFLOW_MONTY_MAX_MEMORY_BYTES",
        default_value_t = 64 * 1024 * 1024,
        help = "Most memory a run may use (a workflow may ask for less, never more)"
    )]
    pub monty_max_memory_bytes: usize,

    /// Execution-time ceiling per run, in seconds
    #[arg(
        long,
        env = "KRUXIAFLOW_MONTY_MAX_DURATION_SECS",
        default_value_t = 10.0,
        help = "Most execution time (and, separately, sleep) a run may use, in seconds"
    )]
    pub monty_max_duration_secs: f64,

    /// Recursion-depth ceiling per run
    #[arg(
        long,
        env = "KRUXIAFLOW_MONTY_MAX_RECURSION_DEPTH",
        default_value_t = 1000,
        help = "Deepest recursion a run may use",
        long_help = "Deepest recursion a run may use. Higher values do not crash the\n\
sandbox but let runaway recursion run until the request timeout.\n\n\
Default: 1000"
    )]
    pub monty_max_recursion_depth: usize,
}

impl MontyArgs {
    /// The sandbox configuration. `default_max_processes` is the worker's
    /// activity concurrency. The binary is required to run, not to print the
    /// contract, which reads only the limits.
    pub fn to_config(
        &self,
        default_max_processes: usize,
        need_binary: bool,
    ) -> Result<MontyConfig> {
        let binary = self
            .monty_binary
            .clone()
            .filter(|path| !path.as_os_str().is_empty());
        let binary = match binary {
            Some(binary) => binary,
            None if need_binary => {
                return Err(anyhow!(
                    "KRUXIAFLOW_MONTY_BINARY (--monty-binary) is required: the absolute path to \
                     the `monty` worker binary (cargo install monty-runtime --version {} --locked)",
                    crate::MONTY_VERSION
                ));
            }
            None => PathBuf::new(),
        };
        let max_duration = Duration::try_from_secs_f64(self.monty_max_duration_secs)
            .ok()
            .filter(|d| !d.is_zero())
            .context("KRUXIAFLOW_MONTY_MAX_DURATION_SECS must be a positive number of seconds")?;
        Ok(MontyConfig {
            binary,
            min_processes: self.monty_min_processes,
            max_processes: self.monty_max_processes.unwrap_or(default_max_processes),
            request_timeout: Duration::from_secs(self.monty_request_timeout_secs),
            recycle_after: self.monty_recycle_after,
            max_memory_bytes: self.monty_max_memory_bytes,
            max_duration,
            max_recursion_depth: self.monty_max_recursion_depth,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Cli {
        Cli::try_parse_from(std::iter::once("kruxiaflow-monty").chain(args.iter().copied()))
            .unwrap()
    }

    #[test]
    fn test_defaults() {
        let cli = parse(&[]);
        assert!(cli.command.is_none());
        assert_eq!(cli.monty.worker, "monty");
        let config = cli.monty.to_config(4, false).unwrap();
        assert_eq!(config.max_processes, 4);
        assert_eq!(config.max_memory_bytes, 64 * 1024 * 1024);
        assert_eq!(config.max_duration, Duration::from_secs(10));
    }

    #[test]
    fn test_binary_required_to_run_not_for_the_contract() {
        let mut cli = parse(&["contract"]);
        // Unset, whatever the environment says
        cli.monty.monty_binary = None;
        assert!(matches!(cli.command, Some(Command::Contract)));
        assert!(cli.monty.to_config(4, false).is_ok());
        let err = cli.monty.to_config(4, true).unwrap_err().to_string();
        assert!(err.contains("KRUXIAFLOW_MONTY_BINARY"), "{err}");
    }

    #[test]
    fn test_invalid_duration_rejected() {
        let cli = parse(&["--monty-max-duration-secs", "0"]);
        assert!(cli.monty.to_config(4, false).is_err());
    }
}
