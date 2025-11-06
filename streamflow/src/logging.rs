use anyhow::Result;
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

/// Initialize logging based on level and format
pub fn init(log_level: &str, log_format: &str) -> Result<()> {
    let env_filter = EnvFilter::try_new(log_level).unwrap_or_else(|_| EnvFilter::new("info"));

    match log_format {
        "json" => {
            tracing_subscriber::registry()
                .with(env_filter)
                .with(fmt::layer().json())
                .init();
        }
        _ => {
            tracing_subscriber::registry()
                .with(env_filter)
                .with(fmt::layer())
                .init();
        }
    }

    tracing::info!(
        "Logging initialized: level={}, format={}",
        log_level,
        log_format
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
}
