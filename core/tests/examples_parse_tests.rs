//! Every workflow in `examples/` parses and validates.
//!
//! `WorkflowDefinition::from_yaml` is what submission runs, so an example that
//! fails here cannot be submitted at all. The directory is read at run time,
//! so a new example is covered without touching this file.

use kruxiaflow_core::WorkflowDefinition;
use std::path::PathBuf;

#[test]
fn test_every_example_parses() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../examples");
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
        .map(|entry| entry.expect("directory entry").path())
        .filter(|p| matches!(p.extension().and_then(|e| e.to_str()), Some("yaml" | "yml")))
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "no examples found in {}", dir.display());

    let failures: Vec<String> = paths
        .iter()
        .filter_map(|path| {
            let yaml = std::fs::read_to_string(path)
                .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
            let name = path.file_name()?.to_string_lossy().into_owned();
            WorkflowDefinition::from_yaml(&yaml)
                .err()
                .map(|e| format!("{name}: {e}"))
        })
        .collect();

    assert!(
        failures.is_empty(),
        "{} of {} examples failed to parse:\n{}",
        failures.len(),
        paths.len(),
        failures.join("\n")
    );
}

/// Example 07b's loop decision reads the start of the evaluation's latest
/// reply, in any case: a reply that starts with CONTINUE loops back, and any
/// other reply (one that mentions "continue" later included) ends the loop
#[test]
fn test_07b_loop_decision_reads_the_reply() {
    use kruxiaflow_core::{TemplateContext, evaluate_condition};
    use rust_decimal::Decimal;
    use serde_json::json;
    use std::collections::HashMap;

    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../examples/07b-agentic-research-complete.yaml");
    let def = WorkflowDefinition::from_yaml(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let conditions = |key: &str| -> Vec<String> {
        def.activities
            .iter()
            .find(|a| a.key == key)
            .and_then(|a| a.depends_on.as_ref())
            .and_then(|deps| {
                deps.iter()
                    .find(|d| d.activity_key == "evaluate_sufficiency")
            })
            .and_then(|d| d.conditions.clone())
            .unwrap_or_else(|| panic!("{key} has conditions on evaluate_sufficiency"))
    };
    let holds = |key: &str, reply: &str, round: u32| -> bool {
        let mut ctx = TemplateContext::new();
        ctx.add_activity_state(
            "evaluate_sufficiency".to_string(),
            vec![],
            Some(HashMap::from([(
                "result".to_string(),
                vec![json!({ "content": reply })],
            )])),
            round,
            Decimal::ZERO,
            "completed".to_string(),
        );
        ctx.add_activity_state(
            "search_information".to_string(),
            vec![],
            Some(HashMap::new()),
            round,
            Decimal::ZERO,
            "completed".to_string(),
        );
        conditions(key)
            .iter()
            .all(|c| evaluate_condition(c, &ctx).unwrap_or_else(|e| panic!("{key}: {c}: {e}")))
    };

    for (reply, more) in [
        ("CONTINUE: the sources disagree", true),
        ("continue - two gaps remain", true),
        ("  CONTINUE", true),
        ("SUFFICIENT. No need to CONTINUE.", false),
        ("SUFFICIENT", false),
    ] {
        assert_eq!(
            holds("search_information", reply, 1),
            more,
            "loop back on {reply:?}"
        );
        assert_eq!(
            holds("compile_report", reply, 1),
            !more,
            "compile on {reply:?}"
        );
        assert_eq!(
            holds("publish_failure", reply, 5),
            more,
            "fail on {reply:?} at the limit"
        );
        assert!(
            !holds("publish_failure", reply, 1),
            "fail on {reply:?} before the limit"
        );
    }
}

/// Example 06a's log step reads only outputs `llm_prompt` returns
#[test]
fn test_06a_log_step_reads_real_llm_outputs() {
    use kruxiaflow_core::{ActivityOutput, TemplateContext, resolve_template_value};
    use serde_json::json;
    use std::collections::HashMap;

    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../examples/06a-faq-bot-caching.yaml");
    let def = WorkflowDefinition::from_yaml(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let store = def
        .activities
        .iter()
        .find(|a| a.key == "store_answer")
        .expect("store_answer");

    // The keys of llm_prompt's `result` (worker/src/activities/llm.rs)
    let mut ctx = TemplateContext::new()
        .with_inputs(HashMap::from([("question".to_string(), json!("Hours?"))]))
        .with_secrets(HashMap::from([(
            "db_url".to_string(),
            "postgres://x".to_string(),
        )]));
    ctx.add_activity_output(
        "answer_question".to_string(),
        vec![ActivityOutput::value(
            "result".to_string(),
            json!({
                "content": "9 to 5",
                "model": "claude-haiku-4-5-20251001",
                "provider": "anthropic",
                "finish_reason": "end_turn",
                "cost_usd": 0.001,
                "budget_skipped_models": [],
                "usage": {"prompt_tokens": 1, "output_tokens": 1, "total_tokens": 2, "cached_tokens": 0}
            }),
        )],
    );
    // A whole-value template naming a missing key resolves to null, which the
    // query would bind as NULL
    let params = serde_json::to_value(&store.parameters).unwrap();
    let resolved =
        resolve_template_value(&params, &ctx).expect("store_answer's parameters resolve");
    let bound = resolved["params"].as_array().expect("params");
    assert!(
        bound.iter().all(|v| !v.is_null()),
        "store_answer binds an output llm_prompt doesn't return: {bound:?}"
    );
}
