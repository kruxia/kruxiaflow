use serde::{Deserialize, Serialize};

/// Performance metrics exported to JSON for CI comparison
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkResults {
    pub timestamp: String,
    pub git_sha: String,
    pub scenarios: Vec<ScenarioMetrics>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioMetrics {
    pub name: String,
    pub total_workflows: usize,
    pub successful_workflows: usize,
    pub failed_workflows: usize,
    pub success_rate: f64,
    pub duration_seconds: f64,
    pub throughput_wf_per_sec: f64,
    pub latency_p50_ms: u64,
    pub latency_p95_ms: u64,
    pub latency_p99_ms: u64,
}

impl BenchmarkResults {
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("Failed to serialize benchmark results")
    }

    pub fn save_to_file(&self, path: &std::path::Path) -> std::io::Result<()> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)
    }
}
