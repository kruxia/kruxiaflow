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
