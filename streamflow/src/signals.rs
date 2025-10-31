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
