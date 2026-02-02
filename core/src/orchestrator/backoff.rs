use std::time::Duration;

/// Adaptive backoff for event polling
/// Starts at min duration, increases by multiplier when no events found,
/// resets to min when events are found, caps at max duration
pub struct AdaptiveBackoff {
    min: Duration,
    max: Duration,
    current: Duration,
    multiplier: f64,
}

impl AdaptiveBackoff {
    pub fn new(min: Duration, max: Duration, multiplier: f64) -> Self {
        Self {
            min,
            max,
            current: min,
            multiplier,
        }
    }

    pub fn current(&self) -> Duration {
        self.current
    }

    pub fn increase(&mut self) {
        let next = Duration::from_secs_f64(self.current.as_secs_f64() * self.multiplier);
        self.current = next.min(self.max);
    }

    pub fn reset(&mut self) {
        self.current = self.min;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_starts_at_min() {
        let backoff = AdaptiveBackoff::new(Duration::from_millis(10), Duration::from_secs(5), 2.0);
        assert_eq!(backoff.current(), Duration::from_millis(10));
    }

    #[test]
    fn test_increase_doubles() {
        let mut backoff =
            AdaptiveBackoff::new(Duration::from_millis(10), Duration::from_secs(5), 2.0);
        backoff.increase();
        assert_eq!(backoff.current(), Duration::from_millis(20));
        backoff.increase();
        assert_eq!(backoff.current(), Duration::from_millis(40));
    }

    #[test]
    fn test_increase_caps_at_max() {
        let mut backoff =
            AdaptiveBackoff::new(Duration::from_secs(1), Duration::from_secs(5), 10.0);
        backoff.increase(); // 1 * 10 = 10, capped at 5
        assert_eq!(backoff.current(), Duration::from_secs(5));
        backoff.increase(); // 5 * 10 = 50, capped at 5
        assert_eq!(backoff.current(), Duration::from_secs(5));
    }

    #[test]
    fn test_reset_returns_to_min() {
        let mut backoff =
            AdaptiveBackoff::new(Duration::from_millis(10), Duration::from_secs(5), 2.0);
        backoff.increase();
        backoff.increase();
        assert!(backoff.current() > Duration::from_millis(10));
        backoff.reset();
        assert_eq!(backoff.current(), Duration::from_millis(10));
    }
}
