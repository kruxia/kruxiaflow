//! What the Stack draws for engineers, worked out from a machine's
//! `workflow.yaml` and a run's event log: pure functions, no engine.
//!
//! - **Timeline:** a run's `workflow_events`, as offsets from the
//!   workflow's start, for the logic-analyzer timing diagram.
//! - **Ladder:** one rung per step, from its `depends_on` entries and
//!   settings, as the engine's dependency rules read them
//!   (`docs/conditional-branching.md`, `docs/loops-guide.md`).
//! - **Datasheet:** the workflow's pins (inputs, signals, secrets, outputs)
//!   and its absolute maximum ratings (limits the workflow sets).
//! - **Drawing number:** a stable number per machine, for the title block.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::{Value, json};

// ---------------------------------------------------------------------------
// Timeline
// ---------------------------------------------------------------------------

/// One row of `workflow_events`, as the engine stored it
pub struct EventRow {
    pub event: String,
    pub step: Option<String>,
    /// The `iteration` column: a loop step's round, or null
    pub round: Option<i32>,
    pub payload: Value,
    pub at: DateTime<Utc>,
}

/// A run's timeline: each event as `ms` after `start` (the workflow's
/// `created_at`), with its step, type, round and attempt, and why a signal
/// wait ended or an attempt failed. Fields the event doesn't carry are left
/// out, to keep results files small. Rows keep their order.
pub fn timeline(rows: &[EventRow], start: DateTime<Utc>) -> Vec<Value> {
    rows.iter()
        .map(|r| {
            let micros = (r.at - start).num_microseconds().unwrap_or(0);
            // Tenths of a millisecond
            let ms = (micros as f64 / 100.0).round() / 10.0;
            let mut e = serde_json::Map::new();
            e.insert("ms".into(), json!(ms));
            if let Some(step) = &r.step {
                e.insert("step".into(), json!(step));
            }
            e.insert("event".into(), json!(r.event));
            if let Some(round) = r.round {
                e.insert("round".into(), json!(round));
            }
            if let Some(attempt) = r.payload.get("attempt").and_then(Value::as_i64) {
                e.insert("attempt".into(), json!(attempt));
            }
            // A wait that ended by its timeout says so; a failure names its code
            let reason = r
                .payload
                .get("reason")
                .or_else(|| r.payload.get("error_code"))
                .and_then(Value::as_str);
            if let Some(reason) = reason {
                e.insert("reason".into(), json!(reason));
            }
            if let Some(action) = r.payload.get("on_timeout").and_then(Value::as_str)
                && r.event != "ActivityWaiting"
            {
                e.insert("on_timeout".into(), json!(action.to_lowercase()));
            }
            if r.payload.get("will_retry") == Some(&Value::Bool(true)) {
                e.insert("will_retry".into(), json!(true));
            }
            Value::Object(e)
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Workflow definitions, as the Stack reads them
// ---------------------------------------------------------------------------

/// One `depends_on` entry, with `dependency_of` folded in
#[derive(Clone, Debug)]
struct Dep {
    step: String,
    conditions: Vec<String>,
}

struct Step<'a> {
    key: String,
    def: &'a Value,
    deps: Vec<Dep>,
}

/// The steps in file order, each with its `depends_on`, as the engine
/// normalizes them: `dependency_of: [b]` on `a` is `depends_on: [a]` on `b`
fn steps(workflow: &Value) -> Vec<Step<'_>> {
    let acts: Vec<&Value> = workflow["activities"]
        .as_array()
        .map(|a| a.iter().collect())
        .unwrap_or_default();
    let dep = |d: &Value| -> Option<Dep> {
        match d {
            Value::String(k) => Some(Dep {
                step: k.clone(),
                conditions: vec![],
            }),
            Value::Object(m) => {
                let step = m.get("activity_key")?.as_str()?.to_string();
                // `conditions` is a list, or one string; `condition` is an alias
                let c = m.get("conditions").or_else(|| m.get("condition"));
                let conditions = match c {
                    Some(Value::String(s)) => vec![s.clone()],
                    Some(Value::Array(list)) => list
                        .iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect(),
                    _ => vec![],
                };
                Some(Dep { step, conditions })
            }
            _ => None,
        }
    };
    let mut deps: BTreeMap<String, Vec<Dep>> = BTreeMap::new();
    for a in &acts {
        let key = a["key"].as_str().unwrap_or_default().to_string();
        let list = deps.entry(key.clone()).or_default();
        list.extend(
            a["depends_on"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(dep),
        );
        for d in a["dependency_of"].as_array().into_iter().flatten() {
            if let Some(target) = dep(d) {
                deps.entry(target.step).or_default().push(Dep {
                    step: key.clone(),
                    conditions: target.conditions,
                });
            }
        }
    }
    acts.into_iter()
        .map(|a| {
            let key = a["key"].as_str().unwrap_or_default().to_string();
            Step {
                deps: deps.get(&key).cloned().unwrap_or_default(),
                key,
                def: a,
            }
        })
        .collect()
}

/// The `depends_on` entries that close a loop, as `(step, dependency)`:
/// a depth-first walk along `depends_on`, from the steps with no
/// dependency of their own first, then the rest, each in file order. An
/// entry whose dependency is still on the walk's path is the back-edge; a
/// self-loop always is.
fn back_edges(steps: &[Step]) -> BTreeSet<(String, String)> {
    // dependents[d] = steps that depend on d, in file order
    let mut dependents: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for s in steps {
        for d in &s.deps {
            dependents
                .entry(d.step.as_str())
                .or_default()
                .push(s.key.as_str());
        }
    }
    let mut order: Vec<&str> = steps
        .iter()
        .filter(|s| s.deps.iter().all(|d| d.step == s.key))
        .map(|s| s.key.as_str())
        .collect();
    let rest: Vec<&str> = steps
        .iter()
        .map(|s| s.key.as_str())
        .filter(|k| !order.contains(k))
        .collect();
    order.extend(rest);

    fn walk<'a>(
        node: &'a str,
        dependents: &BTreeMap<&'a str, Vec<&'a str>>,
        seen: &mut BTreeSet<&'a str>,
        path: &mut Vec<&'a str>,
        found: &mut BTreeSet<(String, String)>,
    ) {
        seen.insert(node);
        path.push(node);
        for &next in dependents.get(node).into_iter().flatten() {
            if path.contains(&next) {
                // `next` depends on `node`, and `node` is downstream of `next`
                found.insert((next.to_string(), node.to_string()));
            } else if !seen.contains(next) {
                walk(next, dependents, seen, path, found);
            }
        }
        path.pop();
    }
    let mut seen = BTreeSet::new();
    let mut found = BTreeSet::new();
    for start in order {
        if !seen.contains(start) {
            walk(start, &dependents, &mut seen, &mut Vec::new(), &mut found);
        }
    }
    found
}

/// `{{ a < b }}` as `a < b`
fn bare(expr: &str) -> String {
    let t = expr.trim();
    let t = t.strip_prefix("{{").unwrap_or(t);
    let t = t.strip_suffix("}}").unwrap_or(t);
    t.trim().to_string()
}

/// A step's `iteration_limit`, on the step or in its settings
fn iteration_limit(def: &Value) -> Option<u64> {
    def["iteration_limit"]
        .as_u64()
        .or_else(|| def["settings"]["iteration_limit"].as_u64())
}

/// The engine's limit for a loop step that sets none
pub const DEFAULT_ITERATION_LIMIT: u64 = 100;

// ---------------------------------------------------------------------------
// Ladder
// ---------------------------------------------------------------------------

/// An element on a rung, read left to right; elements in a list are in
/// series (AND), and `Parallel` branches are alternatives (OR)
#[derive(Serialize, Debug, Clone, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum El {
    /// Normally open: closed when `step` is `completed`, or, for a
    /// dependency with conditions, `finished` (completed, failed or skipped)
    Contact {
        step: String,
        state: &'static str,
    },
    /// Closed when the expression is true: a condition on the `depends_on`
    /// entry for `step`
    Condition {
        step: String,
        expr: String,
    },
    /// Closed only in a loop step's first round
    FirstRound,
    /// Seal-in: closed when `step` has completed a round, so the coil runs
    /// again (a loop's back-edge; `step` is the coil itself on a self-loop)
    SealIn {
        step: String,
    },
    /// A plain wire across a branch: an unconditional `depends_on` makes
    /// the conditions beside it moot
    Shunt,
    Parallel {
        branches: Vec<Vec<El>>,
    },
    /// CTU: counts the coil's rounds, and opens once it reaches `preset`
    /// (`default`: the workflow sets no `iteration_limit`)
    Counter {
        step: String,
        preset: u64,
        default: bool,
    },
    /// An on-delay in series: `settings.delay`
    Delay {
        step: String,
        preset: String,
    },
    /// Closed by a signal: `wait_for_signal`'s event
    Interrupt {
        event: String,
    },
    /// The signal wait's TON timer, done: its time ran out
    TimerDone {
        step: String,
    },
}

/// What a branch at the right of the rung drives
#[derive(Serialize, Debug, Clone, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum End {
    /// The step's coil. `action` is `run`, or for a signal wait's timeout,
    /// what `on_timeout` does: `continue` (run with no signal), `skip` or
    /// `fail`
    Coil { step: String, action: String },
    /// TON timer for a signal wait: times while the step waits
    Timer {
        step: String,
        seconds: u64,
        event: String,
    },
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Output {
    pub contacts: Vec<El>,
    pub end: End,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Rung {
    pub step: String,
    pub activity: String,
    /// The rung's conditions, from the left rail
    pub series: Vec<El>,
    /// The branches the conditions drive, to the right rail
    pub outputs: Vec<Output>,
}

/// One rung per step, in file order.
///
/// The engine's rules, drawn as contacts:
/// - Every `depends_on` entry without conditions is a contact in series:
///   all of them must be completed (a fan-in is an AND).
/// - An entry with conditions is a `finished` contact in series (the step
///   waits for it to complete, fail or be skipped) and its conditions in
///   series on a branch of their own. The engine runs the step when any
///   entry applies, so those branches are in parallel: an OR. Beside an
///   unconditional entry they are moot, so a shunt closes the OR.
/// - A back-edge is a seal-in branch, in parallel with the way in: the
///   way in carries a first-round contact, the seal-in branch the
///   back-edge's conditions. A loop step's rounds pass through a CTU.
/// - A signal wait drives a TON timer and the coil through the signal's
///   contact; the timer's done contact drives what `on_timeout` does.
pub fn ladder(workflow: &Value) -> Vec<Rung> {
    let steps = steps(workflow);
    let back = back_edges(&steps);
    steps
        .iter()
        .map(|s| {
            let is_back = |d: &Dep| back.contains(&(s.key.clone(), d.step.clone()));
            let forward: Vec<&Dep> = s.deps.iter().filter(|d| !is_back(d)).collect();
            let backs: Vec<&Dep> = s.deps.iter().filter(|d| is_back(d)).collect();
            let plain: Vec<&&Dep> = forward.iter().filter(|d| d.conditions.is_empty()).collect();
            let gated: Vec<&&Dep> = forward
                .iter()
                .filter(|d| !d.conditions.is_empty())
                .collect();

            // The way in: unconditional contacts, then each gated entry's
            // finished contact, then the OR of their conditions
            let mut way_in: Vec<El> = plain
                .iter()
                .map(|d| El::Contact {
                    step: d.step.clone(),
                    state: "completed",
                })
                .collect();
            way_in.extend(gated.iter().map(|d| El::Contact {
                step: d.step.clone(),
                state: "finished",
            }));
            let conds = |d: &Dep| -> Vec<El> {
                d.conditions
                    .iter()
                    .map(|c| El::Condition {
                        step: d.step.clone(),
                        expr: bare(c),
                    })
                    .collect()
            };
            let mut branches: Vec<Vec<El>> = gated.iter().map(|d| conds(d)).collect();
            if !branches.is_empty() && !plain.is_empty() {
                branches.push(vec![El::Shunt]);
            }
            match branches.len() {
                0 => {}
                1 => way_in.extend(branches.remove(0)),
                _ => way_in.push(El::Parallel { branches }),
            }

            let mut series = Vec::new();
            if backs.is_empty() {
                series = way_in;
            } else {
                let mut first = vec![El::FirstRound];
                first.extend(way_in);
                let mut seal = Vec::new();
                for d in &backs {
                    seal.push(El::SealIn {
                        step: d.step.clone(),
                    });
                    seal.extend(conds(d));
                }
                series.push(El::Parallel {
                    branches: vec![first, seal],
                });
            }
            let limit = iteration_limit(s.def);
            if !backs.is_empty() || limit.is_some() {
                series.push(El::Counter {
                    step: s.key.clone(),
                    preset: limit.unwrap_or(DEFAULT_ITERATION_LIMIT),
                    default: limit.is_none(),
                });
            }
            if let Some(delay) = s.def["settings"]["delay"].as_str() {
                series.push(El::Delay {
                    step: s.key.clone(),
                    preset: delay.to_string(),
                });
            }

            let wait = &s.def["settings"]["wait_for_signal"];
            let outputs = if wait.is_object() {
                let event = wait["event_name"].as_str().unwrap_or_default().to_string();
                let action = wait["on_timeout"].as_str().unwrap_or("fail").to_string();
                vec![
                    Output {
                        contacts: vec![],
                        end: End::Timer {
                            step: s.key.clone(),
                            seconds: wait["timeout_seconds"].as_u64().unwrap_or(0),
                            event: event.clone(),
                        },
                    },
                    Output {
                        contacts: vec![El::Interrupt { event }],
                        end: End::Coil {
                            step: s.key.clone(),
                            action: "run".into(),
                        },
                    },
                    Output {
                        contacts: vec![El::TimerDone {
                            step: s.key.clone(),
                        }],
                        end: End::Coil {
                            step: s.key.clone(),
                            action,
                        },
                    },
                ]
            } else {
                vec![Output {
                    contacts: vec![],
                    end: End::Coil {
                        step: s.key.clone(),
                        action: "run".into(),
                    },
                }]
            };
            Rung {
                step: s.key.clone(),
                activity: s.def["activity_name"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                series,
                outputs,
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Datasheet
// ---------------------------------------------------------------------------

/// Every `<prefix>name` in the workflow's strings, in order of first use
fn references(workflow: &Value, prefix: &str) -> Vec<String> {
    fn strings<'a>(v: &'a Value, out: &mut Vec<&'a str>) {
        match v {
            Value::String(s) => out.push(s),
            Value::Array(a) => a.iter().for_each(|x| strings(x, out)),
            Value::Object(m) => m.values().for_each(|x| strings(x, out)),
            _ => {}
        }
    }
    let mut all = Vec::new();
    strings(&workflow["activities"], &mut all);
    let mut found: Vec<String> = Vec::new();
    for s in all {
        let mut rest = s;
        while let Some(i) = rest.find(prefix) {
            // Only a whole word: not `MY_INPUT.x`
            let before = rest[..i].chars().next_back();
            let tail = &rest[i + prefix.len()..];
            let name: String = tail
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            if !before.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
                && !name.is_empty()
                && !found.contains(&name)
            {
                found.push(name.clone());
            }
            rest = &tail[name.len()..];
        }
    }
    found
}

/// The workflow's pins: `INPUT` fields and secrets in, signals as
/// interrupts, and the declared outputs of the steps nothing depends on
/// (a step that only depends on itself still counts as nothing)
pub fn pins(workflow: &Value) -> Vec<Value> {
    let steps = steps(workflow);
    let mut pins: Vec<Value> = references(workflow, "INPUT.")
        .into_iter()
        .map(|n| json!({ "dir": "in", "kind": "input", "name": n }))
        .collect();
    for s in &steps {
        let wait = &s.def["settings"]["wait_for_signal"];
        if let Some(event) = wait["event_name"].as_str() {
            pins.push(json!({ "dir": "in", "kind": "signal", "name": event, "step": s.key }));
        }
    }
    pins.extend(
        references(workflow, "SECRET.")
            .into_iter()
            .map(|n| json!({ "dir": "in", "kind": "secret", "name": n })),
    );
    let needed: BTreeSet<&str> = steps
        .iter()
        .flat_map(|s| {
            s.deps
                .iter()
                .filter(|d| d.step != s.key)
                .map(|d| d.step.as_str())
        })
        .collect();
    for s in steps.iter().filter(|s| !needed.contains(s.key.as_str())) {
        for o in s.def["outputs"].as_array().into_iter().flatten() {
            let name = o.as_str().or_else(|| o["name"].as_str());
            if let Some(name) = name {
                pins.push(json!({ "dir": "out", "kind": "output", "name": name, "step": s.key }));
            }
        }
    }
    pins
}

/// The limits the workflow sets, as absolute maximum ratings: rounds,
/// signal waits, activity timeouts, attempts, delays and budgets. A loop
/// step with no `iteration_limit` has the engine's default.
pub fn ratings(workflow: &Value) -> Vec<Value> {
    let steps = steps(workflow);
    let back = back_edges(&steps);
    let heads: BTreeSet<&str> = back.iter().map(|(s, _)| s.as_str()).collect();
    let mut out = Vec::new();
    let w = &workflow["settings"];
    if let Some(t) = w["timeout"].as_u64() {
        out.push(
            json!({ "parameter": "Workflow time", "symbol": "t(WF)", "value": t, "unit": "s" }),
        );
    }
    if let Some(n) = w["max_retries"].as_u64() {
        out.push(
            json!({ "parameter": "Workflow retries", "symbol": "R(WF)", "value": n, "unit": "" }),
        );
    }
    for s in &steps {
        let k = &s.key;
        let set = &s.def["settings"];
        match iteration_limit(s.def) {
            Some(n) => out.push(json!({
                "parameter": format!("Rounds of {k}"), "symbol": format!("N({k})"),
                "value": n, "unit": "rounds", "note": "iteration_limit",
            })),
            None if heads.contains(k.as_str()) => out.push(json!({
                "parameter": format!("Rounds of {k}"), "symbol": format!("N({k})"),
                "value": DEFAULT_ITERATION_LIMIT, "unit": "rounds",
                "note": "engine default; the workflow sets no iteration_limit",
            })),
            None => {}
        }
        let wait = &set["wait_for_signal"];
        if wait.is_object() {
            out.push(json!({
                "parameter": format!("Signal wait, {k} ({})", wait["event_name"].as_str().unwrap_or("")),
                "symbol": format!("tW({k})"),
                "value": wait["timeout_seconds"], "unit": "s",
                "note": format!("on_timeout: {}", wait["on_timeout"].as_str().unwrap_or("fail")),
            }));
        }
        if let Some(t) = set["timeout_seconds"].as_u64() {
            out.push(json!({
                "parameter": format!("Activity time, {k}"), "symbol": format!("tA({k})"),
                "value": t, "unit": "s",
            }));
        }
        let retry = &set["retry"];
        if retry.is_object() {
            let strategy = retry["strategy"].as_str().unwrap_or("exponential");
            let base = retry["base_seconds"].as_u64();
            let note = match base {
                Some(b) => format!("{strategy} backoff, base {b} s"),
                None => format!("{strategy} backoff"),
            };
            out.push(json!({
                "parameter": format!("Attempts, {k}"), "symbol": format!("A({k})"),
                "value": retry["max_attempts"].as_u64().unwrap_or(1), "unit": "attempts",
                "note": note,
            }));
        }
        if let Some(d) = set["delay"].as_str() {
            out.push(json!({
                "parameter": format!("Delay before {k}"), "symbol": format!("tD({k})"),
                "value": d, "unit": "",
            }));
        }
        let budget = &set["budget"];
        if budget.is_object() {
            out.push(json!({
                "parameter": format!("Budget, {k}"), "symbol": format!("B({k})"),
                "value": budget["limit"], "unit": "USD",
                "note": format!("on overrun: {}", budget["action"].as_str().unwrap_or("")),
            }));
        }
    }
    out
}

/// A drawing number that stays with the machine's directory name, whatever
/// machines are added or removed: `MS-` and five digits of its FNV-1a hash
pub fn drawing_number(dir: &str) -> String {
    let mut h: u32 = 0x811c_9dc5;
    for b in dir.bytes() {
        h ^= u32::from(b);
        h = h.wrapping_mul(0x0100_0193);
    }
    format!("MS-{:05}", h % 100_000)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wf(yaml: &str) -> Value {
        serde_yaml::from_str(yaml).unwrap()
    }

    fn contact(step: &str, state: &'static str) -> El {
        El::Contact {
            step: step.into(),
            state,
        }
    }

    fn cond(step: &str, expr: &str) -> El {
        El::Condition {
            step: step.into(),
            expr: expr.into(),
        }
    }

    fn coil(step: &str) -> Vec<Output> {
        vec![Output {
            contacts: vec![],
            end: End::Coil {
                step: step.into(),
                action: "run".into(),
            },
        }]
    }

    fn rung<'a>(rungs: &'a [Rung], step: &str) -> &'a Rung {
        rungs.iter().find(|r| r.step == step).unwrap()
    }

    #[test]
    fn timeline_offsets_and_fields() {
        let start: DateTime<Utc> = "2026-11-20T10:00:00Z".parse().unwrap();
        let at = |ms: i64| start + chrono::Duration::microseconds(ms * 1000 + 250);
        let rows = vec![
            EventRow {
                event: "WorkflowCreated".into(),
                step: None,
                round: None,
                payload: json!({ "input": {} }),
                at: start,
            },
            EventRow {
                event: "ActivityScheduled".into(),
                step: Some("heater".into()),
                round: Some(1),
                payload: json!({ "worker": "std" }),
                at: at(12),
            },
            EventRow {
                event: "ActivityWaiting".into(),
                step: Some("toast".into()),
                round: None,
                payload: json!({ "event_name": "pop", "on_timeout": "Continue" }),
                at: at(20),
            },
            EventRow {
                event: "ActivitySignaled".into(),
                step: Some("toast".into()),
                round: None,
                payload: json!({ "reason": "timeout", "on_timeout": "continue" }),
                at: at(2020),
            },
            EventRow {
                event: "ActivityFailed".into(),
                step: Some("yellow".into()),
                round: None,
                payload: json!({ "error_code": "QUERY_FAILED", "attempt": 2, "will_retry": true }),
                at: at(3000),
            },
        ];
        let t = timeline(&rows, start);
        assert_eq!(t[0], json!({ "ms": 0.0, "event": "WorkflowCreated" }));
        assert_eq!(
            t[1],
            json!({ "ms": 12.3, "step": "heater", "event": "ActivityScheduled", "round": 1 })
        );
        // The wait's own on_timeout is its setting, not an outcome
        assert_eq!(
            t[2],
            json!({ "ms": 20.3, "step": "toast", "event": "ActivityWaiting" })
        );
        assert_eq!(
            t[3],
            json!({ "ms": 2020.3, "step": "toast", "event": "ActivitySignaled",
                    "reason": "timeout", "on_timeout": "continue" })
        );
        assert_eq!(
            t[4],
            json!({ "ms": 3000.3, "step": "yellow", "event": "ActivityFailed",
                    "attempt": 2, "reason": "QUERY_FAILED", "will_retry": true })
        );
    }

    #[test]
    fn ladder_fan_in_is_contacts_in_series() {
        let rungs = ladder(&wf(r#"
activities:
  - { key: beam, activity_name: echo }
  - { key: red, activity_name: echo, depends_on: [beam] }
  - { key: blue, activity_name: echo, depends_on: [beam] }
  - key: funnel
    activity_name: echo
    depends_on: [red, blue]
"#));
        assert_eq!(rung(&rungs, "beam").series, vec![]);
        assert_eq!(rung(&rungs, "beam").outputs, coil("beam"));
        assert_eq!(
            rung(&rungs, "funnel").series,
            vec![contact("red", "completed"), contact("blue", "completed")]
        );
    }

    #[test]
    fn ladder_dependency_of_is_folded_into_depends_on() {
        let rungs = ladder(&wf(r#"
activities:
  - { key: drum, activity_name: echo, dependency_of: [oar] }
  - { key: oar, activity_name: echo }
"#));
        assert_eq!(
            rung(&rungs, "oar").series,
            vec![contact("drum", "completed")]
        );
    }

    #[test]
    fn ladder_conditions() {
        let rungs = ladder(&wf(r#"
activities:
  - { key: lever, activity_name: echo }
  - key: left
    activity_name: echo
    depends_on:
      - activity_key: lever
        conditions: ["{{ lever.echo.track == 'left' }}", "{{ lever.echo.ok }}"]
  - key: close_out
    activity_name: echo
    depends_on:
      - activity_key: lever
        conditions: ["{{ lever.status == 'completed' }}"]
      - activity_key: left
        condition: "{{ left.status == 'skipped' }}"
  - key: moot
    activity_name: echo
    depends_on:
      - left
      - activity_key: lever
        conditions: ["{{ lever.echo.x }}"]
"#));
        // One entry: its finished contact, then its conditions, in series
        assert_eq!(
            rung(&rungs, "left").series,
            vec![
                contact("lever", "finished"),
                cond("lever", "lever.echo.track == 'left'"),
                cond("lever", "lever.echo.ok"),
            ]
        );
        // Two entries with conditions: either one is enough
        assert_eq!(
            rung(&rungs, "close_out").series,
            vec![
                contact("lever", "finished"),
                contact("left", "finished"),
                El::Parallel {
                    branches: vec![
                        vec![cond("lever", "lever.status == 'completed'")],
                        vec![cond("left", "left.status == 'skipped'")],
                    ]
                },
            ]
        );
        // Beside an unconditional entry, a condition is shunted
        assert_eq!(
            rung(&rungs, "moot").series,
            vec![
                contact("left", "completed"),
                contact("lever", "finished"),
                El::Parallel {
                    branches: vec![vec![cond("lever", "lever.echo.x")], vec![El::Shunt]]
                },
            ]
        );
    }

    #[test]
    fn ladder_self_loop_is_a_seal_in_with_a_counter() {
        let rungs = ladder(&wf(r#"
activities:
  - { key: dial, activity_name: echo }
  - key: heater
    activity_name: echo
    iteration_limit: 10
    depends_on:
      - activity_key: dial
        conditions: ["{{ dial.echo.room < dial.echo.setpoint }}"]
      - activity_key: heater
        conditions: ["{{ INPUT.room + heater.iteration < INPUT.setpoint }}"]
  - key: meter
    activity_name: echo
    depends_on: [meter]
"#));
        assert_eq!(
            rung(&rungs, "heater").series,
            vec![
                El::Parallel {
                    branches: vec![
                        vec![
                            El::FirstRound,
                            contact("dial", "finished"),
                            cond("dial", "dial.echo.room < dial.echo.setpoint"),
                        ],
                        vec![
                            El::SealIn {
                                step: "heater".into()
                            },
                            cond("heater", "INPUT.room + heater.iteration < INPUT.setpoint"),
                        ],
                    ]
                },
                El::Counter {
                    step: "heater".into(),
                    preset: 10,
                    default: false
                },
            ]
        );
        // A loop with no way in but its first round, and the engine's limit
        assert_eq!(
            rung(&rungs, "meter").series,
            vec![
                El::Parallel {
                    branches: vec![
                        vec![El::FirstRound],
                        vec![El::SealIn {
                            step: "meter".into()
                        }],
                    ]
                },
                El::Counter {
                    step: "meter".into(),
                    preset: DEFAULT_ITERATION_LIMIT,
                    default: true
                },
            ]
        );
    }

    #[test]
    fn ladder_two_step_loop_seals_in_on_the_back_edge() {
        // Water pump: piston depends on valve, which depends on piston
        let rungs = ladder(&wf(r#"
activities:
  - { key: intake, activity_name: echo }
  - key: piston
    activity_name: echo
    iteration_limit: 3
    depends_on:
      - intake
      - activity_key: valve
        conditions: ["{{ valve.status == 'completed' }}"]
  - { key: valve, activity_name: echo, depends_on: [piston] }
"#));
        assert_eq!(
            rung(&rungs, "piston").series,
            vec![
                El::Parallel {
                    branches: vec![
                        vec![El::FirstRound, contact("intake", "completed")],
                        vec![
                            El::SealIn {
                                step: "valve".into()
                            },
                            cond("valve", "valve.status == 'completed'"),
                        ],
                    ]
                },
                El::Counter {
                    step: "piston".into(),
                    preset: 3,
                    default: false
                },
            ]
        );
        // valve's dependency on piston is the way forward, not a back-edge
        assert_eq!(
            rung(&rungs, "valve").series,
            vec![contact("piston", "completed")]
        );
        // With no root, the walk starts in file order: kiln's heat comes first
        let kiln = ladder(&wf(r#"
activities:
  - key: heat
    activity_name: echo
    depends_on: [{ activity_key: stoke, conditions: ["{{ stoke.status == 'completed' }}"] }]
  - { key: stoke, activity_name: echo, depends_on: [heat] }
"#));
        assert!(
            matches!(&rung(&kiln, "heat").series[0], El::Parallel { branches } if branches[1][0] == El::SealIn { step: "stoke".into() })
        );
        assert_eq!(
            rung(&kiln, "stoke").series,
            vec![contact("heat", "completed")]
        );
    }

    #[test]
    fn ladder_signal_wait_drives_a_timer_and_its_timeout() {
        let rungs = ladder(&wf(r#"
activities:
  - { key: load, activity_name: echo }
  - key: toast
    activity_name: echo
    settings:
      wait_for_signal: { event_name: pop, timeout_seconds: 2, on_timeout: continue }
    depends_on: [load]
  - key: butter
    activity_name: echo
    settings: { delay: 2s }
    depends_on: [toast]
  - key: locker
    activity_name: echo
    settings:
      wait_for_signal: { event_name: collect, timeout_seconds: 5 }
    depends_on: [load]
"#));
        let toast = rung(&rungs, "toast");
        assert_eq!(toast.series, vec![contact("load", "completed")]);
        assert_eq!(
            toast.outputs,
            vec![
                Output {
                    contacts: vec![],
                    end: End::Timer {
                        step: "toast".into(),
                        seconds: 2,
                        event: "pop".into()
                    },
                },
                Output {
                    contacts: vec![El::Interrupt {
                        event: "pop".into()
                    }],
                    end: End::Coil {
                        step: "toast".into(),
                        action: "run".into()
                    },
                },
                Output {
                    contacts: vec![El::TimerDone {
                        step: "toast".into()
                    }],
                    end: End::Coil {
                        step: "toast".into(),
                        action: "continue".into()
                    },
                },
            ]
        );
        // on_timeout defaults to fail
        assert_eq!(
            rung(&rungs, "locker").outputs[2].end,
            End::Coil {
                step: "locker".into(),
                action: "fail".into()
            }
        );
        assert_eq!(
            rung(&rungs, "butter").series,
            vec![
                contact("toast", "completed"),
                El::Delay {
                    step: "butter".into(),
                    preset: "2s".into()
                },
            ]
        );
    }

    #[test]
    fn datasheet_pins_and_ratings() {
        let w = wf(r#"
activities:
  - key: load
    activity_name: echo
    parameters: { bread: "{{ INPUT.bread }}", db: "{{ SECRET.db_url }}", n: "{{ MY_INPUT.x }}" }
    outputs: [echo]
  - key: toast
    activity_name: echo
    parameters: { bread: "{{ INPUT.bread }} and {{INPUT.jam}}" }
    settings:
      wait_for_signal: { event_name: pop, timeout_seconds: 2, on_timeout: continue }
      retry: { max_attempts: 3, strategy: fixed, base_seconds: 1 }
    depends_on: [load]
    outputs: [echo, { name: shade }]
  - key: tick
    activity_name: echo
    depends_on: [tick]
    outputs: [echo]
"#);
        let names: Vec<String> = pins(&w)
            .iter()
            .map(|p| {
                format!(
                    "{} {} {}",
                    p["kind"].as_str().unwrap(),
                    p["step"].as_str().unwrap_or("-"),
                    p["name"].as_str().unwrap()
                )
            })
            .collect();
        assert_eq!(
            names,
            [
                "input - bread",
                "input - jam",
                "signal toast pop",
                "secret - db_url",
                "output toast echo",
                "output toast shade",
                "output tick echo",
            ]
        );
        let r = ratings(&w);
        let params: Vec<&str> = r.iter().map(|x| x["parameter"].as_str().unwrap()).collect();
        assert_eq!(
            params,
            [
                "Signal wait, toast (pop)",
                "Attempts, toast",
                "Rounds of tick"
            ]
        );
        assert_eq!(r[0]["note"], "on_timeout: continue");
        assert_eq!(r[1]["value"], 3);
        assert_eq!(r[2]["value"], DEFAULT_ITERATION_LIMIT);
    }

    #[test]
    fn drawing_numbers_are_stable() {
        assert_eq!(drawing_number("thermostat"), drawing_number("thermostat"));
        assert_ne!(drawing_number("thermostat"), drawing_number("toaster"));
        assert_eq!(drawing_number("x").len(), 8);
    }
}

/// Every machine's drawing number is its own
#[test]
fn machine_drawing_numbers_are_unique() {
    let mut seen: BTreeMap<String, String> = BTreeMap::new();
    for e in std::fs::read_dir(crate::machines_dir()).expect("machines/") {
        let path = e.expect("machines/ entry").path();
        if !path.join("card.yaml").is_file() {
            continue;
        }
        let dir = path.file_name().unwrap().to_string_lossy().to_string();
        let n = drawing_number(&dir);
        if let Some(other) = seen.insert(n.clone(), dir.clone()) {
            panic!("{dir} and {other} share drawing number {n}");
        }
    }
}
