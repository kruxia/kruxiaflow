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

/// How long an event consumer waits before its next poll, once a poll has
/// returned nothing.
///
/// The first empty poll after a batch or a wake is followed by a short wait
/// ([`linger`](Self::linger)) and no question asked. On a busy consumer the
/// next wake arrives within that wait, so the hot path makes no extra query.
///
/// When that wait runs out and the poll is still empty, the consumer asks
/// the event source whether committed events lie past its cursor and passes
/// the answer to [`caught_up`](Self::caught_up):
///
/// - **Events are pending** (committed, but withheld by the poll): retry
///   from the minimum interval, growing up to the timer-only maximum.
///   Nothing announces the moment they become deliverable, so the wake
///   cannot help and the longer wake-mode idle wait does not apply.
/// - **Nothing is pending, wake attached**: wait the full idle maximum. Any
///   event that commits later sends a wake, and the timer covers a lost one.
/// - **Nothing is pending, no wake**: the timer is the only signal, so keep
///   backing off to the idle maximum.
///
/// So no wait longer than the minimum is ever chosen without asking first.
pub struct ConsumerPacing {
    backoff: AdaptiveBackoff,
    pending_max: Duration,
    idle_max: Duration,
    has_wake: bool,
    /// No empty poll yet since the last batch or wake
    fresh: bool,
    pending: bool,
}

impl ConsumerPacing {
    /// `timer_max` is the idle maximum without a wake and the retry cap
    /// while events are pending; `idle_max` is the idle maximum in the
    /// consumer's actual mode (longer than `timer_max` with a wake).
    pub fn new(
        min: Duration,
        timer_max: Duration,
        idle_max: Duration,
        multiplier: f64,
        has_wake: bool,
    ) -> Self {
        Self {
            backoff: AdaptiveBackoff::new(min, idle_max, multiplier),
            pending_max: timer_max.min(idle_max),
            idle_max,
            has_wake,
            fresh: true,
            pending: false,
        }
    }

    /// The consumer processed a batch: start again from the minimum
    pub fn progressed(&mut self) {
        self.backoff.reset();
        self.fresh = true;
        self.pending = false;
    }

    /// A wake arrived during the wait: start again from the minimum
    pub fn woken(&mut self) {
        self.backoff.reset();
        self.fresh = true;
    }

    /// The wait after a failed poll or cursor write
    pub fn failed(&mut self) -> Duration {
        self.backoff.increase();
        self.backoff.current()
    }

    /// Called after a poll that returned nothing. `Some` on the first one
    /// since a batch or a wake: wait that long (the minimum interval)
    /// without asking the event source anything. `None` on the following
    /// ones: ask whether events are pending and call
    /// [`caught_up`](Self::caught_up).
    pub fn linger(&mut self) -> Option<Duration> {
        if !self.fresh {
            return None;
        }
        self.fresh = false;
        self.backoff.reset();
        let interval = self.backoff.current();
        self.backoff.increase();
        Some(interval)
    }

    /// The wait after an empty poll for which [`linger`](Self::linger)
    /// returned `None`. `pending` is the event source's answer to whether
    /// committed events lie past the cursor.
    pub fn caught_up(&mut self, pending: bool) -> Duration {
        self.fresh = false;
        if pending {
            if !self.pending {
                // First sight of withheld events: retry promptly
                self.backoff.reset();
            }
            self.pending = true;
            let interval = self.backoff.current().min(self.pending_max);
            self.backoff.increase();
            return interval;
        }

        self.pending = false;
        if self.has_wake {
            self.idle_max
        } else {
            let interval = self.backoff.current();
            self.backoff.increase();
            interval
        }
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

    const MIN: Duration = Duration::from_millis(50);
    const TIMER_MAX: Duration = Duration::from_secs(1);
    const WAKE_MAX: Duration = Duration::from_secs(5);

    fn with_wake() -> ConsumerPacing {
        ConsumerPacing::new(MIN, TIMER_MAX, WAKE_MAX, 2.0, true)
    }

    fn timer_only() -> ConsumerPacing {
        ConsumerPacing::new(MIN, TIMER_MAX, TIMER_MAX, 2.0, false)
    }

    /// A pacing whose first empty poll has already been lingered on
    fn lingered(mut pacing: ConsumerPacing) -> ConsumerPacing {
        assert_eq!(pacing.linger(), Some(MIN));
        pacing
    }

    #[test]
    fn test_pacing_first_empty_poll_lingers_without_asking() {
        let mut pacing = with_wake();
        assert_eq!(pacing.linger(), Some(MIN));
        // The next empty poll has to ask
        assert_eq!(pacing.linger(), None);
        assert_eq!(pacing.linger(), None);

        // A batch or a wake makes the next empty poll a first one again
        pacing.progressed();
        assert_eq!(pacing.linger(), Some(MIN));
        assert_eq!(pacing.linger(), None);
        pacing.woken();
        assert_eq!(pacing.linger(), Some(MIN));

        // Asking ends the linger too
        pacing.woken();
        pacing.caught_up(false);
        assert_eq!(pacing.linger(), None);
    }

    #[test]
    fn test_pacing_with_wake_and_nothing_pending_waits_the_idle_maximum() {
        let mut pacing = lingered(with_wake());
        assert_eq!(pacing.caught_up(false), WAKE_MAX);
        assert_eq!(pacing.caught_up(false), WAKE_MAX);

        // A wake that brought nothing costs one short wait, not a run of them
        pacing.woken();
        assert_eq!(pacing.linger(), Some(MIN));
        assert_eq!(pacing.linger(), None);
        assert_eq!(pacing.caught_up(false), WAKE_MAX);
    }

    #[test]
    fn test_pacing_timer_only_backs_off_to_the_timer_maximum() {
        let mut pacing = lingered(timer_only());
        let waits: Vec<_> = (0..6).map(|_| pacing.caught_up(false)).collect();
        // Continuing from the 50 ms linger
        assert_eq!(
            waits,
            [100, 200, 400, 800, 1000, 1000].map(Duration::from_millis)
        );
    }

    #[test]
    fn test_pacing_pending_events_retry_up_to_the_timer_maximum_despite_a_wake() {
        let mut pacing = lingered(with_wake());
        let waits: Vec<_> = (0..7).map(|_| pacing.caught_up(true)).collect();
        // Capped at the timer maximum, not the 5 s idle maximum
        assert_eq!(
            waits,
            [50, 100, 200, 400, 800, 1000, 1000].map(Duration::from_millis)
        );
    }

    #[test]
    fn test_pacing_first_sight_of_pending_events_retries_from_the_minimum() {
        // Timer-only consumer, fully backed off, then events turn up held back
        let mut pacing = lingered(timer_only());
        for _ in 0..10 {
            pacing.caught_up(false);
        }
        assert_eq!(pacing.caught_up(false), TIMER_MAX);
        assert_eq!(pacing.caught_up(true), MIN);
        assert_eq!(pacing.caught_up(true), MIN * 2);

        // Same for a consumer idling on its wake timer
        let mut pacing = lingered(with_wake());
        assert_eq!(pacing.caught_up(false), WAKE_MAX);
        assert_eq!(pacing.caught_up(true), MIN);
    }

    #[test]
    fn test_pacing_pending_retries_keep_growing_until_the_events_are_delivered() {
        let mut pacing = lingered(with_wake());
        assert_eq!(pacing.caught_up(true), MIN);
        assert_eq!(pacing.caught_up(true), MIN * 2);
        // Still pending: no reset
        assert_eq!(pacing.caught_up(true), MIN * 4);

        // A wake restarts the retries (something changed)
        pacing.woken();
        assert_eq!(pacing.linger(), Some(MIN));
        assert_eq!(pacing.caught_up(true), MIN * 2);

        // Delivered, then held back again: from the minimum
        pacing.progressed();
        assert_eq!(pacing.linger(), Some(MIN));
        assert_eq!(pacing.caught_up(true), MIN);
    }

    #[test]
    fn test_pacing_returns_to_idle_once_nothing_is_pending() {
        let mut pacing = lingered(with_wake());
        pacing.caught_up(true);
        pacing.caught_up(true);
        assert_eq!(pacing.caught_up(false), WAKE_MAX);
        // And a later hold starts from the minimum again
        assert_eq!(pacing.caught_up(true), MIN);
    }

    #[test]
    fn test_pacing_failures_back_off_to_the_idle_maximum() {
        let mut pacing = with_wake();
        let waits: Vec<_> = (0..8).map(|_| pacing.failed()).collect();
        assert_eq!(
            waits,
            [100, 200, 400, 800, 1600, 3200, 5000, 5000].map(Duration::from_millis)
        );
        pacing.progressed();
        assert_eq!(pacing.failed(), MIN * 2);
    }

    #[test]
    fn test_pacing_pending_cap_never_exceeds_the_idle_maximum() {
        // A wake-mode idle maximum configured below the timer maximum
        let mut pacing = lingered(ConsumerPacing::new(
            MIN,
            TIMER_MAX,
            Duration::from_millis(300),
            2.0,
            true,
        ));
        let waits: Vec<_> = (0..5).map(|_| pacing.caught_up(true)).collect();
        assert_eq!(waits, [50, 100, 200, 300, 300].map(Duration::from_millis));
    }
}
