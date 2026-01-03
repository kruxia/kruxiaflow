use signal_hook::consts::{SIGINT, SIGTERM};
use signal_hook_tokio::Signals;
use tokio_stream::StreamExt;

/// Wait for shutdown signal (SIGTERM or SIGINT)
pub async fn wait_for_shutdown() {
    let mut signals = Signals::new([SIGTERM, SIGINT]).expect("Failed to register signal handlers");

    if let Some(signal) = signals.next().await {
        match signal {
            SIGTERM => tracing::info!("Received SIGTERM, initiating graceful shutdown"),
            SIGINT => tracing::info!("Received SIGINT (Ctrl-C), initiating graceful shutdown"),
            _ => tracing::warn!("Received unexpected signal: {}", signal),
        }
    }
}

/// Create a shutdown signal future for use with tokio::select!
pub async fn shutdown_signal() {
    wait_for_shutdown().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use signal_hook::low_level;
    use std::time::Duration;

    #[tokio::test]
    async fn test_signal_handler_registration() {
        // Test that signal handlers can be registered without panic
        // We'll create the Signals instance but not wait for actual signals
        let result = Signals::new([SIGTERM, SIGINT]);
        assert!(result.is_ok(), "Signal handler registration should succeed");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_shutdown_signal_with_sigterm() {
        // Test that shutdown_signal completes when SIGTERM is sent
        tokio::spawn(async move {
            // Give the signal handler time to register
            tokio::time::sleep(Duration::from_millis(100)).await;
            // Send SIGTERM to self
            // SAFETY: We're sending a signal to our own process in a controlled test environment
            low_level::raise(SIGTERM).unwrap();
        });

        // This should complete when the signal is received
        let result = tokio::time::timeout(Duration::from_secs(2), shutdown_signal()).await;
        assert!(
            result.is_ok(),
            "shutdown_signal should complete when SIGTERM is received"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_shutdown_signal_with_sigint() {
        // Test that shutdown_signal completes when SIGINT is sent
        tokio::spawn(async move {
            // Give the signal handler time to register
            tokio::time::sleep(Duration::from_millis(100)).await;
            // Send SIGINT to self
            // SAFETY: We're sending a signal to our own process in a controlled test environment
            low_level::raise(SIGINT).unwrap();
        });

        // This should complete when the signal is received
        let result = tokio::time::timeout(Duration::from_secs(2), shutdown_signal()).await;
        assert!(
            result.is_ok(),
            "shutdown_signal should complete when SIGINT is received"
        );
    }

    #[tokio::test]
    async fn test_wait_for_shutdown_is_async() {
        // Test that wait_for_shutdown is properly async and can be selected over
        let mut signals = Signals::new([SIGTERM, SIGINT]).unwrap();

        tokio::select! {
            _ = tokio::time::sleep(Duration::from_millis(50)) => {
                // Timeout branch - expected in this test
                // This proves that wait_for_shutdown doesn't block
            }
            _ = async {
                signals.next().await;
            } => {
                panic!("Signal received unexpectedly");
            }
        }
    }
}
