// core/tests/backoff_tests.rs
//! Unit tests for AdaptiveBackoff

use kruxiaflow_core::orchestrator::backoff::AdaptiveBackoff;
use std::time::Duration;

#[test]
fn test_backoff_starts_at_min() {
    let min = Duration::from_millis(10);
    let max = Duration::from_secs(5);
    let backoff = AdaptiveBackoff::new(min, max, 2.0);

    assert_eq!(backoff.current(), min);
}

#[test]
fn test_backoff_increases_exponentially() {
    let min = Duration::from_millis(10);
    let max = Duration::from_secs(5);
    let mut backoff = AdaptiveBackoff::new(min, max, 2.0);

    // First increase: 10ms * 2 = 20ms
    backoff.increase();
    assert_eq!(backoff.current(), Duration::from_millis(20));

    // Second increase: 20ms * 2 = 40ms
    backoff.increase();
    assert_eq!(backoff.current(), Duration::from_millis(40));

    // Third increase: 40ms * 2 = 80ms
    backoff.increase();
    assert_eq!(backoff.current(), Duration::from_millis(80));
}

#[test]
fn test_backoff_caps_at_max() {
    let min = Duration::from_millis(100);
    let max = Duration::from_millis(500);
    let mut backoff = AdaptiveBackoff::new(min, max, 2.0);

    // 100 -> 200 -> 400 -> 800 (capped at 500)
    backoff.increase(); // 200
    backoff.increase(); // 400
    backoff.increase(); // 500 (capped)

    assert_eq!(backoff.current(), max);

    // Further increases should stay at max
    backoff.increase();
    assert_eq!(backoff.current(), max);
}

#[test]
fn test_backoff_reset_to_min() {
    let min = Duration::from_millis(10);
    let max = Duration::from_secs(5);
    let mut backoff = AdaptiveBackoff::new(min, max, 2.0);

    // Increase several times
    backoff.increase();
    backoff.increase();
    backoff.increase();
    assert_ne!(backoff.current(), min);

    // Reset should go back to min
    backoff.reset();
    assert_eq!(backoff.current(), min);
}

#[test]
fn test_backoff_with_different_multipliers() {
    let min = Duration::from_millis(10);
    let max = Duration::from_secs(10);

    // Test with multiplier 1.5
    let mut backoff = AdaptiveBackoff::new(min, max, 1.5);
    backoff.increase();
    assert_eq!(backoff.current(), Duration::from_millis(15));
    backoff.increase();
    // 15 * 1.5 = 22.5ms
    assert!(
        backoff.current() >= Duration::from_millis(22)
            && backoff.current() <= Duration::from_millis(23)
    );

    // Test with multiplier 3.0
    let mut backoff = AdaptiveBackoff::new(min, max, 3.0);
    backoff.increase();
    assert_eq!(backoff.current(), Duration::from_millis(30));
    backoff.increase();
    assert_eq!(backoff.current(), Duration::from_millis(90));
}

#[test]
fn test_backoff_min_equals_max() {
    let duration = Duration::from_millis(100);
    let mut backoff = AdaptiveBackoff::new(duration, duration, 2.0);

    assert_eq!(backoff.current(), duration);

    // Increases should not change the duration
    backoff.increase();
    assert_eq!(backoff.current(), duration);

    backoff.reset();
    assert_eq!(backoff.current(), duration);
}

#[test]
fn test_backoff_typical_usage_pattern() {
    // Typical usage: 10ms to 5s with 2x multiplier
    let min = Duration::from_millis(10);
    let max = Duration::from_secs(5);
    let mut backoff = AdaptiveBackoff::new(min, max, 2.0);

    // No events found - backoff increases
    for _ in 0..10 {
        backoff.increase();
    }

    // Should be at max
    assert_eq!(backoff.current(), max);

    // Events found - reset to fast polling
    backoff.reset();
    assert_eq!(backoff.current(), min);
}

#[test]
fn test_backoff_with_fractional_multiplier() {
    let min = Duration::from_millis(100);
    let max = Duration::from_secs(10);
    let mut backoff = AdaptiveBackoff::new(min, max, 1.1);

    // Small multiplier should increase slowly
    backoff.increase();
    assert_eq!(backoff.current(), Duration::from_millis(110));
    backoff.increase();
    assert_eq!(backoff.current(), Duration::from_millis(121));
}

#[test]
fn test_backoff_with_large_max() {
    let min = Duration::from_millis(10);
    let max = Duration::from_secs(3600); // 1 hour
    let mut backoff = AdaptiveBackoff::new(min, max, 2.0);

    // Even with many increases, should cap at max
    for _ in 0..50 {
        backoff.increase();
    }

    assert!(backoff.current() <= max);
    assert_eq!(backoff.current(), max);
}

#[test]
fn test_backoff_sequence_realistic() {
    // Realistic sequence for event polling:
    // Start fast, slow down when idle, reset when active
    let min = Duration::from_millis(10);
    let max = Duration::from_secs(5);
    let mut backoff = AdaptiveBackoff::new(min, max, 2.0);

    // Initial state
    assert_eq!(backoff.current(), Duration::from_millis(10));

    // First poll: no events
    backoff.increase();
    assert_eq!(backoff.current(), Duration::from_millis(20));

    // Second poll: no events
    backoff.increase();
    assert_eq!(backoff.current(), Duration::from_millis(40));

    // Third poll: events found!
    backoff.reset();
    assert_eq!(backoff.current(), Duration::from_millis(10));

    // Fourth poll: more events
    backoff.reset();
    assert_eq!(backoff.current(), Duration::from_millis(10));

    // Fifth poll: no events
    backoff.increase();
    assert_eq!(backoff.current(), Duration::from_millis(20));
}

#[test]
fn test_backoff_multiple_reset_cycles() {
    let min = Duration::from_millis(10);
    let max = Duration::from_secs(5);
    let mut backoff = AdaptiveBackoff::new(min, max, 2.0);

    for _ in 0..3 {
        // Increase to max
        for _ in 0..20 {
            backoff.increase();
        }
        assert_eq!(backoff.current(), max);

        // Reset to min
        backoff.reset();
        assert_eq!(backoff.current(), min);
    }
}

#[test]
fn test_backoff_edge_case_zero_multiplier() {
    let min = Duration::from_millis(100);
    let max = Duration::from_secs(10);
    let mut backoff = AdaptiveBackoff::new(min, max, 0.0);

    // With 0 multiplier, backoff should stay at 0 (or min if clamped)
    backoff.increase();
    // 100ms * 0 = 0, but clamped to at least Duration::ZERO
    assert_eq!(backoff.current(), Duration::ZERO);
}

#[test]
fn test_backoff_boundary_conditions() {
    // Test with very small durations
    let min = Duration::from_nanos(1);
    let max = Duration::from_nanos(1000);
    let mut backoff = AdaptiveBackoff::new(min, max, 2.0);

    backoff.increase();
    assert!(backoff.current() >= min && backoff.current() <= max);

    // Test with very large durations
    let min = Duration::from_secs(1);
    let max = Duration::from_secs(86400); // 1 day
    let mut backoff = AdaptiveBackoff::new(min, max, 2.0);

    for _ in 0..50 {
        backoff.increase();
    }
    assert_eq!(backoff.current(), max);
}
