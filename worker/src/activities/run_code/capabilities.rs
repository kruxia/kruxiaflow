//! Host capabilities: the only functions sandboxed code can call.
//!
//! A workflow names the capabilities a `run_code` step may use; it can only
//! choose from this registry, which is compiled into the worker. Every call
//! is checked against the granted set when it arrives (Monty sends a call to
//! any undefined name straight to the host), and every argument is treated
//! as untrusted input.

use super::convert::{json_to_monty, monty_to_json};
use crate::registry::ActivityContext;
use async_trait::async_trait;
use chrono::{DateTime, SecondsFormat, Utc};
use monty_types::{CallArgs, ExcType, MontyException, MontyObject, ObjectRef};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Most `log()` entries kept per run
const MAX_LOG_ENTRIES: usize = 100;
/// Longest `log()` entry kept, in characters
const MAX_LOG_CHARS: usize = 1000;

/// Per-run state the capabilities read and write
pub(crate) struct CallState {
    /// The frozen clock: the activity's start time
    pub started_at: DateTime<Utc>,
    /// Messages from `log()`
    pub logs: Vec<String>,
    /// Set when `log()` dropped entries over the cap
    pub logs_truncated: bool,
    /// Set by `fail()`; sticky, so catching the exception does not undo it
    pub failure: Option<String>,
}

impl CallState {
    pub fn new(started_at: DateTime<Utc>) -> Self {
        Self {
            started_at,
            logs: Vec::new(),
            logs_truncated: false,
            failure: None,
        }
    }
}

/// What a capability can see while it answers a call
pub(crate) struct CallContext<'a> {
    /// Per-run state
    pub state: &'a mut CallState,
    /// The activity being executed (workflow id, storage). Unused by the
    /// phase 1 capabilities; phase 2 capabilities (kv, llm) need it.
    #[allow(dead_code)]
    pub activity: &'a ActivityContext,
}

/// A host function the sandbox may call
#[async_trait]
pub(crate) trait Capability: Send + Sync {
    /// The Python name the sandbox calls
    fn name(&self) -> &'static str;

    /// Its `.pyi` declaration, used to type-check code before it runs
    fn stub(&self) -> &'static str;

    /// Answer one call. An `Err` is raised as that exception in the sandbox.
    async fn call(
        &self,
        args: &CallArgs,
        cx: &mut CallContext<'_>,
    ) -> Result<MontyObject, MontyException>;
}

/// The capabilities compiled into this worker, by name
pub(crate) struct CapabilityRegistry {
    capabilities: BTreeMap<&'static str, Arc<dyn Capability>>,
}

impl CapabilityRegistry {
    /// The phase 1 starter set: pure functions with no side effects outside
    /// the run
    pub fn starter() -> Self {
        let mut registry = Self {
            capabilities: BTreeMap::new(),
        };
        registry.add(Arc::new(JsonPath));
        registry.add(Arc::new(Now));
        registry.add(Arc::new(Log));
        registry.add(Arc::new(Fail));
        registry
    }

    fn add(&mut self, capability: Arc<dyn Capability>) {
        self.capabilities.insert(capability.name(), capability);
    }

    pub fn get(&self, name: &str) -> Option<&Arc<dyn Capability>> {
        self.capabilities.get(name)
    }

    pub fn names(&self) -> Vec<&'static str> {
        self.capabilities.keys().copied().collect()
    }
}

/// Header shared by every stub file
pub(crate) const STUB_HEADER: &str = "from typing import Any, NoReturn\n";

fn type_error(message: impl Into<String>) -> MontyException {
    MontyException::new(ExcType::TypeError, Some(message.into()))
}

/// The `index`th positional argument, or the keyword argument `name`
fn arg<'a>(args: &'a CallArgs, index: usize, name: &str) -> Option<ObjectRef<'a>> {
    args.arg(index).or_else(|| args.kwarg(name))
}

fn str_arg(
    args: &CallArgs,
    index: usize,
    name: &str,
    function: &str,
) -> Result<String, MontyException> {
    let value = arg(args, index, name)
        .ok_or_else(|| type_error(format!("{function}() missing required argument '{name}'")))?;
    value.as_str().map(str::to_string).ok_or_else(|| {
        type_error(format!(
            "{function}() argument '{name}' must be str, not {}",
            value.type_name()
        ))
    })
}

/// `json_path(obj, path, default=None)`: read a nested value with a dotted
/// path such as `"customer.orders[0].total"`. Returns `default` when any step
/// is missing.
struct JsonPath;

#[async_trait]
impl Capability for JsonPath {
    fn name(&self) -> &'static str {
        "json_path"
    }

    fn stub(&self) -> &'static str {
        "def json_path(obj: Any, path: str, default: Any = None) -> Any:\n    \
         \"\"\"Read a nested value by dotted path, e.g. 'customer.orders[0].total'. \
         Returns default when a step is missing.\"\"\"\n    ...\n"
    }

    async fn call(
        &self,
        args: &CallArgs,
        _cx: &mut CallContext<'_>,
    ) -> Result<MontyObject, MontyException> {
        let obj = arg(args, 0, "obj")
            .ok_or_else(|| type_error("json_path() missing required argument 'obj'"))?;
        let path = str_arg(args, 1, "path", "json_path")?;
        let default = arg(args, 2, "default").map(|d| d.to_owned());
        let root = monty_to_json(obj).map_err(|e| type_error(format!("json_path(): {e}")))?;

        let found =
            walk(&root, &path).map_err(|e| MontyException::new(ExcType::ValueError, Some(e)))?;
        Ok(match found {
            Some(value) => json_to_monty(value),
            None => default.unwrap_or_else(MontyObject::none),
        })
    }
}

/// One step of a `json_path` path
#[derive(Debug, PartialEq)]
enum Step<'p> {
    Key(&'p str),
    Index(usize),
}

/// Parse a dotted path with `[n]` indexes, e.g. `customer.orders[0].total`.
/// The whole path is checked up front, so a malformed path is an error
/// whatever the data looks like.
fn parse_path(path: &str) -> Result<Vec<Step<'_>>, String> {
    let mut steps = Vec::new();
    for segment in path.split('.') {
        let (key, mut rest) = match segment.find('[') {
            Some(i) => (&segment[..i], &segment[i..]),
            None => (segment, ""),
        };
        if !key.is_empty() {
            steps.push(Step::Key(key));
        } else if rest.is_empty() {
            return Err(format!("json_path(): empty segment in path {path:?}"));
        }
        while !rest.is_empty() {
            let close = rest
                .find(']')
                .filter(|_| rest.starts_with('['))
                .ok_or_else(|| format!("json_path(): malformed index in path {path:?}"))?;
            let index: usize = rest[1..close].trim().parse().map_err(|_| {
                format!("json_path(): index must be a non-negative integer in {path:?}")
            })?;
            steps.push(Step::Index(index));
            rest = &rest[close + 1..];
        }
    }
    Ok(steps)
}

/// Follow a path. `Ok(None)` when a step is missing, `Err` when the path
/// itself is malformed.
fn walk<'a>(root: &'a Value, path: &str) -> Result<Option<&'a Value>, String> {
    let mut current = root;
    for step in parse_path(path)? {
        let next = match step {
            Step::Key(key) => current.get(key),
            Step::Index(index) => current.get(index),
        };
        match next {
            Some(next) => current = next,
            None => return Ok(None),
        }
    }
    Ok(Some(current))
}

/// `now()`: the activity's start time as an ISO-8601 UTC string. The clock
/// is frozen for the whole run (as `datetime.now()` is), so a retried step
/// sees the same time.
struct Now;

#[async_trait]
impl Capability for Now {
    fn name(&self) -> &'static str {
        "now"
    }

    fn stub(&self) -> &'static str {
        "def now() -> str:\n    \
         \"\"\"The step's start time as an ISO-8601 UTC string (frozen for the run).\"\"\"\n    ...\n"
    }

    async fn call(
        &self,
        _args: &CallArgs,
        cx: &mut CallContext<'_>,
    ) -> Result<MontyObject, MontyException> {
        Ok(MontyObject::string(
            cx.state
                .started_at
                .to_rfc3339_opts(SecondsFormat::Micros, true),
        ))
    }
}

/// `log(message)`: record a message in the run's output (`logs`)
struct Log;

#[async_trait]
impl Capability for Log {
    fn name(&self) -> &'static str {
        "log"
    }

    fn stub(&self) -> &'static str {
        "def log(message: str) -> None:\n    \
         \"\"\"Record a message in the step's output.\"\"\"\n    ...\n"
    }

    async fn call(
        &self,
        args: &CallArgs,
        cx: &mut CallContext<'_>,
    ) -> Result<MontyObject, MontyException> {
        let value = arg(args, 0, "message")
            .ok_or_else(|| type_error("log() missing required argument 'message'"))?;
        let text = value
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| value.py_repr());
        if cx.state.logs.len() >= MAX_LOG_ENTRIES {
            cx.state.logs_truncated = true;
        } else {
            cx.state
                .logs
                .push(text.chars().take(MAX_LOG_CHARS).collect());
        }
        Ok(MontyObject::none())
    }
}

/// `fail(message)`: stop with `ok: false` and a message for whoever reads
/// the result (often the model that will repair the code)
struct Fail;

#[async_trait]
impl Capability for Fail {
    fn name(&self) -> &'static str {
        "fail"
    }

    fn stub(&self) -> &'static str {
        "def fail(message: str) -> NoReturn:\n    \
         \"\"\"Stop and report failure with this message.\"\"\"\n    ...\n"
    }

    async fn call(
        &self,
        args: &CallArgs,
        cx: &mut CallContext<'_>,
    ) -> Result<MontyObject, MontyException> {
        let message = str_arg(args, 0, "message", "fail")?;
        cx.state.failure = Some(message.clone());
        Err(MontyException::new(
            ExcType::RuntimeError,
            Some(format!("fail(): {message}")),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use serde_json::json;
    use uuid::Uuid;

    fn activity() -> ActivityContext {
        ActivityContext::new(Uuid::nil(), Uuid::nil(), "step".to_string(), None)
    }

    fn started() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 18, 12, 30, 0).unwrap()
    }

    async fn call(
        capability: &dyn Capability,
        args: CallArgs,
        state: &mut CallState,
    ) -> Result<MontyObject, MontyException> {
        let activity = activity();
        let mut cx = CallContext {
            state,
            activity: &activity,
        };
        capability.call(&args, &mut cx).await
    }

    fn args(values: Vec<MontyObject>) -> CallArgs {
        CallArgs::from(values)
    }

    // --- walk ---

    #[test]
    fn test_walk_keys_and_indexes() {
        let root = json!({"customer": {"orders": [{"total": 5}, {"total": 7}]}});
        assert_eq!(
            walk(&root, "customer.orders[1].total").unwrap(),
            Some(&json!(7))
        );
        assert_eq!(walk(&root, "customer").unwrap(), Some(&root["customer"]));
    }

    #[test]
    fn test_walk_leading_and_chained_indexes() {
        let root = json!([[1, 2], [3, 4]]);
        assert_eq!(walk(&root, "[1][0]").unwrap(), Some(&json!(3)));
    }

    #[test]
    fn test_walk_missing_steps_are_none() {
        let root = json!({"a": {"b": [1]}});
        assert_eq!(walk(&root, "a.c").unwrap(), None);
        assert_eq!(walk(&root, "a.b[5]").unwrap(), None);
        assert_eq!(walk(&root, "a.b.c").unwrap(), None);
    }

    #[test]
    fn test_parse_path_steps() {
        assert_eq!(
            parse_path("a.b[2][0].c").unwrap(),
            vec![
                Step::Key("a"),
                Step::Key("b"),
                Step::Index(2),
                Step::Index(0),
                Step::Key("c"),
            ]
        );
    }

    #[test]
    fn test_walk_rejects_malformed_paths_even_when_data_is_missing() {
        // Regression: the path used to be checked only as far as the data
        // went, so a bad index after a missing key returned None
        assert!(walk(&json!({}), "a[x]").is_err());
        assert!(walk(&json!({}), "missing.[0").is_err());
    }

    #[test]
    fn test_walk_malformed_paths_are_errors() {
        let root = json!({"a": [1]});
        assert!(walk(&root, "a..b").is_err());
        assert!(walk(&root, "a[x]").is_err());
        assert!(walk(&root, "a[-1]").is_err());
        assert!(walk(&root, "a[0").is_err());
    }

    // --- json_path ---

    #[tokio::test]
    async fn test_json_path_reads_a_nested_value() {
        let obj = json_to_monty(&json!({"customer": {"tags": ["new", "vip"]}}));
        let mut state = CallState::new(started());
        let value = call(
            &JsonPath,
            args(vec![obj, MontyObject::string("customer.tags[1]")]),
            &mut state,
        )
        .await
        .unwrap();
        assert_eq!(value, MontyObject::string("vip"));
    }

    #[tokio::test]
    async fn test_json_path_default_positional_and_keyword() {
        let obj = json_to_monty(&json!({"a": 1}));
        let mut state = CallState::new(started());
        let value = call(
            &JsonPath,
            args(vec![
                obj.clone(),
                MontyObject::string("missing"),
                MontyObject::string("fallback"),
            ]),
            &mut state,
        )
        .await
        .unwrap();
        assert_eq!(value, MontyObject::string("fallback"));

        let mut kwargs = CallArgs::new();
        kwargs.push_kwarg("obj", obj);
        kwargs.push_kwarg("path", MontyObject::string("missing"));
        let value = call(&JsonPath, kwargs, &mut state).await.unwrap();
        assert_eq!(value, MontyObject::none());
    }

    #[tokio::test]
    async fn test_json_path_argument_errors() {
        let mut state = CallState::new(started());
        let err = call(&JsonPath, args(vec![]), &mut state).await.unwrap_err();
        assert_eq!(err.exc_type(), ExcType::TypeError);

        let err = call(
            &JsonPath,
            args(vec![MontyObject::none(), MontyObject::int(3)]),
            &mut state,
        )
        .await
        .unwrap_err();
        assert_eq!(err.exc_type(), ExcType::TypeError);
        assert!(err.message().unwrap_or_default().contains("must be str"));

        let err = call(
            &JsonPath,
            args(vec![json_to_monty(&json!({})), MontyObject::string("a[x]")]),
            &mut state,
        )
        .await
        .unwrap_err();
        assert_eq!(err.exc_type(), ExcType::ValueError);
    }

    // --- now ---

    #[tokio::test]
    async fn test_now_returns_the_frozen_start_time() {
        let mut state = CallState::new(started());
        let value = call(&Now, args(vec![]), &mut state).await.unwrap();
        assert_eq!(value, MontyObject::string("2026-11-18T12:30:00.000000Z"));
    }

    // --- log ---

    #[tokio::test]
    async fn test_log_records_strings_and_reprs() {
        let mut state = CallState::new(started());
        call(&Log, args(vec![MontyObject::string("hello")]), &mut state)
            .await
            .unwrap();
        call(&Log, args(vec![MontyObject::int(42)]), &mut state)
            .await
            .unwrap();
        assert_eq!(state.logs, vec!["hello".to_string(), "42".to_string()]);
        assert!(!state.logs_truncated);
    }

    #[tokio::test]
    async fn test_log_caps_entries_and_length() {
        let mut state = CallState::new(started());
        let long = "x".repeat(MAX_LOG_CHARS + 50);
        call(&Log, args(vec![MontyObject::string(long)]), &mut state)
            .await
            .unwrap();
        assert_eq!(state.logs[0].chars().count(), MAX_LOG_CHARS);

        for i in 0..MAX_LOG_ENTRIES + 3 {
            call(
                &Log,
                args(vec![MontyObject::string(i.to_string())]),
                &mut state,
            )
            .await
            .unwrap();
        }
        assert_eq!(state.logs.len(), MAX_LOG_ENTRIES);
        assert!(state.logs_truncated);
    }

    #[tokio::test]
    async fn test_log_requires_a_message() {
        let mut state = CallState::new(started());
        let err = call(&Log, args(vec![]), &mut state).await.unwrap_err();
        assert_eq!(err.exc_type(), ExcType::TypeError);
    }

    // --- fail ---

    #[tokio::test]
    async fn test_fail_records_the_message_and_raises() {
        let mut state = CallState::new(started());
        let err = call(
            &Fail,
            args(vec![MontyObject::string("total must be positive")]),
            &mut state,
        )
        .await
        .unwrap_err();
        assert_eq!(err.exc_type(), ExcType::RuntimeError);
        assert_eq!(state.failure.as_deref(), Some("total must be positive"));
    }

    #[tokio::test]
    async fn test_fail_requires_a_string() {
        let mut state = CallState::new(started());
        let err = call(&Fail, args(vec![MontyObject::int(1)]), &mut state)
            .await
            .unwrap_err();
        assert_eq!(err.exc_type(), ExcType::TypeError);
        assert!(state.failure.is_none());
    }

    // --- registry ---

    #[test]
    fn test_starter_registry() {
        let registry = CapabilityRegistry::starter();
        assert_eq!(registry.names(), vec!["fail", "json_path", "log", "now"]);
        assert!(registry.get("os_system").is_none());
        for name in registry.names() {
            let stub = registry.get(name).unwrap().stub();
            assert!(
                stub.starts_with(&format!("def {name}(")),
                "stub for {name} must declare it: {stub}"
            );
        }
    }
}
