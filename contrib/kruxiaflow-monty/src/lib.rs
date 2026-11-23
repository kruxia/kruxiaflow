//! Sandboxed Python for Kruxia Flow: the `run_code` activity on
//! [Pydantic Monty](https://github.com/pydantic/monty), as an add-on worker.
//!
//! The worker polls a Kruxia Flow server for `run_code` steps (worker type
//! [`WORKER`] by default) and runs each step's code in a pooled `monty`
//! sandbox subprocess, under hard limits and with only the capabilities the
//! step grants. See `README.md` and `docs/run-code.md` in the engine
//! repository.

pub mod cli;
pub mod run_code;

pub use run_code::{AVAILABLE_MODULES, MONTY_VERSION, MontyConfig, RunCodeActivity, contract};

/// The worker type `run_code` steps name by default (`worker: monty`)
pub const WORKER: &str = "monty";
