use crate::registry::ActivityImpl;
use anyhow::Result;
use async_trait::async_trait;
use kruxiaflow_worker::ActivityResult;
use serde_json::Value;

/// Echo activity (for testing)
///
/// Returns the input parameters as output.
pub struct EchoActivity;

#[async_trait]
impl ActivityImpl for EchoActivity {
    async fn execute(&self, parameters: Value) -> Result<ActivityResult> {
        tracing::debug!("Executing echo activity with parameters: {:?}", parameters);

        Ok(ActivityResult::value("echo", parameters))
    }

    fn name(&self) -> &str {
        "echo"
    }

    fn worker(&self) -> &str {
        "std"
    }
}
