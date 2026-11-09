//! `LISTEN`/`NOTIFY` wake-up for event consumers.
//!
//! An `AFTER INSERT ... FOR EACH STATEMENT` trigger on `workflow_events` runs
//! `pg_notify('workflow_events', '')`. Postgres delivers the notification when
//! the inserting transaction commits, to every connection listening at that
//! moment. [`EventWake`] holds one `PgListener` per process and fans each
//! notification out to its consumers through a `tokio::sync::watch` counter.
//!
//! A wake is only ever a hint to poll. It carries no data, and every consumer
//! is correct with it disabled:
//!
//! 1. The payload is empty and never read. A wake means "poll from your
//!    cursor until empty".
//! 2. Every wake and every backoff timer tick runs the same catch-up poll, so
//!    a missed notification only delays an event until the next tick.
//! 3. A listener reconnect counts as a wake, because notifications sent while
//!    the connection was down are gone.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use sqlx::PgPool;
use sqlx::postgres::PgListener;
use tokio::sync::watch;
use tokio::task::AbortHandle;

use super::Result;

/// The channel the `workflow_events_notify` trigger notifies on
pub const EVENT_NOTIFY_CHANNEL: &str = "workflow_events";

/// Environment variable selecting the wake mode (`notify` or `poll`)
pub const EVENT_WAKE_ENV: &str = "KRUXIAFLOW_EVENT_WAKE";

/// How event consumers learn that new events may be available
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EventWakeMode {
    /// `LISTEN` for commit notifications, with the poll timer as a safety net
    #[default]
    Notify,
    /// Poll on the backoff timer alone
    Poll,
}

impl EventWakeMode {
    /// Parse `notify` / `poll` (case-insensitive)
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "notify" => Some(Self::Notify),
            "poll" => Some(Self::Poll),
            _ => None,
        }
    }

    /// Read [`EVENT_WAKE_ENV`]; unset means `notify`, an unknown value logs a
    /// warning and falls back to `notify`
    pub fn from_env() -> Self {
        match std::env::var(EVENT_WAKE_ENV) {
            Ok(value) => Self::parse(&value).unwrap_or_else(|| {
                tracing::warn!(
                    value = %value,
                    "Unknown {} value (expected notify or poll); using notify",
                    EVENT_WAKE_ENV
                );
                Self::Notify
            }),
            Err(_) => Self::Notify,
        }
    }
}

impl std::fmt::Display for EventWakeMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Notify => write!(f, "notify"),
            Self::Poll => write!(f, "poll"),
        }
    }
}

/// What ended a [`WakeReceiver::wait`]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wake {
    /// A notification (or a listener reconnect) arrived
    Woken,
    /// The timeout elapsed first
    TimedOut,
}

/// One notification from the listener connection
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListenerEvent {
    /// A notification on the channel
    Notification,
    /// The connection was lost and re-established; notifications sent in
    /// between are gone
    Reconnected,
}

/// The notification source behind [`EventWake`]: a `PgListener` in
/// production, a stub in tests
#[async_trait]
pub trait NotificationSource: Send + 'static {
    async fn recv(&mut self) -> Result<ListenerEvent>;
}

#[async_trait]
impl NotificationSource for PgListener {
    async fn recv(&mut self) -> Result<ListenerEvent> {
        // try_recv returns Ok(None) when the connection was lost; the next
        // call reconnects and re-issues LISTEN
        match self.try_recv().await? {
            Some(_) => Ok(ListenerEvent::Notification),
            None => Ok(ListenerEvent::Reconnected),
        }
    }
}

struct Inner {
    sender: Arc<watch::Sender<u64>>,
    task: AbortHandle,
}

impl Drop for Inner {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// A process-wide wake-up signal for event consumers.
///
/// Cheap to clone. The listener task runs until the last `EventWake` and the
/// last [`WakeReceiver`] are dropped.
#[derive(Clone)]
pub struct EventWake {
    inner: Arc<Inner>,
}

impl EventWake {
    /// Open a dedicated listener connection on `pool`'s connect options and
    /// `LISTEN` on [`EVENT_NOTIFY_CHANNEL`].
    ///
    /// Fails if the connection or the `LISTEN` fails (no permission, a
    /// transaction-mode connection pooler); callers fall back to the timer.
    pub async fn connect(pool: &PgPool) -> Result<Self> {
        let mut listener = PgListener::connect_with(pool).await?;
        listener.listen(EVENT_NOTIFY_CHANNEL).await?;
        Ok(Self::from_source(listener))
    }

    /// Run the listener loop over any notification source
    pub fn from_source<S: NotificationSource>(source: S) -> Self {
        let sender = Arc::new(watch::channel(0u64).0);
        let task = tokio::spawn(run_listener(source, sender.clone()));
        Self {
            inner: Arc::new(Inner {
                sender,
                task: task.abort_handle(),
            }),
        }
    }

    /// A receiver that wakes on every notification from now on
    pub fn subscribe(&self) -> WakeReceiver {
        WakeReceiver {
            rx: self.inner.sender.subscribe(),
            _inner: self.inner.clone(),
        }
    }
}

impl std::fmt::Debug for EventWake {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventWake").finish_non_exhaustive()
    }
}

async fn run_listener<S: NotificationSource>(mut source: S, sender: Arc<watch::Sender<u64>>) {
    const RETRY_MIN: Duration = Duration::from_millis(100);
    const RETRY_MAX: Duration = Duration::from_secs(5);
    let mut retry = RETRY_MIN;

    loop {
        match source.recv().await {
            Ok(ListenerEvent::Notification) => {
                retry = RETRY_MIN;
                bump(&sender);
            }
            Ok(ListenerEvent::Reconnected) => {
                tracing::warn!(
                    "Event listener connection lost; reconnecting (consumers poll to catch up)"
                );
                retry = RETRY_MIN;
                bump(&sender);
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    retry_in_ms = retry.as_millis() as u64,
                    "Event listener error; consumers continue on the poll timer"
                );
                // Notifications may have been lost while the listener was
                // failing: wake consumers so they poll
                bump(&sender);
                tokio::time::sleep(retry).await;
                retry = (retry * 2).min(RETRY_MAX);
            }
        }
    }
}

fn bump(sender: &watch::Sender<u64>) {
    sender.send_modify(|n| *n = n.wrapping_add(1));
}

/// One consumer's view of an [`EventWake`]
pub struct WakeReceiver {
    rx: watch::Receiver<u64>,
    _inner: Arc<Inner>,
}

impl WakeReceiver {
    /// Wait for a notification or `timeout`, whichever comes first.
    ///
    /// A notification that arrived since the previous `wait` returned is not
    /// lost: it ends this wait immediately.
    pub async fn wait(&mut self, timeout: Duration) -> Wake {
        match tokio::time::timeout(timeout, self.rx.changed()).await {
            Ok(Ok(())) => Wake::Woken,
            // The sender is kept alive by `_inner`, so this cannot happen;
            // behave as a plain timer if it ever does
            Ok(Err(_)) => {
                tokio::time::sleep(timeout).await;
                Wake::TimedOut
            }
            Err(_) => Wake::TimedOut,
        }
    }
}

impl std::fmt::Debug for WakeReceiver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WakeReceiver").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::EventError;
    use tokio::sync::mpsc;

    /// Yields whatever the test sends; pending forever once the channel is
    /// drained and still open
    struct StubSource(mpsc::UnboundedReceiver<Result<ListenerEvent>>);

    #[async_trait]
    impl NotificationSource for StubSource {
        async fn recv(&mut self) -> Result<ListenerEvent> {
            match self.0.recv().await {
                Some(event) => event,
                None => std::future::pending().await,
            }
        }
    }

    fn stub() -> (mpsc::UnboundedSender<Result<ListenerEvent>>, EventWake) {
        let (tx, rx) = mpsc::unbounded_channel();
        (tx, EventWake::from_source(StubSource(rx)))
    }

    const SHORT: Duration = Duration::from_millis(50);
    const LONG: Duration = Duration::from_secs(2);

    #[tokio::test]
    async fn test_notification_wakes() {
        let (tx, wake) = stub();
        let mut rx = wake.subscribe();
        tx.send(Ok(ListenerEvent::Notification)).unwrap();
        assert_eq!(rx.wait(LONG).await, Wake::Woken);
    }

    #[tokio::test]
    async fn test_times_out_without_notification() {
        let (_tx, wake) = stub();
        let mut rx = wake.subscribe();
        assert_eq!(rx.wait(SHORT).await, Wake::TimedOut);
    }

    #[tokio::test]
    async fn test_reconnect_counts_as_wake() {
        let (tx, wake) = stub();
        let mut rx = wake.subscribe();
        tx.send(Ok(ListenerEvent::Reconnected)).unwrap();
        assert_eq!(rx.wait(LONG).await, Wake::Woken);
    }

    #[tokio::test]
    async fn test_listener_error_wakes_and_loop_continues() {
        let (tx, wake) = stub();
        let mut rx = wake.subscribe();
        tx.send(Err(EventError::Invalid("connection refused".into())))
            .unwrap();
        assert_eq!(rx.wait(LONG).await, Wake::Woken);
        // After the retry sleep the loop keeps delivering notifications
        tx.send(Ok(ListenerEvent::Notification)).unwrap();
        assert_eq!(rx.wait(LONG).await, Wake::Woken);
    }

    #[tokio::test]
    async fn test_wake_before_wait_is_not_lost() {
        let (tx, wake) = stub();
        let mut rx = wake.subscribe();
        tx.send(Ok(ListenerEvent::Notification)).unwrap();
        // Let the listener task deliver it before anyone waits
        tokio::time::sleep(SHORT).await;
        assert_eq!(rx.wait(SHORT).await, Wake::Woken);
        // ...and it is consumed exactly once
        assert_eq!(rx.wait(SHORT).await, Wake::TimedOut);
    }

    #[tokio::test]
    async fn test_every_receiver_wakes() {
        let (tx, wake) = stub();
        let mut a = wake.subscribe();
        let mut b = wake.subscribe();
        tx.send(Ok(ListenerEvent::Notification)).unwrap();
        assert_eq!(a.wait(LONG).await, Wake::Woken);
        assert_eq!(b.wait(LONG).await, Wake::Woken);
    }

    #[tokio::test]
    async fn test_receiver_outlives_event_wake() {
        let (tx, wake) = stub();
        let mut rx = wake.subscribe();
        drop(wake);
        tx.send(Ok(ListenerEvent::Notification)).unwrap();
        assert_eq!(rx.wait(LONG).await, Wake::Woken);
    }

    #[test]
    fn test_mode_parse() {
        assert_eq!(EventWakeMode::parse("notify"), Some(EventWakeMode::Notify));
        assert_eq!(EventWakeMode::parse(" POLL "), Some(EventWakeMode::Poll));
        assert_eq!(EventWakeMode::parse("listen"), None);
        assert_eq!(EventWakeMode::default(), EventWakeMode::Notify);
        assert_eq!(EventWakeMode::Poll.to_string(), "poll");
    }
}
