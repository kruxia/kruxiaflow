//! `LISTEN`/`NOTIFY` wake-ups: for event consumers, and for workers waiting
//! on the activity queue.
//!
//! # Announcing
//!
//! A writer that has committed an event, or an activity a worker can claim,
//! tells the process's [`Announcer`]. The announcer wakes the process's own
//! consumers in memory at once, and queues a notification for every other
//! process. One sender task per process sends what is queued as a single
//! `pg_notify` statement, on its own connection; announcements made while a
//! send is in flight are sent together when it returns. So there is at most
//! one notifying transaction per process at a time, however many writers
//! there are: PostgreSQL serializes the commits of notifying transactions on
//! one database-wide lock, and a notification sent from inside every writing
//! transaction made those commits queue behind one another under load. The
//! sender's connection commits with `synchronous_commit = off`: its only
//! write is the notification, which is a hint.
//!
//! The application is the only writer of `workflow_events` and of the
//! activity queue, so every write is announced this way. A row written by
//! anything else (SQL by hand, a process that has no announcer) wakes nobody
//! and is picked up on the consumers' poll timer.
//!
//! # Listening
//!
//! [`EventWake`] holds one `PgListener` per process and fans each
//! notification out to its consumers through a `tokio::sync::watch` counter.
//! Notifications this process's own sender sent are ignored (recognized by
//! the sending backend's process id): their wake was delivered in memory.
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
//!
//! The same listener connection also carries the activity queue's channel
//! (payload = worker type). [`ActivityWake`] hands out one receiver per
//! worker type; the API passes each wake on to the workers of that type
//! (`GET /api/v1/workers/wake`), which then poll for work. The same three
//! rules apply, with the payload used only to decide whom to wake.

use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use sqlx::postgres::{PgConnectOptions, PgConnection, PgListener};
use sqlx::{Connection, PgPool};
use tokio::sync::{Notify, watch};
use tokio::task::AbortHandle;

use super::Result;

/// The channel the `workflow_events_notify` trigger notifies on
pub const EVENT_NOTIFY_CHANNEL: &str = "workflow_events";

/// The channel the `activity_queue_notify` trigger notifies on, with the
/// worker type as payload
pub const ACTIVITY_NOTIFY_CHANNEL: &str = "activity_queue";

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
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListenerEvent {
    /// A notification on one of the channels, sent by the database backend
    /// `process_id`
    Notification {
        channel: String,
        payload: String,
        process_id: u32,
    },
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
            Some(notification) => Ok(ListenerEvent::Notification {
                channel: notification.channel().to_string(),
                payload: notification.payload().to_string(),
                process_id: notification.process_id(),
            }),
            None => Ok(ListenerEvent::Reconnected),
        }
    }
}

/// Where the announcer's notifications go: a dedicated connection in
/// production, a stub in tests
#[async_trait]
pub trait NotificationSink: Send + 'static {
    /// Send every `(channel, payload)` pair, in one transaction.
    ///
    /// Before sending from a new database backend, store its process id in
    /// `own_pid`, so that the listener can recognize this process's own
    /// notifications when they come back.
    async fn send(&mut self, notifications: &[(String, String)], own_pid: &AtomicU32)
    -> Result<()>;
}

/// The announcer's own connection, opened on first use and again after an
/// error
pub struct PgNotifySink {
    options: PgConnectOptions,
    connection: Option<PgConnection>,
}

impl PgNotifySink {
    /// A sink on `pool`'s connect options. The connection commits with
    /// `synchronous_commit = off`: a notification is a hint, so losing one in
    /// a crash costs nothing, and not waiting for a WAL flush keeps the
    /// database-wide notify lock held for as short a time as possible.
    pub fn new(pool: &PgPool) -> Self {
        let options = (*pool.connect_options())
            .clone()
            .application_name("kruxiaflow-announcer")
            .options([("synchronous_commit", "off")]);
        Self {
            options,
            connection: None,
        }
    }
}

#[async_trait]
impl NotificationSink for PgNotifySink {
    async fn send(
        &mut self,
        notifications: &[(String, String)],
        own_pid: &AtomicU32,
    ) -> Result<()> {
        if self.connection.is_none() {
            let mut connection = PgConnection::connect_with(&self.options).await?;
            let pid = sqlx::query_scalar!(r#"SELECT pg_backend_pid() AS "pid!""#)
                .fetch_one(&mut connection)
                .await?;
            own_pid.store(pid as u32, Ordering::Relaxed);
            self.connection = Some(connection);
        }
        let Some(connection) = self.connection.as_mut() else {
            return Ok(());
        };

        let (channels, payloads): (Vec<String>, Vec<String>) =
            notifications.iter().cloned().unzip();
        // pg_notify returns void; counting its rows keeps every call in the
        // plan and gives the statement a result the macro can type
        let sent = sqlx::query_scalar!(
            r#"
            SELECT count(*) AS "sent!"
            FROM (
                SELECT pg_notify(t.channel, t.payload)
                FROM unnest($1::text[], $2::text[]) AS t(channel, payload)
            ) AS notified
            "#,
            &channels,
            &payloads
        )
        .fetch_one(&mut *connection)
        .await;

        if let Err(e) = sent {
            // Start over on a fresh connection next time
            self.connection = None;
            return Err(e.into());
        }
        Ok(())
    }
}

/// Notifications waiting for the sender: whether events were committed, and
/// which worker types have new claimable work
#[derive(Default)]
struct Outbox {
    events: bool,
    workers: BTreeSet<String>,
}

impl Outbox {
    /// Everything queued, as `(channel, payload)` pairs, leaving it empty
    fn take(&mut self) -> Vec<(String, String)> {
        let mut notifications = Vec::with_capacity(self.workers.len() + 1);
        if std::mem::take(&mut self.events) {
            notifications.push((EVENT_NOTIFY_CHANNEL.to_string(), String::new()));
        }
        notifications.extend(
            std::mem::take(&mut self.workers)
                .into_iter()
                .map(|worker| (ACTIVITY_NOTIFY_CHANNEL.to_string(), worker)),
        );
        notifications
    }

    /// Queue a batch again after a failed send
    fn restore(&mut self, notifications: Vec<(String, String)>) {
        for (channel, payload) in notifications {
            if channel == EVENT_NOTIFY_CHANNEL {
                self.events = true;
            } else {
                self.workers.insert(payload);
            }
        }
    }
}

/// The wake counters the listener task writes and receivers read, and the
/// announcer's outbox
struct Shared {
    events: watch::Sender<u64>,
    /// One counter per worker type, created on first subscription
    activities: Mutex<HashMap<String, watch::Sender<u64>>>,
    outbox: Mutex<Outbox>,
    /// Tells the sender that the outbox has something in it. A `Notify`
    /// keeps one permit while the sender is busy, so an announcement made
    /// during a send is sent right after it.
    outbox_ready: Notify,
    /// Process id of the database backend the sender sends from (0 before
    /// its first send, or without a sender)
    own_pid: AtomicU32,
    /// Set when the last handle is dropped: the sender sends what is queued,
    /// then stops
    closed: AtomicBool,
}

impl Shared {
    fn new() -> Self {
        Self {
            events: watch::channel(0u64).0,
            activities: Mutex::new(HashMap::new()),
            outbox: Mutex::new(Outbox::default()),
            outbox_ready: Notify::new(),
            own_pid: AtomicU32::new(0),
            closed: AtomicBool::new(false),
        }
    }

    /// Whether a notification came from this process's own sender
    fn is_own(&self, process_id: u32) -> bool {
        let own = self.own_pid.load(Ordering::Relaxed);
        own != 0 && own == process_id
    }

    fn queue_events(&self) {
        if let Ok(mut outbox) = self.outbox.lock() {
            outbox.events = true;
        }
        self.outbox_ready.notify_one();
    }

    fn queue_activity(&self, worker: &str) {
        if let Ok(mut outbox) = self.outbox.lock()
            && !outbox.workers.contains(worker)
        {
            outbox.workers.insert(worker.to_string());
        }
        self.outbox_ready.notify_one();
    }

    fn take_outbox(&self) -> Vec<(String, String)> {
        self.outbox
            .lock()
            .map(|mut outbox| outbox.take())
            .unwrap_or_default()
    }

    fn restore_outbox(&self, notifications: Vec<(String, String)>) {
        if let Ok(mut outbox) = self.outbox.lock() {
            outbox.restore(notifications);
        }
    }

    fn wake_events(&self) {
        bump(&self.events);
    }

    /// Wake the subscribers of one worker type (no-op if there are none)
    fn wake_activity(&self, worker: &str) {
        if let Ok(activities) = self.activities.lock()
            && let Some(sender) = activities.get(worker)
        {
            bump(sender);
        }
    }

    /// Wake everything: notifications may have been lost
    fn wake_all(&self) {
        bump(&self.events);
        if let Ok(activities) = self.activities.lock() {
            activities.values().for_each(bump);
        }
    }

    fn subscribe_activity(&self, worker: &str) -> watch::Receiver<u64> {
        match self.activities.lock() {
            Ok(mut activities) => activities
                .entry(worker.to_string())
                .or_insert_with(|| watch::channel(0u64).0)
                .subscribe(),
            // A poisoned map cannot be used; hand out a receiver that only
            // ever times out, which leaves the subscriber on its timer
            Err(_) => watch::channel(0u64).1,
        }
    }
}

struct Inner {
    shared: Arc<Shared>,
    /// The listener task. The sender task is not aborted: it sends what is
    /// still queued when the last handle goes, then stops.
    listener: AbortHandle,
}

impl Drop for Inner {
    fn drop(&mut self) {
        self.listener.abort();
        self.shared.closed.store(true, Ordering::Release);
        self.shared.outbox_ready.notify_one();
    }
}

/// A process-wide wake-up signal for event consumers, and the
/// [`Announcer`] that sends this process's wakes to the others.
///
/// Cheap to clone. The listener and sender tasks run until the last
/// `EventWake`, [`Announcer`] and [`WakeReceiver`] are dropped; the sender
/// first sends whatever is still queued.
#[derive(Clone)]
pub struct EventWake {
    inner: Arc<Inner>,
}

impl EventWake {
    /// Open a dedicated listener connection on `pool`'s connect options and
    /// `LISTEN` on [`EVENT_NOTIFY_CHANNEL`] and [`ACTIVITY_NOTIFY_CHANNEL`];
    /// announcements go out on a second connection, opened on first use.
    ///
    /// Fails if the connection or the `LISTEN` fails (no permission, a
    /// transaction-mode connection pooler); callers fall back to the timer.
    pub async fn connect(pool: &PgPool) -> Result<Self> {
        let mut listener = PgListener::connect_with(pool).await?;
        listener
            .listen_all([EVENT_NOTIFY_CHANNEL, ACTIVITY_NOTIFY_CHANNEL])
            .await?;
        Ok(Self::from_parts(listener, Some(PgNotifySink::new(pool))))
    }

    /// Run the listener loop over any notification source. Announcements
    /// wake this process's receivers only.
    pub fn from_source<S: NotificationSource>(source: S) -> Self {
        Self::from_parts::<S, PgNotifySink>(source, None)
    }

    /// Run the listener loop over `source`, and send announcements to other
    /// processes through `sink` (with `None`, they wake this process only)
    pub fn from_parts<S: NotificationSource, K: NotificationSink>(
        source: S,
        sink: Option<K>,
    ) -> Self {
        let shared = Arc::new(Shared::new());
        let listener = tokio::spawn(run_listener(source, shared.clone())).abort_handle();
        if let Some(sink) = sink {
            tokio::spawn(run_sender(sink, shared.clone()));
        }
        Self {
            inner: Arc::new(Inner { shared, listener }),
        }
    }

    /// The announcer for writes made by this process
    pub fn announcer(&self) -> Announcer {
        Announcer {
            inner: Some(self.inner.clone()),
        }
    }

    /// A receiver that wakes on every event notification from now on
    pub fn subscribe(&self) -> WakeReceiver {
        WakeReceiver {
            rx: self.inner.shared.events.subscribe(),
            _inner: self.inner.clone(),
        }
    }

    /// The activity-queue side of the same listener
    pub fn activity_wake(&self) -> ActivityWake {
        ActivityWake {
            inner: self.inner.clone(),
        }
    }
}

/// A process-wide wake-up signal for workers of each worker type, fed by the
/// same listener connection as [`EventWake`].
///
/// Cheap to clone. A wake means "an activity for this worker type was written
/// as pending, or notifications may have been lost": the worker should poll.
#[derive(Clone)]
pub struct ActivityWake {
    inner: Arc<Inner>,
}

impl ActivityWake {
    /// A receiver that wakes on every notification for `worker` from now on
    pub fn subscribe(&self, worker: &str) -> WakeReceiver {
        WakeReceiver {
            rx: self.inner.shared.subscribe_activity(worker),
            _inner: self.inner.clone(),
        }
    }
}

/// Announces committed writes: events, and activities a worker can claim.
///
/// Call it after the write has committed, and only when a row was written.
/// It wakes this process's own receivers at once and queues one notification
/// for the other processes (see the module docs); it never blocks or fails.
///
/// Cheap to clone. [`Announcer::none`] (the default) announces nothing, for
/// a process without a wake (poll mode, or `LISTEN` failed) and for tests.
#[derive(Clone, Default)]
pub struct Announcer {
    inner: Option<Arc<Inner>>,
}

impl Announcer {
    /// An announcer that does nothing
    pub fn none() -> Self {
        Self::default()
    }

    /// Workflow events were committed
    pub fn events(&self) {
        if let Some(inner) = &self.inner {
            inner.shared.wake_events();
            inner.shared.queue_events();
        }
    }

    /// An activity for `worker` was committed as pending (due now or later)
    pub fn activity(&self, worker: &str) {
        if let Some(inner) = &self.inner {
            inner.shared.wake_activity(worker);
            inner.shared.queue_activity(worker);
        }
    }
}

impl std::fmt::Debug for Announcer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Announcer")
            .field("active", &self.inner.is_some())
            .finish()
    }
}

impl std::fmt::Debug for ActivityWake {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActivityWake").finish_non_exhaustive()
    }
}

impl std::fmt::Debug for EventWake {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventWake").finish_non_exhaustive()
    }
}

async fn run_listener<S: NotificationSource>(mut source: S, shared: Arc<Shared>) {
    const RETRY_MIN: Duration = Duration::from_millis(100);
    const RETRY_MAX: Duration = Duration::from_secs(5);
    let mut retry = RETRY_MIN;

    loop {
        match source.recv().await {
            Ok(ListenerEvent::Notification {
                channel,
                payload,
                process_id,
            }) => {
                retry = RETRY_MIN;
                if shared.is_own(process_id) {
                    // Sent by this process's announcer, which already woke
                    // this process's receivers
                    continue;
                }
                if channel == ACTIVITY_NOTIFY_CHANNEL {
                    shared.wake_activity(&payload);
                } else {
                    shared.wake_events();
                }
            }
            Ok(ListenerEvent::Reconnected) => {
                tracing::warn!(
                    "Event listener connection lost; reconnecting (consumers poll to catch up)"
                );
                retry = RETRY_MIN;
                shared.wake_all();
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    retry_in_ms = retry.as_millis() as u64,
                    "Event listener error; consumers continue on the poll timer"
                );
                // Notifications may have been lost while the listener was
                // failing: wake consumers so they poll
                shared.wake_all();
                tokio::time::sleep(retry).await;
                retry = (retry * 2).min(RETRY_MAX);
            }
        }
    }
}

/// Send what the announcer queued, one batch at a time, until the last
/// handle is dropped; then send what is still queued, and stop.
///
/// A failed send is queued again and retried on a backoff: the processes
/// it was for poll on their timers meanwhile, and this process's own
/// receivers were woken when the announcement was made.
async fn run_sender<K: NotificationSink>(mut sink: K, shared: Arc<Shared>) {
    const RETRY_MIN: Duration = Duration::from_millis(100);
    const RETRY_MAX: Duration = Duration::from_secs(5);
    let mut retry = RETRY_MIN;

    loop {
        shared.outbox_ready.notified().await;
        let closed = shared.closed.load(Ordering::Acquire);
        let batch = shared.take_outbox();
        if batch.is_empty() {
            if closed {
                return;
            }
            continue;
        }
        match sink.send(&batch, &shared.own_pid).await {
            Ok(()) if closed => return,
            Ok(()) => retry = RETRY_MIN,
            // Nobody is left to retry for
            Err(_) if closed => return,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    retry_in_ms = retry.as_millis() as u64,
                    "Could not send wake notifications; other processes poll on their timers"
                );
                shared.restore_outbox(batch);
                tokio::time::sleep(retry).await;
                retry = (retry * 2).min(RETRY_MAX);
                shared.outbox_ready.notify_one();
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
    /// Wait until a notification arrives, however long that takes.
    ///
    /// Like [`wait`](Self::wait), a notification that arrived since the
    /// previous wait returned ends this one immediately.
    pub async fn woken(&mut self) {
        if self.rx.changed().await.is_err() {
            // The sender is kept alive by `_inner`, so this cannot happen;
            // never resolve rather than spin
            std::future::pending::<()>().await;
        }
    }

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

    fn event_notification() -> ListenerEvent {
        ListenerEvent::Notification {
            channel: EVENT_NOTIFY_CHANNEL.to_string(),
            payload: String::new(),

            process_id: OTHER_PROCESS,
        }
    }

    /// A backend pid that is not this process's sender
    const OTHER_PROCESS: u32 = 4242;

    const SHORT: Duration = Duration::from_millis(50);
    const LONG: Duration = Duration::from_secs(2);

    #[tokio::test]
    async fn test_notification_wakes() {
        let (tx, wake) = stub();
        let mut rx = wake.subscribe();
        tx.send(Ok(event_notification())).unwrap();
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
        tx.send(Ok(event_notification())).unwrap();
        assert_eq!(rx.wait(LONG).await, Wake::Woken);
    }

    #[tokio::test]
    async fn test_wake_before_wait_is_not_lost() {
        let (tx, wake) = stub();
        let mut rx = wake.subscribe();
        tx.send(Ok(event_notification())).unwrap();
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
        tx.send(Ok(event_notification())).unwrap();
        assert_eq!(a.wait(LONG).await, Wake::Woken);
        assert_eq!(b.wait(LONG).await, Wake::Woken);
    }

    #[tokio::test]
    async fn test_receiver_outlives_event_wake() {
        let (tx, wake) = stub();
        let mut rx = wake.subscribe();
        drop(wake);
        tx.send(Ok(event_notification())).unwrap();
        assert_eq!(rx.wait(LONG).await, Wake::Woken);
    }

    fn activity_notification(worker: &str) -> ListenerEvent {
        ListenerEvent::Notification {
            channel: ACTIVITY_NOTIFY_CHANNEL.to_string(),
            payload: worker.to_string(),

            process_id: OTHER_PROCESS,
        }
    }

    #[tokio::test]
    async fn test_activity_notification_wakes_only_that_worker_type() {
        let (tx, wake) = stub();
        let activities = wake.activity_wake();
        let mut std_worker = activities.subscribe("std");
        let mut other_worker = activities.subscribe("other");

        tx.send(Ok(activity_notification("std"))).unwrap();
        assert_eq!(std_worker.wait(LONG).await, Wake::Woken);
        assert_eq!(other_worker.wait(SHORT).await, Wake::TimedOut);
    }

    #[tokio::test]
    async fn test_event_and_activity_channels_do_not_wake_each_other() {
        let (tx, wake) = stub();
        let mut events = wake.subscribe();
        let mut std_worker = wake.activity_wake().subscribe("std");

        tx.send(Ok(activity_notification("std"))).unwrap();
        assert_eq!(std_worker.wait(LONG).await, Wake::Woken);
        assert_eq!(events.wait(SHORT).await, Wake::TimedOut);

        tx.send(Ok(event_notification())).unwrap();
        assert_eq!(events.wait(LONG).await, Wake::Woken);
        assert_eq!(std_worker.wait(SHORT).await, Wake::TimedOut);
    }

    #[tokio::test]
    async fn test_notification_for_a_worker_type_nobody_listens_to_is_ignored() {
        let (tx, wake) = stub();
        let mut std_worker = wake.activity_wake().subscribe("std");

        tx.send(Ok(activity_notification("nobody"))).unwrap();
        assert_eq!(std_worker.wait(SHORT).await, Wake::TimedOut);
        // The listener keeps running
        tx.send(Ok(activity_notification("std"))).unwrap();
        assert_eq!(std_worker.wait(LONG).await, Wake::Woken);
    }

    #[tokio::test]
    async fn test_reconnect_and_error_wake_every_activity_subscriber() {
        let (tx, wake) = stub();
        let activities = wake.activity_wake();
        let mut events = wake.subscribe();
        let mut a = activities.subscribe("a");
        let mut b = activities.subscribe("b");

        tx.send(Ok(ListenerEvent::Reconnected)).unwrap();
        assert_eq!(events.wait(LONG).await, Wake::Woken);
        assert_eq!(a.wait(LONG).await, Wake::Woken);
        assert_eq!(b.wait(LONG).await, Wake::Woken);

        tx.send(Err(EventError::Invalid("connection reset".into())))
            .unwrap();
        assert_eq!(a.wait(LONG).await, Wake::Woken);
        assert_eq!(b.wait(LONG).await, Wake::Woken);
    }

    #[tokio::test]
    async fn test_every_subscriber_of_a_worker_type_wakes() {
        let (tx, wake) = stub();
        let activities = wake.activity_wake();
        let mut first = activities.subscribe("std");
        let mut second = activities.subscribe("std");

        tx.send(Ok(activity_notification("std"))).unwrap();
        assert_eq!(first.wait(LONG).await, Wake::Woken);
        assert_eq!(second.wait(LONG).await, Wake::Woken);
    }

    #[tokio::test]
    async fn test_woken_waits_without_a_timeout_and_keeps_an_earlier_wake() {
        let (tx, wake) = stub();
        let mut std_worker = wake.activity_wake().subscribe("std");

        // Nothing yet: still pending after a while
        assert!(
            tokio::time::timeout(SHORT, std_worker.woken())
                .await
                .is_err()
        );

        // A wake that arrives while nobody waits is kept for the next wait
        tx.send(Ok(activity_notification("std"))).unwrap();
        tokio::time::sleep(SHORT).await;
        tokio::time::timeout(SHORT, std_worker.woken())
            .await
            .expect("an earlier wake ends the wait at once");

        // ...and is consumed exactly once
        assert!(
            tokio::time::timeout(SHORT, std_worker.woken())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn test_activity_receiver_outlives_the_event_wake() {
        let (tx, wake) = stub();
        let activities = wake.activity_wake();
        let mut std_worker = activities.subscribe("std");
        drop(wake);
        drop(activities);

        tx.send(Ok(activity_notification("std"))).unwrap();
        assert_eq!(std_worker.wait(LONG).await, Wake::Woken);
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
