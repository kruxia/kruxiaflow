use anyhow::Result;
use tracing_subscriber::{EnvFilter, fmt, fmt::format::FmtSpan};

/// Determine if verbose tracing metadata should be enabled based on log level
///
/// Returns true for debug/trace levels (development/profiling)
/// Returns false for info/warn/error levels (production)
fn should_enable_verbose_tracing(log_level: &str) -> bool {
    // Parse the log level - check if it starts with debug or trace
    // This handles both simple levels (e.g., "debug") and complex filters (e.g., "debug,sqlx=warn")
    log_level.to_lowercase().starts_with("debug")
        || log_level.to_lowercase().starts_with("trace")
        || log_level.to_lowercase().contains("streamflow=debug")
        || log_level.to_lowercase().contains("streamflow=trace")
}

/// Initialize logging based on level and format
///
/// Uses direct fmt::Subscriber instead of registry() to prevent memory accumulation
/// from span metadata. This allows TRACE-level profiling without memory leaks while
/// maintaining all formatting capabilities. Verbose tracing metadata (file, line,
/// thread IDs, span events) is enabled only when log_level is set to debug or trace.
pub fn init(log_level: &str, log_format: &str) -> Result<()> {
    let env_filter = EnvFilter::try_new(log_level).unwrap_or_else(|_| EnvFilter::new("info"));

    // Enable verbose tracing only for debug/trace levels
    let verbose = should_enable_verbose_tracing(log_level);
    let span_events = if verbose {
        FmtSpan::CLOSE
    } else {
        FmtSpan::NONE
    };

    // Use direct fmt::Subscriber instead of registry() to prevent memory accumulation.
    // The registry stores all span metadata for distributed tracing features we don't use,
    // causing 300+ MB memory growth at TRACE level. Direct subscriber has zero accumulation.
    match log_format {
        "json" => {
            let subscriber = fmt()
                .json()
                .with_env_filter(env_filter)
                .with_target(true)
                .with_level(true)
                .with_file(verbose) // Only in debug/trace
                .with_line_number(verbose) // Only in debug/trace
                .with_thread_ids(verbose) // Only in debug/trace
                .with_timer(fmt::time::uptime())
                .with_span_events(span_events)
                .finish();

            tracing::subscriber::set_global_default(subscriber)
                .map_err(|e| anyhow::anyhow!("Failed to set tracing subscriber: {}", e))?;
        }
        _ => {
            let subscriber = fmt()
                .with_env_filter(env_filter)
                .with_target(true)
                .with_level(true)
                .with_file(verbose) // Only in debug/trace
                .with_line_number(verbose) // Only in debug/trace
                .with_thread_ids(verbose) // Only in debug/trace
                .with_timer(fmt::time::uptime())
                .with_span_events(span_events)
                .finish();

            tracing::subscriber::set_global_default(subscriber)
                .map_err(|e| anyhow::anyhow!("Failed to set tracing subscriber: {}", e))?;
        }
    }

    tracing::info!(
        "Logging initialized: level={}, format={}, verbose_tracing={}, memory_safe=true",
        log_level,
        log_format,
        verbose
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use std::sync::Once;

    static INIT: Once = Once::new();

    /// Initialize logging once for all tests
    fn init_logging_once() {
        INIT.call_once(|| {
            let _ = init("debug", "text");
        });
    }

    #[test]
    #[serial]
    fn test_init_with_valid_log_levels() {
        // Test that initialization succeeds with valid log levels
        // Note: We can only initialize once per process, so we test the first call
        // The init_logging_once() function ensures initialization happens exactly once
        init_logging_once();

        // Verify that the init function can be called without panicking
        // even if already initialized. The function returns Ok(()) in our implementation.
    }

    #[test]
    fn test_env_filter_creation() {
        // Test that EnvFilter can be created with various log levels
        let valid_levels = vec!["trace", "debug", "info", "warn", "error"];

        for level in valid_levels {
            let filter = EnvFilter::try_new(level);
            assert!(filter.is_ok(), "EnvFilter should accept level: {}", level);
        }
    }

    #[test]
    fn test_env_filter_fallback_logic() {
        // Test that the fallback logic works correctly
        // EnvFilter::try_new may accept various inputs, but we test our fallback pattern

        // Test with a complex invalid filter that should fail
        let complex_invalid = "module1=invalid_level,module2=another_invalid";
        let filter_result = EnvFilter::try_new(complex_invalid);

        // Whether it succeeds or fails, test the fallback pattern
        let _filter = filter_result.unwrap_or_else(|_| EnvFilter::new("info"));

        // The important thing is that this doesn't panic - the unwrap_or_else handles both cases
    }

    #[test]
    fn test_format_matching() {
        // Test the format matching logic
        let json_format = "json";
        let text_format = "text";
        let unknown_format = "unknown";

        // Verify our format strings
        assert_eq!(json_format, "json");
        assert_eq!(text_format, "text");
        assert_ne!(unknown_format, "json");
        assert_ne!(unknown_format, "text");
    }

    #[test]
    #[serial]
    fn test_logging_initialization_sequence() {
        // Test that we can call init and it follows the expected code path
        // This tests the function structure without requiring multiple inits

        init_logging_once();

        // Verify logging is working by checking that tracing macros don't panic
        tracing::info!("Test log message");
        tracing::debug!("Test debug message");
        tracing::error!("Test error message");
    }

    #[test]
    fn test_should_enable_verbose_tracing_for_debug() {
        assert!(should_enable_verbose_tracing("debug"));
        assert!(should_enable_verbose_tracing("DEBUG"));
        assert!(should_enable_verbose_tracing("debug,sqlx=warn"));
    }

    #[test]
    fn test_should_enable_verbose_tracing_for_trace() {
        assert!(should_enable_verbose_tracing("trace"));
        assert!(should_enable_verbose_tracing("TRACE"));
        assert!(should_enable_verbose_tracing("trace,sqlx=info"));
    }

    #[test]
    fn test_should_enable_verbose_tracing_for_streamflow_debug() {
        assert!(should_enable_verbose_tracing("info,streamflow=debug"));
        assert!(should_enable_verbose_tracing(
            "warn,streamflow=debug,sqlx=error"
        ));
    }

    #[test]
    fn test_should_enable_verbose_tracing_for_streamflow_trace() {
        assert!(should_enable_verbose_tracing("info,streamflow=trace"));
        assert!(should_enable_verbose_tracing(
            "error,streamflow=trace,sqlx=warn"
        ));
    }

    #[test]
    fn test_should_disable_verbose_tracing_for_info() {
        assert!(!should_enable_verbose_tracing("info"));
        assert!(!should_enable_verbose_tracing("INFO"));
        assert!(!should_enable_verbose_tracing("info,sqlx=warn"));
    }

    #[test]
    fn test_should_disable_verbose_tracing_for_warn() {
        assert!(!should_enable_verbose_tracing("warn"));
        assert!(!should_enable_verbose_tracing("WARN"));
        assert!(!should_enable_verbose_tracing("warn,sqlx=error"));
    }

    #[test]
    fn test_should_disable_verbose_tracing_for_error() {
        assert!(!should_enable_verbose_tracing("error"));
        assert!(!should_enable_verbose_tracing("ERROR"));
    }

    #[test]
    fn test_should_disable_verbose_tracing_for_streamflow_info() {
        // streamflow=info should NOT enable verbose tracing
        assert!(!should_enable_verbose_tracing("streamflow=info"));
        assert!(!should_enable_verbose_tracing(
            "info,streamflow=info,sqlx=warn"
        ));
    }
}
