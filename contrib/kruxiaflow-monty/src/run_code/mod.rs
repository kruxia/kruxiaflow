//! `run_code`: run untrusted, model-written Python in a Monty sandbox.
//!
//! Code runs in pooled `monty` worker subprocesses (crate `monty-pool`), so a
//! crash in the interpreter kills a worker, never this process. The sandbox
//! has no filesystem, network or environment; it reaches the host only
//! through the capabilities a workflow grants (see [`capabilities`]).
//!
//! Every outcome the code itself causes — success, a type or runtime error,
//! a tripped limit, a host-side kill for running too long — *completes* the
//! activity with `result.ok` true or false, so a later step (usually the LLM
//! that wrote the code) can read the error and repair it in a bounded loop.
//! Only infrastructure failures fail the activity and go through retries.
//!
//! Isolation note: `run_code` is the sandboxed code activity. The py-std
//! worker's `script` activity is NOT sandboxed and must not run model-written
//! code.

mod capabilities;
mod config;
mod convert;

pub use config::{MONTY_VERSION, MontyConfig};

use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use capabilities::{CallContext, CallState, CapabilityRegistry, STUB_HEADER};
use chrono::{DateTime, Utc};
use config::MAX_SUSPENSIONS_CEILING;
use convert::{json_to_monty, monty_to_json};
use kruxiaflow_worker::{ActivityContext, ActivityError, ActivityImpl, ActivityResult};
use monty_pool::{Pool, PoolConfig, PoolError, ReplConfig, ResumeValue, TurnEvent, on_print_sync};
use monty_types::{
    DateTimeSource, ExcType, IMPORT_CALL, MontyException, NameLookupResult, NamedValues, OsPolicy,
    RandomSeed, RandomStart, ResourceLimits,
};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};
use tokio::sync::OnceCell;

/// Captured `print()` output kept per run
const MAX_STDOUT_BYTES: usize = 64 * 1024;
/// Traceback lines kept in an error message (the last ones: the frames
/// nearest the error and the exception line)
const MAX_ERROR_LINES: usize = 30;
/// Characters kept in an error message
const MAX_ERROR_CHARS: usize = 4000;
/// Largest JSON result returned, in bytes
const MAX_VALUE_BYTES: usize = 1024 * 1024;

/// Standard-library modules built into Monty 1.6 that `run_code` documents
/// as available (each verified to import on 1.6.0). Imports of anything else
/// suspend to the host and are refused with `ModuleNotFoundError`.
pub const AVAILABLE_MODULES: &[&str] = &[
    "json",
    "re",
    "datetime",
    "decimal",
    "csv",
    "math",
    "collections",
    "itertools",
    "functools",
    "hashlib",
    "base64",
    "random",
    "tomllib",
    "typing",
    "dataclasses",
];

/// Python keywords: not usable as input names
const PYTHON_KEYWORDS: &[&str] = &[
    "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class", "continue",
    "def", "del", "elif", "else", "except", "finally", "for", "from", "global", "if", "import",
    "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise", "return", "try", "while",
    "with", "yield",
];

/// `run_code` parameters. Unknown keys are rejected, so a misplaced limit is
/// reported at once. (Activity settings accept any key.)
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RunCodeParams {
    /// Python source. A single surrounding Markdown code fence is stripped,
    /// so an LLM step's `result.content` can be passed straight through.
    code: String,
    /// Exposed to the code as globals, one per key
    #[serde(default)]
    inputs: Map<String, Value>,
    /// Names from the worker's capability registry the code may call
    #[serde(default)]
    capabilities: Vec<String>,
    /// Type-check the code against the inputs and capability stubs first
    #[serde(default = "default_true")]
    type_check: bool,
    /// Per-run limits, each at most the worker's ceiling
    #[serde(default)]
    limits: LimitParams,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct LimitParams {
    max_memory: Option<u64>,
    max_feed_duration_secs: Option<f64>,
    max_recursion_depth: Option<usize>,
    max_suspensions: Option<usize>,
    max_total_sleep_secs: Option<f64>,
}

/// The limits one run executes under, after defaults (a missing limit takes
/// the worker's ceiling)
#[derive(Debug, Clone)]
struct EffectiveLimits {
    max_memory: usize,
    max_feed_duration: Duration,
    max_recursion_depth: usize,
    max_suspensions: usize,
    max_total_sleep: Duration,
    request_timeout: Duration,
}

impl EffectiveLimits {
    fn resolve(requested: &LimitParams, config: &MontyConfig) -> Result<Self> {
        let max_memory = match requested.max_memory {
            None => config.max_memory_bytes,
            Some(0) => bail!("limits.max_memory must be greater than 0"),
            Some(m) => {
                let m = usize::try_from(m).unwrap_or(usize::MAX);
                if m > config.max_memory_bytes {
                    bail!(
                        "limits.max_memory ({m}) exceeds this worker's ceiling ({}, \
                         KRUXIAFLOW_MONTY_MAX_MEMORY_BYTES)",
                        config.max_memory_bytes
                    );
                }
                m
            }
        };
        let max_recursion_depth = match requested.max_recursion_depth {
            None => config.max_recursion_depth,
            Some(0) => bail!("limits.max_recursion_depth must be at least 1"),
            Some(d) if d > config.max_recursion_depth => bail!(
                "limits.max_recursion_depth ({d}) exceeds this worker's ceiling ({}, \
                 KRUXIAFLOW_MONTY_MAX_RECURSION_DEPTH)",
                config.max_recursion_depth
            ),
            Some(d) => d,
        };
        let max_suspensions = match requested.max_suspensions {
            None => MAX_SUSPENSIONS_CEILING,
            Some(s) if s > MAX_SUSPENSIONS_CEILING => bail!(
                "limits.max_suspensions ({s}) exceeds the ceiling ({MAX_SUSPENSIONS_CEILING})"
            ),
            Some(s) => s,
        };
        Ok(Self {
            max_memory,
            max_feed_duration: duration_limit(
                "max_feed_duration_secs",
                requested.max_feed_duration_secs,
                config.max_duration,
            )?,
            max_recursion_depth,
            max_suspensions,
            max_total_sleep: duration_limit(
                "max_total_sleep_secs",
                requested.max_total_sleep_secs,
                config.max_duration,
            )?,
            request_timeout: config.request_timeout,
        })
    }

    fn to_monty(&self) -> ResourceLimits {
        ResourceLimits {
            max_memory: Some(self.max_memory),
            max_feed_duration: Some(self.max_feed_duration),
            max_recursion_depth: self.max_recursion_depth,
            max_suspensions: self.max_suspensions,
            max_total_sleep: Some(self.max_total_sleep),
            ..ResourceLimits::default()
        }
    }

    fn to_json(&self) -> Value {
        json!({
            "max_memory": self.max_memory,
            "max_feed_duration_secs": self.max_feed_duration.as_secs_f64(),
            "max_recursion_depth": self.max_recursion_depth,
            "max_suspensions": self.max_suspensions,
            "max_total_sleep_secs": self.max_total_sleep.as_secs_f64(),
            "request_timeout_secs": self.request_timeout.as_secs_f64(),
        })
    }
}

fn duration_limit(name: &str, requested: Option<f64>, ceiling: Duration) -> Result<Duration> {
    let Some(secs) = requested else {
        return Ok(ceiling);
    };
    let duration = Duration::try_from_secs_f64(secs)
        .ok()
        .filter(|d| !d.is_zero())
        .ok_or_else(|| anyhow!("limits.{name} must be a positive number of seconds"))?;
    if duration > ceiling {
        bail!(
            "limits.{name} ({secs}) exceeds this worker's ceiling ({}s, \
             KRUXIAFLOW_MONTY_MAX_DURATION_SECS)",
            ceiling.as_secs_f64()
        );
    }
    Ok(duration)
}

/// How one run of the code ended
#[derive(Debug)]
enum Outcome {
    /// The code finished; its value as JSON
    Value(Value),
    /// The code failed in a way the code's author must fix
    Error {
        kind: &'static str,
        message: String,
        limit: Option<&'static str>,
    },
    /// The Monty worker crashed; retried once on a fresh worker
    Crashed(String),
}

impl Outcome {
    fn error(kind: &'static str, message: impl Into<String>) -> Self {
        Self::Error {
            kind,
            message: message.into(),
            limit: None,
        }
    }

    fn limit(limit: &'static str, message: impl Into<String>) -> Self {
        Self::Error {
            kind: "limit",
            message: message.into(),
            limit: Some(limit),
        }
    }
}

/// Everything one attempt produced
struct RunReport {
    outcome: Outcome,
    stdout: CappedText,
    state: CallState,
    calls: BTreeMap<String, u32>,
    host_calls: usize,
    execution: Duration,
}

/// `print()` output, kept up to a byte cap
#[derive(Debug, Default)]
struct CappedText {
    text: String,
    truncated: bool,
}

impl CappedText {
    fn push(&mut self, fragment: &str) {
        // Once something was dropped, keep nothing after it: a later fragment
        // squeezed into the remaining room would hide the gap
        if self.truncated {
            return;
        }
        let room = MAX_STDOUT_BYTES.saturating_sub(self.text.len());
        if fragment.len() <= room {
            self.text.push_str(fragment);
            return;
        }
        let mut end = room;
        while !fragment.is_char_boundary(end) {
            end -= 1;
        }
        self.text.push_str(&fragment[..end]);
        self.truncated = true;
    }
}

/// The `run_code` activity
pub struct RunCodeActivity {
    config: MontyConfig,
    /// The worker type `run_code` steps name
    worker: String,
    /// Version line the worker binary reported at start-up
    monty_version: String,
    /// Started on first use (pool construction is async)
    pool: OnceCell<Pool>,
    capabilities: CapabilityRegistry,
}

impl RunCodeActivity {
    /// Create the activity, validating the configuration and the `monty`
    /// worker binary. Fails with an actionable message when either is wrong,
    /// so a misconfigured worker refuses to start.
    pub async fn new(config: MontyConfig, worker: &str) -> Result<Self> {
        let monty_version = config.validate().await?;
        Ok(Self {
            config,
            worker: worker.to_string(),
            monty_version,
            pool: OnceCell::new(),
            capabilities: CapabilityRegistry::starter(),
        })
    }

    /// Names of the capabilities this worker offers
    pub fn capability_names(&self) -> Vec<&'static str> {
        self.capabilities.names()
    }

    async fn pool(&self) -> Result<&Pool> {
        self.pool
            .get_or_try_init(|| async {
                let mut pool_config = PoolConfig::subprocess(self.config.binary.clone());
                pool_config.min_processes = self.config.min_processes;
                pool_config.max_processes = self.config.max_processes;
                pool_config.request_timeout = Some(self.config.request_timeout);
                pool_config.max_checkouts_per_worker = Some(self.config.recycle_after);
                let pool = Pool::new(pool_config)
                    .await
                    .map_err(|e| anyhow!("failed to start the Monty worker pool: {e}"))?;
                tracing::info!(
                    binary = %self.config.binary.display(),
                    min_processes = self.config.min_processes,
                    max_processes = self.config.max_processes,
                    "Monty worker pool started"
                );
                Ok::<_, anyhow::Error>(pool)
            })
            .await
    }

    /// Run the code once on a fresh checkout
    #[allow(clippy::too_many_arguments)]
    async fn run_once(
        &self,
        pool: &Pool,
        repl: &ReplConfig,
        code: &str,
        inputs: &Map<String, Value>,
        granted: &[String],
        limits: &EffectiveLimits,
        started_at: DateTime<Utc>,
        ctx: &ActivityContext,
    ) -> Result<RunReport> {
        let mut checkout = pool
            .checkout(repl)
            .await
            .map_err(|e| anyhow!("could not get a Monty worker: {e}"))?;

        let mut state = CallState::new(started_at);
        let mut calls: BTreeMap<String, u32> = BTreeMap::new();
        let mut host_calls = 0usize;
        let mut stdout = CappedText::default();
        let named: Vec<(String, _)> = inputs
            .iter()
            .map(|(name, value)| (name.clone(), json_to_monty(value)))
            .collect();
        let clock = Instant::now();

        let result = {
            let mut on_print = on_print_sync(|_stream, text| stdout.push(text));
            let mut event = checkout
                .feed(
                    code,
                    NamedValues::from(named),
                    Vec::new(),
                    false,
                    &mut on_print,
                )
                .await;
            loop {
                let current = match event {
                    Ok(current) => current,
                    Err(error) => break Err(error),
                };
                event = match current {
                    TurnEvent::Complete(value) => break Ok(value),
                    TurnEvent::FunctionCall {
                        function_name,
                        args,
                        object_id,
                        ..
                    } => {
                        host_calls += 1;
                        let answer = if object_id.is_some() {
                            Err(MontyException::new(
                                ExcType::AttributeError,
                                Some("host objects are not available in run_code".to_string()),
                            ))
                        } else if let Some(capability) = granted
                            .iter()
                            .any(|g| g == &function_name)
                            .then(|| self.capabilities.get(&function_name))
                            .flatten()
                        {
                            *calls.entry(function_name.clone()).or_default() += 1;
                            let mut cx = CallContext {
                                state: &mut state,
                                activity: ctx,
                            };
                            capability.call(&args, &mut cx).await
                        } else {
                            Err(not_granted(&function_name, granted))
                        };
                        let reply = match answer {
                            Ok(value) => ResumeValue::Return(value),
                            Err(exception) => ResumeValue::Error(exception),
                        };
                        checkout.resume(reply, &mut on_print).await
                    }
                    TurnEvent::OsCall {
                        function_name,
                        args,
                        ..
                    } => {
                        // Built-in modules import without asking the host, so
                        // every import that reaches us is for a module Monty
                        // does not have. Everything else (files, environment,
                        // type-checking stubs) is not handled: no mounts here.
                        let reply = if function_name == IMPORT_CALL {
                            let module = args
                                .arg(0)
                                .and_then(|m| m.as_str().map(str::to_string))
                                .unwrap_or_default();
                            ResumeValue::Error(MontyException::new(
                                ExcType::ModuleNotFoundError,
                                Some(format!(
                                    "No module named '{module}'. run_code provides only these \
                                     standard-library modules: {}",
                                    AVAILABLE_MODULES.join(", ")
                                )),
                            ))
                        } else {
                            ResumeValue::NotHandled
                        };
                        checkout.resume(reply, &mut on_print).await
                    }
                    TurnEvent::NameLookup { .. } => {
                        checkout
                            .resume_name_lookup(NameLookupResult::Undefined, &mut on_print)
                            .await
                    }
                    TurnEvent::ResolveFutures { .. } => {
                        // No capability returns a future, so this only happens
                        // if the protocol changes under us.
                        break Err(PoolError::Runtime(MontyException::new(
                            ExcType::RuntimeError,
                            Some("async host calls are not supported by run_code".to_string()),
                        )));
                    }
                };
            }
        };
        let execution = clock.elapsed();

        let outcome = match result {
            Ok(value) => {
                if let Err(error) = checkout.finish().await {
                    tracing::warn!(%error, "Monty checkout did not finish cleanly");
                }
                match monty_to_json(value.as_ref()) {
                    Ok(json) => Outcome::Value(json),
                    Err(message) => Outcome::error("output", message),
                }
            }
            Err(error) => {
                // Any error discards the worker: Monty makes no guarantees
                // about a session's heap after a tripped limit.
                let outcome = classify(error, host_calls, limits);
                checkout.terminate().await;
                match outcome {
                    Ok(outcome) => outcome,
                    Err(infra) => return Err(infra),
                }
            }
        };

        Ok(RunReport {
            outcome,
            stdout,
            state,
            calls,
            host_calls,
            execution,
        })
    }
}

/// Sort a pool error into an outcome the code caused, or an infrastructure
/// failure (`Err`) the workflow's retry policy should handle
fn classify(error: PoolError, host_calls: usize, limits: &EffectiveLimits) -> Result<Outcome> {
    Ok(match error {
        PoolError::Typing(diagnostics) => Outcome::error("type", trim_message(&diagnostics)),
        PoolError::Runtime(exception) => {
            let message = trim_message(&exception.to_string());
            match exception.exc_type() {
                ExcType::MemoryError => Outcome::limit("max_memory", message),
                ExcType::TimeoutError => Outcome::limit("max_feed_duration", message),
                ExcType::RecursionError => Outcome::limit("max_recursion_depth", message),
                ExcType::RuntimeError if host_calls >= limits.max_suspensions => {
                    Outcome::limit("max_suspensions", message)
                }
                _ => Outcome::error("runtime", message),
            }
        }
        // Host-side kills caused by the code (an infinite loop, say): a retry
        // would run into the same wall, so they go back to the code's author.
        PoolError::Timeout { .. } => Outcome::limit(
            "request_timeout",
            format!(
                "{error}. The code ran past the worker's time limit without finishing; look \
                 for a loop that never ends"
            ),
        ),
        PoolError::CpuLimit { .. } => Outcome::limit("cpu", error.to_string()),
        PoolError::Crashed { .. } => Outcome::Crashed(error.to_string()),
        other => return Err(anyhow!("Monty worker failure: {other}")),
    })
}

/// The `NameError` for a call to a name the step was not granted
fn not_granted(name: &str, granted: &[String]) -> MontyException {
    let allowed = if granted.is_empty() {
        "this step was granted no capabilities".to_string()
    } else {
        format!("this step may call only: {}", granted.join(", "))
    };
    MontyException::new(
        ExcType::NameError,
        Some(format!("name '{name}' is not defined ({allowed})")),
    )
}

/// Keep the end of a long message: the frames nearest the error and the
/// exception line matter most to whoever repairs the code
fn trim_message(message: &str) -> String {
    let lines: Vec<&str> = message.lines().collect();
    let kept = if lines.len() > MAX_ERROR_LINES {
        let mut kept = vec!["..."];
        kept.extend_from_slice(&lines[lines.len() - MAX_ERROR_LINES..]);
        kept.join("\n")
    } else {
        message.trim_end().to_string()
    };
    let count = kept.chars().count();
    if count > MAX_ERROR_CHARS {
        let tail: String = kept.chars().skip(count - MAX_ERROR_CHARS).collect();
        format!("...{tail}")
    } else {
        kept
    }
}

/// Strip a single Markdown code fence around the whole text (```` ```python
/// … ``` ````), the usual shape of code in an LLM reply. Anything else is
/// returned unchanged.
fn strip_fence(code: &str) -> String {
    let trimmed = code.trim();
    if let Some(body) = trimmed
        .strip_prefix("```")
        .and_then(|rest| rest.strip_suffix("```"))
        && let Some(newline) = body.find('\n')
    {
        let info = &body[..newline];
        // The info string is a language tag, never code
        if !info.contains(char::is_whitespace) || info.trim().is_empty() {
            return body[newline + 1..].trim_end().to_string();
        }
    }
    code.to_string()
}

fn validate_input_name(name: &str) -> Result<()> {
    let mut chars = name.chars();
    let valid_start = chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_');
    if !valid_start
        || !chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        || PYTHON_KEYWORDS.contains(&name)
    {
        bail!("input name {name:?} is not a valid Python identifier");
    }
    Ok(())
}

/// The stub file the code is type-checked against: one `name: Any` per
/// input, then each granted capability's declaration
fn build_stubs<'a>(
    inputs: impl Iterator<Item = &'a String>,
    granted: &[String],
    registry: &CapabilityRegistry,
) -> String {
    let mut stubs = String::from(STUB_HEADER);
    for name in inputs {
        stubs.push_str(&format!("{name}: Any\n"));
    }
    for name in granted {
        if let Some(capability) = registry.get(name) {
            stubs.push('\n');
            stubs.push_str(capability.stub());
        }
    }
    stubs
}

/// A step's parameters, checked against this worker's configuration
struct Checked {
    params: RunCodeParams,
    limits: EffectiveLimits,
    granted: Vec<String>,
    code: String,
    code_sha256: String,
}

impl RunCodeActivity {
    /// Check a step's parameters. An error here is the workflow's mistake
    /// (wrong parameters, a limit above the ceiling, an unknown capability),
    /// so it fails the step without a retry.
    fn check(&self, parameters: Value) -> Result<Checked> {
        let params: RunCodeParams =
            serde_json::from_value(parameters).context("invalid run_code parameters")?;
        let limits = EffectiveLimits::resolve(&params.limits, &self.config)?;

        let mut granted: Vec<String> = Vec::new();
        for name in &params.capabilities {
            if self.capabilities.get(name).is_none() {
                bail!(
                    "unknown capability {name:?}; this worker offers: {}",
                    self.capabilities.names().join(", ")
                );
            }
            if !granted.contains(name) {
                granted.push(name.clone());
            }
        }
        for name in params.inputs.keys() {
            validate_input_name(name)?;
            if granted.contains(name) {
                bail!("input {name:?} has the same name as a granted capability");
            }
        }
        let code = strip_fence(&params.code);
        if code.trim().is_empty() {
            bail!("run_code parameter 'code' is empty");
        }
        let code_sha256 = format!("{:x}", Sha256::digest(code.as_bytes()));

        Ok(Checked {
            params,
            limits,
            granted,
            code,
            code_sha256,
        })
    }

    async fn run(&self, checked: Checked, ctx: &ActivityContext) -> Result<ActivityResult> {
        let started_at = Utc::now();
        let Checked {
            params,
            limits,
            granted,
            code,
            code_sha256,
        } = checked;

        // --- Session: limits, frozen clock, seeded random, stubs ---
        let os_policy = OsPolicy {
            datetime: DateTimeSource::Fixed {
                unix_seconds: started_at.timestamp(),
                microsecond: started_at.timestamp_subsec_micros().min(999_999),
            },
            random_start: RandomStart::Seed(RandomSeed::Str(format!(
                "{}/{}",
                ctx.workflow_id, ctx.activity_key
            ))),
            ..OsPolicy::default()
        };
        let repl = ReplConfig {
            limits: Some(limits.to_monty()),
            type_check: params.type_check,
            type_check_stubs: params
                .type_check
                .then(|| build_stubs(params.inputs.keys(), &granted, &self.capabilities)),
            os_policy,
            ..ReplConfig::default()
        };

        // --- Run: a crash is retried once on a fresh worker ---
        let pool = self.pool().await?;
        let mut worker_crashes = 0u32;
        let report = loop {
            let report = self
                .run_once(
                    pool,
                    &repl,
                    &code,
                    &params.inputs,
                    &granted,
                    &limits,
                    started_at,
                    ctx,
                )
                .await?;
            match &report.outcome {
                Outcome::Crashed(message) if worker_crashes == 0 => {
                    worker_crashes += 1;
                    tracing::warn!(
                        workflow_id = %ctx.workflow_id,
                        activity_key = %ctx.activity_key,
                        %message,
                        "Monty worker crashed; retrying once on a fresh worker"
                    );
                }
                Outcome::Crashed(_) => {
                    worker_crashes += 1;
                    break report;
                }
                _ => break report,
            }
        };

        let output = build_output(
            report,
            &limits,
            &RunFacts {
                granted: &granted,
                type_check: params.type_check,
                worker_crashes,
                code_sha256: &code_sha256,
                monty_version: &self.monty_version,
            },
        );
        Ok(ActivityResult::value("result", output))
    }
}

#[async_trait]
impl ActivityImpl for RunCodeActivity {
    async fn execute(
        &self,
        parameters: Value,
        ctx: &ActivityContext,
    ) -> std::result::Result<ActivityResult, ActivityError> {
        let checked = self
            .check(parameters)
            .map_err(|e| ActivityError::terminal("INVALID_PARAMETERS", format!("{e:#}")))?;
        // Every outcome the code causes completes the step; an error here is
        // the sandbox's (no worker, a worker failure), which a retry may fix
        self.run(checked, ctx)
            .await
            .map_err(|e| ActivityError::retryable("SANDBOX_UNAVAILABLE", format!("{e:#}")))
    }

    fn name(&self) -> &str {
        "run_code"
    }

    fn worker(&self) -> &str {
        &self.worker
    }
}

/// Facts about a run that go into its run record besides the report itself
struct RunFacts<'a> {
    granted: &'a [String],
    type_check: bool,
    worker_crashes: u32,
    code_sha256: &'a str,
    monty_version: &'a str,
}

/// The activity's `result` output: `ok`, `value` or `error`, `stdout`,
/// `logs`, and the run record (`run`)
fn build_output(report: RunReport, limits: &EffectiveLimits, facts: &RunFacts<'_>) -> Value {
    let RunReport {
        outcome,
        stdout,
        state,
        calls,
        host_calls,
        execution,
    } = report;

    // fail() is sticky: catching its exception does not undo it
    let outcome = match (state.failure.clone(), outcome) {
        (Some(message), _) => Outcome::error("fail", message),
        (None, Outcome::Crashed(message)) => Outcome::error(
            "crash",
            format!(
                "{message}. The Monty worker crashed twice running this code; simplify it \
                     (deep recursion and very large values are common causes)"
            ),
        ),
        (None, Outcome::Value(value)) => match serde_json::to_vec(&value) {
            Ok(bytes) if bytes.len() > MAX_VALUE_BYTES => Outcome::error(
                "output",
                format!(
                    "the result is {} bytes; the limit is {MAX_VALUE_BYTES}. Return a \
                         smaller value",
                    bytes.len()
                ),
            ),
            _ => Outcome::Value(value),
        },
        (None, other) => other,
    };

    let mut output = json!({
        "ok": matches!(outcome, Outcome::Value(_)),
        "stdout": stdout.text,
        "logs": state.logs,
        "run": {
            "limits": limits.to_json(),
            "consumed": {
                "execution_ms": execution.as_secs_f64() * 1000.0,
                "host_calls": host_calls,
            },
            "capabilities": {
                "granted": facts.granted,
                "called": calls,
            },
            "type_check": facts.type_check,
            "worker_crashes": facts.worker_crashes,
            "stdout_truncated": stdout.truncated,
            "logs_truncated": state.logs_truncated,
            "monty_version": facts.monty_version,
            "code_sha256": facts.code_sha256,
        },
    });
    match outcome {
        Outcome::Value(value) => output["value"] = value,
        Outcome::Error {
            kind,
            message,
            limit,
        } => {
            output["error"] = json!({ "kind": kind, "message": message });
            if let Some(limit) = limit {
                output["error"]["limit"] = json!(limit);
            }
        }
        // Converted to a "crash" error above; kept total without a panic
        Outcome::Crashed(message) => {
            output["error"] = json!({ "kind": "crash", "message": message });
        }
    }
    output
}

/// What a workflow author (often an agent, through the MCP server) needs to
/// write `run_code` steps for this worker: the capabilities with their
/// Python signatures, the available modules, the limit ceilings, the result
/// shape and the rules the code must follow. Built from the same registry
/// and constants the activity uses, so it cannot drift from the sandbox.
pub fn contract(config: &MontyConfig, worker: &str) -> Value {
    let registry = CapabilityRegistry::starter();
    let capabilities: Vec<Value> = registry
        .names()
        .into_iter()
        .filter_map(|name| registry.get(name).map(|c| (name, c.stub())))
        .map(|(name, stub)| json!({ "name": name, "stub": stub.trim_end() }))
        .collect();
    json!({
        "activity": format!("{worker}.run_code"),
        "worker": worker,
        "monty_version": MONTY_VERSION,
        "capabilities": capabilities,
        "available_modules": AVAILABLE_MODULES,
        "limit_ceilings": {
            "max_memory": config.max_memory_bytes,
            "max_feed_duration_secs": config.max_duration.as_secs_f64(),
            "max_total_sleep_secs": config.max_duration.as_secs_f64(),
            "max_recursion_depth": config.max_recursion_depth,
            "max_suspensions": MAX_SUSPENSIONS_CEILING,
            "request_timeout_secs": config.request_timeout.as_secs_f64(),
        },
        "rules": [
            "The value of the code's last line, if it is an expression, is the result; it must be JSON-compatible (None, bool, int, float, str, list, tuple, dict, Decimal)",
            "Each key of `inputs` becomes a global variable; inputs must be valid Python identifiers",
            "Only the listed capabilities can be called, and only those granted in the step's `capabilities` parameter",
            "Only the listed standard-library modules can be imported; there are no third-party packages",
            "No filesystem, network or environment access",
            "A single surrounding Markdown code fence is stripped, so an llm_prompt step's result.content can be passed straight in as `code`",
            "Limits in `limits` may be lower than the ceilings, never higher; unknown parameters are rejected",
            "The Python subset has no generators (yield), del, @property, or user-defined exception classes",
        ],
        "result": {
            "ok": "true when the code finished and returned a JSON-compatible value",
            "value": "the code's result (when ok)",
            "error": "{kind, message, limit?} when not ok; message is written to be fed back to the model that wrote the code",
            "stdout": "captured print() output (up to 64 KB)",
            "logs": "messages from log()",
            "run": "run record: effective limits, execution_ms, host_calls, capabilities granted and called, monty_version, code_sha256",
        },
        "error_kinds": {
            "type": "type check failed before running (wrong call, undefined name)",
            "runtime": "exception or syntax error, including a call to an ungranted capability (NameError) or an unavailable module (ModuleNotFoundError)",
            "limit": "a limit tripped; error.limit is max_memory, max_feed_duration, max_recursion_depth, max_suspensions, request_timeout or cpu",
            "output": "the result has no JSON form or is over 1 MB",
            "fail": "the code called fail(message)",
            "crash": "the sandbox worker crashed twice on this code",
        },
        "outcome_semantics": "Every outcome the code causes COMPLETES the step with result.ok true or false, so later steps can branch on {{step.result.ok}}. Only infrastructure problems and invalid parameters fail the activity.",
        "repair_loop": "To let a model fix its own code, give the writing step a back-edge on the run_code step with condition {{ run.result.ok == false }} and an iteration_limit, and guard the error reference in its prompt with {% if run.status == 'completed' %} (not `is defined`, which is always true).",
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use monty_pool::CrashCause;
    use std::path::PathBuf;

    fn config() -> MontyConfig {
        MontyConfig {
            binary: PathBuf::from("/nonexistent/monty"),
            min_processes: 1,
            max_processes: 4,
            request_timeout: Duration::from_secs(30),
            recycle_after: 1,
            max_memory_bytes: 64 * 1024 * 1024,
            max_duration: Duration::from_secs(10),
            max_recursion_depth: 1000,
        }
    }

    /// The contract the engine's MCP server reads: it names the worker and
    /// the capabilities (the engine refuses a file without them), and its
    /// ceilings and modules are this worker's own
    #[test]
    fn test_contract_matches_the_worker() {
        let mut config = config();
        config.max_memory_bytes = 12_345_678;
        config.max_duration = Duration::from_secs(4);
        config.max_recursion_depth = 500;
        let contract = contract(&config, "sandbox");

        assert_eq!(contract["worker"], "sandbox");
        assert_eq!(contract["activity"], "sandbox.run_code");
        assert_eq!(contract["limit_ceilings"]["max_memory"], 12_345_678);
        assert_eq!(contract["limit_ceilings"]["max_feed_duration_secs"], 4.0);
        assert_eq!(contract["limit_ceilings"]["max_recursion_depth"], 500);
        let modules = contract["available_modules"].as_array().unwrap();
        assert_eq!(modules.len(), AVAILABLE_MODULES.len());
        let capabilities = contract["capabilities"].as_array().unwrap();
        let names: Vec<&str> = capabilities
            .iter()
            .map(|c| c["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["fail", "json_path", "log", "now"]);
        for cap in capabilities {
            let name = cap["name"].as_str().unwrap();
            assert!(
                cap["stub"]
                    .as_str()
                    .unwrap()
                    .starts_with(&format!("def {name}("))
            );
        }
    }

    fn limits() -> EffectiveLimits {
        EffectiveLimits::resolve(&LimitParams::default(), &config()).unwrap()
    }

    fn params(value: Value) -> Result<RunCodeParams, serde_json::Error> {
        serde_json::from_value(value)
    }

    fn report(outcome: Outcome) -> RunReport {
        RunReport {
            outcome,
            stdout: CappedText::default(),
            state: CallState::new(Utc::now()),
            calls: BTreeMap::new(),
            host_calls: 0,
            execution: Duration::from_millis(12),
        }
    }

    fn facts() -> RunFacts<'static> {
        RunFacts {
            granted: &[],
            type_check: true,
            worker_crashes: 0,
            code_sha256: "abc",
            monty_version: "monty-runtime 1.6.0",
        }
    }

    // --- parameters ---

    #[test]
    fn test_params_defaults() {
        let p = params(json!({"code": "1 + 1"})).unwrap();
        assert_eq!(p.code, "1 + 1");
        assert!(p.inputs.is_empty());
        assert!(p.capabilities.is_empty());
        assert!(p.type_check);
        assert!(p.limits.max_memory.is_none());
    }

    #[test]
    fn test_params_reject_unknown_keys() {
        assert!(params(json!({"code": "1", "timeout": 5})).is_err());
        assert!(params(json!({"code": "1", "limits": {"max_mem": 5}})).is_err());
        assert!(params(json!({"inputs": {}})).is_err(), "code is required");
    }

    // --- limits ---

    #[test]
    fn test_limits_default_to_the_ceilings() {
        let l = limits();
        assert_eq!(l.max_memory, 64 * 1024 * 1024);
        assert_eq!(l.max_feed_duration, Duration::from_secs(10));
        assert_eq!(l.max_total_sleep, Duration::from_secs(10));
        assert_eq!(l.max_recursion_depth, 1000);
        assert_eq!(l.max_suspensions, MAX_SUSPENSIONS_CEILING);
        assert_eq!(l.request_timeout, Duration::from_secs(30));
    }

    #[test]
    fn test_limits_accept_values_under_the_ceilings() {
        let requested = LimitParams {
            max_memory: Some(1_000_000),
            max_feed_duration_secs: Some(0.5),
            max_recursion_depth: Some(200),
            max_suspensions: Some(10),
            max_total_sleep_secs: Some(1.0),
        };
        let l = EffectiveLimits::resolve(&requested, &config()).unwrap();
        assert_eq!(l.max_memory, 1_000_000);
        assert_eq!(l.max_feed_duration, Duration::from_millis(500));
        assert_eq!(l.max_recursion_depth, 200);
        assert_eq!(l.max_suspensions, 10);
        assert_eq!(l.max_total_sleep, Duration::from_secs(1));
        let monty = l.to_monty();
        assert_eq!(monty.max_memory, Some(1_000_000));
        assert_eq!(monty.max_feed_duration, Some(Duration::from_millis(500)));
        assert_eq!(monty.max_recursion_depth, 200);
        assert_eq!(monty.max_suspensions, 10);
        assert_eq!(monty.max_total_sleep, Some(Duration::from_secs(1)));
    }

    #[test]
    fn test_limits_above_a_ceiling_are_errors_naming_the_setting() {
        let cases = [
            (
                LimitParams {
                    max_memory: Some(u64::MAX),
                    ..Default::default()
                },
                "KRUXIAFLOW_MONTY_MAX_MEMORY_BYTES",
            ),
            (
                LimitParams {
                    max_recursion_depth: Some(1001),
                    ..Default::default()
                },
                "KRUXIAFLOW_MONTY_MAX_RECURSION_DEPTH",
            ),
            (
                LimitParams {
                    max_feed_duration_secs: Some(10.5),
                    ..Default::default()
                },
                "KRUXIAFLOW_MONTY_MAX_DURATION_SECS",
            ),
            (
                LimitParams {
                    max_suspensions: Some(MAX_SUSPENSIONS_CEILING + 1),
                    ..Default::default()
                },
                "ceiling",
            ),
        ];
        for (requested, needle) in cases {
            let err = EffectiveLimits::resolve(&requested, &config())
                .unwrap_err()
                .to_string();
            assert!(err.contains(needle), "{err}");
        }
    }

    #[test]
    fn test_limits_reject_zero_negative_and_non_finite() {
        for requested in [
            LimitParams {
                max_memory: Some(0),
                ..Default::default()
            },
            LimitParams {
                max_recursion_depth: Some(0),
                ..Default::default()
            },
            LimitParams {
                max_feed_duration_secs: Some(0.0),
                ..Default::default()
            },
            LimitParams {
                max_feed_duration_secs: Some(-1.0),
                ..Default::default()
            },
            LimitParams {
                max_total_sleep_secs: Some(f64::NAN),
                ..Default::default()
            },
        ] {
            assert!(EffectiveLimits::resolve(&requested, &config()).is_err());
        }
    }

    #[test]
    fn test_limits_json_names() {
        let json = limits().to_json();
        for key in [
            "max_memory",
            "max_feed_duration_secs",
            "max_recursion_depth",
            "max_suspensions",
            "max_total_sleep_secs",
            "request_timeout_secs",
        ] {
            assert!(json.get(key).is_some(), "missing {key}");
        }
    }

    // --- code and names ---

    #[test]
    fn test_strip_fence() {
        assert_eq!(strip_fence("```python\nx = 1\nx\n```"), "x = 1\nx");
        assert_eq!(strip_fence("  ```py\nx\n```  \n"), "x");
        assert_eq!(strip_fence("```\nx\n```"), "x");
        // Not a single surrounding fence: unchanged
        assert_eq!(strip_fence("x = 1"), "x = 1");
        assert_eq!(
            strip_fence("Here:\n```python\nx\n```"),
            "Here:\n```python\nx\n```"
        );
        // An info string with spaces is prose, not a language tag
        let prose = "```this is not a tag\nx\n```";
        assert_eq!(strip_fence(prose), prose);
        // A fence on one line has no body
        assert_eq!(strip_fence("```x```"), "```x```");
    }

    #[test]
    fn test_validate_input_name() {
        for ok in ["orders", "_private", "x1", "CamelCase"] {
            assert!(validate_input_name(ok).is_ok(), "{ok}");
        }
        for bad in [
            "",
            "1st",
            "has-dash",
            "has space",
            "class",
            "None",
            "ünïcode",
        ] {
            assert!(validate_input_name(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn test_build_stubs() {
        let registry = CapabilityRegistry::starter();
        let inputs = ["orders".to_string(), "rate".to_string()];
        let stubs = build_stubs(
            inputs.iter(),
            &["log".to_string(), "now".to_string()],
            &registry,
        );
        assert!(stubs.starts_with(STUB_HEADER));
        assert!(stubs.contains("orders: Any\n"));
        assert!(stubs.contains("rate: Any\n"));
        assert!(stubs.contains("def log("));
        assert!(stubs.contains("def now("));
        assert!(!stubs.contains("def fail("));
    }

    #[test]
    fn test_not_granted_names_the_allowed_capabilities() {
        let none = not_granted("price_of", &[]);
        assert_eq!(none.exc_type(), ExcType::NameError);
        assert!(none.message().unwrap().contains("granted no capabilities"));
        let some = not_granted("price_of", &["log".to_string(), "now".to_string()]);
        assert!(some.message().unwrap().contains("may call only: log, now"));
    }

    // --- message trimming and stdout capping ---

    #[test]
    fn test_trim_message_keeps_the_end() {
        let long: String = (0..100).map(|i| format!("line {i}\n")).collect();
        let trimmed = trim_message(&long);
        assert!(trimmed.starts_with("..."));
        assert!(trimmed.ends_with("line 99"));
        assert_eq!(trimmed.lines().count(), MAX_ERROR_LINES + 1);

        let wide = "x".repeat(MAX_ERROR_CHARS * 2);
        let trimmed = trim_message(&wide);
        assert_eq!(trimmed.chars().count(), MAX_ERROR_CHARS + 3);

        assert_eq!(trim_message("short\n"), "short");
    }

    #[test]
    fn test_capped_text_truncates_on_a_char_boundary() {
        let mut text = CappedText::default();
        text.push(&"a".repeat(MAX_STDOUT_BYTES - 1));
        text.push("é"); // two bytes: does not fit in the last byte
        assert_eq!(text.text.len(), MAX_STDOUT_BYTES - 1);
        assert!(text.truncated);
        text.push("more");
        assert_eq!(text.text.len(), MAX_STDOUT_BYTES - 1);
    }

    // --- error classification ---

    fn exception(exc_type: ExcType) -> MontyException {
        MontyException::new(exc_type, Some("boom".to_string()))
    }

    fn kind_and_limit(outcome: Outcome) -> (&'static str, Option<&'static str>) {
        match outcome {
            Outcome::Error { kind, limit, .. } => (kind, limit),
            Outcome::Value(_) => ("value", None),
            Outcome::Crashed(_) => ("crashed", None),
        }
    }

    #[test]
    fn test_classify_code_errors() {
        let l = limits();
        let cases = [
            (PoolError::Typing("bad".into()), ("type", None)),
            (
                PoolError::Runtime(exception(ExcType::KeyError)),
                ("runtime", None),
            ),
            (
                PoolError::Runtime(exception(ExcType::RuntimeError)),
                ("runtime", None),
            ),
            (
                PoolError::Runtime(exception(ExcType::MemoryError)),
                ("limit", Some("max_memory")),
            ),
            (
                PoolError::Runtime(exception(ExcType::TimeoutError)),
                ("limit", Some("max_feed_duration")),
            ),
            (
                PoolError::Runtime(exception(ExcType::RecursionError)),
                ("limit", Some("max_recursion_depth")),
            ),
            (
                PoolError::Timeout {
                    timeout: Duration::from_secs(30),
                },
                ("limit", Some("request_timeout")),
            ),
            (
                PoolError::CpuLimit {
                    limit: Duration::from_secs(1),
                    consumed: Duration::from_secs(2),
                },
                ("limit", Some("cpu")),
            ),
        ];
        for (error, expected) in cases {
            let outcome = classify(error, 0, &l).expect("a code error");
            assert_eq!(kind_and_limit(outcome), expected);
        }
    }

    #[test]
    fn test_classify_suspension_limit_needs_the_count() {
        let l = EffectiveLimits::resolve(
            &LimitParams {
                max_suspensions: Some(3),
                ..Default::default()
            },
            &config(),
        )
        .unwrap();
        let at_limit = classify(PoolError::Runtime(exception(ExcType::RuntimeError)), 3, &l);
        assert_eq!(
            kind_and_limit(at_limit.unwrap()),
            ("limit", Some("max_suspensions"))
        );
    }

    #[test]
    fn test_classify_timeout_message_points_at_loops() {
        let outcome = classify(
            PoolError::Timeout {
                timeout: Duration::from_secs(30),
            },
            0,
            &limits(),
        )
        .unwrap();
        match outcome {
            Outcome::Error { message, .. } => assert!(message.contains("loop"), "{message}"),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn test_classify_crash_and_infrastructure() {
        let crash = classify(
            PoolError::Crashed {
                status: None,
                cause: CrashCause::Vanished {
                    context: "running".into(),
                },
            },
            0,
            &limits(),
        )
        .unwrap();
        assert!(matches!(crash, Outcome::Crashed(_)));

        for infra in [
            PoolError::Exhausted,
            PoolError::Spawn("no such file".into()),
            PoolError::Protocol("bad frame".into()),
        ] {
            assert!(classify(infra, 0, &limits()).is_err());
        }
    }

    // --- output ---

    #[test]
    fn test_output_success_shape() {
        let output = build_output(report(Outcome::Value(json!({"a": 1}))), &limits(), &facts());
        assert_eq!(output["ok"], json!(true));
        assert_eq!(output["value"], json!({"a": 1}));
        assert!(output.get("error").is_none());
        let run = &output["run"];
        for key in [
            "limits",
            "consumed",
            "capabilities",
            "type_check",
            "worker_crashes",
            "stdout_truncated",
            "logs_truncated",
            "monty_version",
            "code_sha256",
        ] {
            assert!(run.get(key).is_some(), "run record missing {key}");
        }
        assert_eq!(run["consumed"]["execution_ms"], json!(12.0));
        assert_eq!(run["monty_version"], json!("monty-runtime 1.6.0"));
    }

    #[test]
    fn test_output_error_with_limit() {
        let output = build_output(
            report(Outcome::limit("max_memory", "too big")),
            &limits(),
            &facts(),
        );
        assert_eq!(output["ok"], json!(false));
        assert!(output.get("value").is_none());
        assert_eq!(
            output["error"],
            json!({"kind": "limit", "limit": "max_memory", "message": "too big"})
        );
    }

    #[test]
    fn test_output_fail_is_sticky() {
        let mut r = report(Outcome::Value(json!("carried on")));
        r.state.failure = Some("total must be positive".to_string());
        let output = build_output(r, &limits(), &facts());
        assert_eq!(output["ok"], json!(false));
        assert_eq!(output["error"]["kind"], json!("fail"));
        assert_eq!(output["error"]["message"], json!("total must be positive"));
    }

    #[test]
    fn test_output_repeated_crash_is_an_error() {
        let output = build_output(
            report(Outcome::Crashed("monty worker crashed".into())),
            &limits(),
            &facts(),
        );
        assert_eq!(output["error"]["kind"], json!("crash"));
        assert!(
            output["error"]["message"]
                .as_str()
                .unwrap()
                .contains("crashed twice")
        );
    }

    #[test]
    fn test_output_value_size_is_capped() {
        let big = Value::String("x".repeat(MAX_VALUE_BYTES + 1));
        let output = build_output(report(Outcome::Value(big)), &limits(), &facts());
        assert_eq!(output["ok"], json!(false));
        assert_eq!(output["error"]["kind"], json!("output"));
    }
}
