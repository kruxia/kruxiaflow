//! Poll / lease / execute loop with semaphore-based concurrency.

use crate::client::{WakeConnect, WorkerApiClient};
use crate::config::{WakeMode, WorkerConfig};
use crate::context::ActivityContext;
use crate::error::{ActivityError, ClientError};
use crate::registry::ActivityExecutor;
use crate::types::PendingActivity;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, watch};
use tokio::task::AbortHandle;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// Sleep before retrying after a poll error.
const ERROR_RETRY_INTERVAL: Duration = Duration::from_secs(5);

/// Wake stream reconnect backoff after a failure.
const WAKE_RETRY_MIN: Duration = Duration::from_secs(1);
const WAKE_RETRY_MAX: Duration = Duration::from_secs(30);

/// How often to look again for a wake stream on a server that has none.
const WAKE_UNAVAILABLE_RETRY: Duration = Duration::from_secs(60);

/// What the wake stream task tells the poll loop.
///
/// `counter` changes on every wake, and whenever the stream drops (wakes may
/// have been missed). `connected` says whether the stream is up right now,
/// which decides how long the loop may wait between polls.
struct WakeSignal {
    counter: watch::Sender<u64>,
    connected: AtomicBool,
}

impl WakeSignal {
    fn new() -> Self {
        Self {
            counter: watch::channel(0u64).0,
            connected: AtomicBool::new(false),
        }
    }

    fn wake(&self) {
        self.counter.send_modify(|n| *n = n.wrapping_add(1));
    }
}

/// Stops a background task when dropped.
///
/// The wake stream task must not outlive the poll loop. The loop can end by
/// returning, or by its future being dropped (a caller aborting the task that
/// runs it, as `WorkerManager::stop` does); a bare `JoinHandle` would detach
/// in the second case and leave the stream open.
struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Activities with a timeout at or below this run without a heartbeat task.
const HEARTBEAT_THRESHOLD: Duration = Duration::from_secs(60);

/// Worker poll loop.
///
/// Maintains up to `max_concurrent_activities` in-flight activities via a
/// semaphore, polling for more work whenever slots free up. When a poll
/// finds nothing, the loop waits for the server's wake stream, for the next
/// delayed activity to come due, or for its timer, whichever is first (see
/// [`WorkerConfig`]); the wake only shortens the wait, and the loop is
/// correct without it. Each claimed
/// activity runs in its own task; panics are caught and reported as retryable
/// failures, and completion is always reported before its heartbeat stops
/// (so the activity cannot be reclaimed as stale mid-report).
///
/// On shutdown (see [`WorkerPoller::shutdown_token`]) polling stops and
/// in-flight activities drain for up to `shutdown_timeout`; whatever remains
/// is aborted and failed as retryable so it re-queues.
pub struct WorkerPoller {
    config: WorkerConfig,
    client: WorkerApiClient,
    executor: Arc<dyn ActivityExecutor>,
    semaphore: Arc<Semaphore>,
    shutdown: CancellationToken,
    in_flight: Arc<Mutex<HashMap<Uuid, AbortHandle>>>,
}

impl WorkerPoller {
    /// Create a poller. `config.worker` must be the worker type to poll for.
    pub fn new(
        config: WorkerConfig,
        client: WorkerApiClient,
        executor: Arc<dyn ActivityExecutor>,
    ) -> Self {
        let semaphore = Arc::new(Semaphore::new(config.max_concurrent_activities));
        Self {
            config,
            client,
            executor,
            semaphore,
            shutdown: CancellationToken::new(),
            in_flight: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Token that stops the loop and starts the graceful drain when
    /// cancelled.
    pub fn shutdown_token(&self) -> CancellationToken {
        self.shutdown.clone()
    }

    /// Run until the shutdown token is cancelled, then drain.
    pub async fn run(&self) {
        tracing::info!(
            worker_id = %self.config.worker_id,
            worker = %self.config.worker,
            max_concurrent_activities = self.config.max_concurrent_activities,
            "Starting worker poller"
        );

        let signal = Arc::new(WakeSignal::new());
        let mut wakes = signal.counter.subscribe();
        let wake_task = (self.config.wake == WakeMode::Sse).then(|| {
            AbortOnDrop(tokio::spawn(run_wake_stream(
                self.client.clone(),
                self.config.worker.clone(),
                Arc::clone(&signal),
                self.shutdown.clone(),
            )))
        });

        loop {
            // Wait for a free slot (or shutdown). Holding a permit before
            // polling guarantees we never claim work we cannot start.
            let permit = tokio::select! {
                _ = self.shutdown.cancelled() => break,
                permit = Arc::clone(&self.semaphore).acquire_owned() => match permit {
                    Ok(permit) => permit,
                    Err(_) => break,
                },
            };

            // The poll + spawn phase is not cancelled by shutdown: dropping
            // it after the server assigned activities would strand them until
            // their timeout. Worst-case shutdown latency is one poll call.
            //
            // Mark the wakes seen so far before polling: one that arrives
            // while the poll is in flight then ends the next wait at once,
            // instead of being lost.
            wakes.borrow_and_update();
            match self.poll_and_spawn(permit).await {
                Ok((0, next_due)) => self.idle_wait(&mut wakes, &signal, next_due).await,
                Ok(_) => {} // work found: poll again immediately
                Err(err) => {
                    tracing::error!(error = %err, "Poll failed");
                    self.sleep_or_shutdown(ERROR_RETRY_INTERVAL).await;
                }
            }
        }

        // Close the stream before draining: nothing more will be claimed
        drop(wake_task);

        self.drain().await;
    }

    /// Wait after an empty poll: until a wake, until the next delayed
    /// activity is due, until the timer, or until shutdown.
    ///
    /// The timer is `wake_poll_interval` only while the wake stream is
    /// connected. Without it nothing would announce new work, so the loop
    /// polls every `poll_interval` as it always has.
    async fn idle_wait(
        &self,
        wakes: &mut watch::Receiver<u64>,
        signal: &WakeSignal,
        next_due: Option<Duration>,
    ) {
        let timer = if signal.connected.load(Ordering::Relaxed) {
            self.config.wake_poll_interval
        } else {
            self.config.poll_interval
        };
        let wait = next_due.map_or(timer, |due| due.min(timer));

        tokio::select! {
            _ = self.shutdown.cancelled() => {}
            _ = wakes.changed() => {}
            _ = tokio::time::sleep(wait) => {}
        }
    }

    /// Poll once, run whatever was claimed to completion, and return the
    /// number of activities executed (0 = no work was available).
    ///
    /// Activities still running after the configured `shutdown_timeout` are
    /// aborted and failed as retryable, exactly like a graceful drain.
    pub async fn run_once(&self) -> usize {
        let permit = match Arc::clone(&self.semaphore).acquire_owned().await {
            Ok(permit) => permit,
            Err(_) => return 0,
        };

        let count = match self.poll_and_spawn(permit).await {
            Ok((count, _)) => count,
            Err(err) => {
                tracing::error!(error = %err, "Poll failed");
                0
            }
        };

        self.drain().await;
        count
    }

    async fn sleep_or_shutdown(&self, duration: Duration) {
        tokio::select! {
            _ = self.shutdown.cancelled() => {}
            _ = tokio::time::sleep(duration) => {}
        }
    }

    /// Poll for up to the available slots and spawn a task per claimed
    /// activity. Returns the number of activities claimed and, when that is
    /// zero, how long until a delayed activity comes due (if the server knows
    /// of one).
    async fn poll_and_spawn(
        &self,
        permit: OwnedSemaphorePermit,
    ) -> Result<(usize, Option<Duration>), ClientError> {
        // +1 for the permit we already hold
        let available_slots = self.semaphore.available_permits() + 1;
        let max_to_poll = available_slots.min(self.config.poll_max_activities);

        let outcome = self
            .client
            .poll(&self.config.worker, &self.config.worker_id, max_to_poll)
            .await?;
        let activities = outcome.activities;

        if activities.is_empty() {
            drop(permit);
            return Ok((0, outcome.next_due));
        }

        let count = activities.len();
        tracing::debug!(
            worker_id = %self.config.worker_id,
            count,
            "Claimed activities"
        );

        let mut permits: Vec<OwnedSemaphorePermit> = vec![permit];
        for _ in 1..count {
            // Cannot block: only this loop acquires permits, and we polled at
            // most the number available.
            let permit = Arc::clone(&self.semaphore)
                .acquire_owned()
                .await
                .expect("semaphore is never closed");
            permits.push(permit);
        }

        for (activity, permit) in activities.into_iter().zip(permits) {
            self.spawn_activity(activity, permit);
        }

        Ok((count, None))
    }

    fn spawn_activity(&self, activity: PendingActivity, permit: OwnedSemaphorePermit) {
        let runner = ActivityRunner {
            config: self.config.clone(),
            client: self.client.clone(),
            executor: Arc::clone(&self.executor),
            in_flight: Arc::clone(&self.in_flight),
        };

        let activity_id = activity.activity_id;
        let handle = tokio::spawn(async move {
            runner.run(activity).await;
            drop(permit); // release the slot only after in_flight removal
        });

        self.in_flight
            .lock()
            .expect("in_flight lock poisoned")
            .insert(activity_id, handle.abort_handle());
    }

    /// Wait for in-flight activities to finish, up to `shutdown_timeout`;
    /// abort and fail (retryable) whatever remains so it re-queues.
    async fn drain(&self) {
        let slots = u32::try_from(self.config.max_concurrent_activities).unwrap_or(u32::MAX);
        let in_flight_count = self
            .in_flight
            .lock()
            .expect("in_flight lock poisoned")
            .len();
        tracing::info!(
            in_flight = in_flight_count,
            timeout_secs = self.config.shutdown_timeout.as_secs_f64(),
            "Shutting down; draining in-flight activities"
        );

        // Owning every permit proves every activity task has finished.
        let drained = tokio::time::timeout(
            self.config.shutdown_timeout,
            self.semaphore.acquire_many(slots),
        )
        .await;

        match drained {
            Ok(_) => tracing::info!("All in-flight activities drained"),
            Err(_) => {
                let remaining: Vec<(Uuid, AbortHandle)> = self
                    .in_flight
                    .lock()
                    .expect("in_flight lock poisoned")
                    .drain()
                    .collect();
                tracing::warn!(
                    count = remaining.len(),
                    "Drain deadline exceeded; failing remaining activities as retryable"
                );
                for (activity_id, abort) in remaining {
                    abort.abort();
                    let error = ActivityError::retryable(
                        "WORKER_SHUTDOWN",
                        "Worker shut down before the activity completed; it will re-queue",
                    );
                    match self
                        .client
                        .fail_activity(activity_id, &self.config.worker_id, &error)
                        .await
                    {
                        Ok(_) => {}
                        Err(ClientError::Conflict { body }) => {
                            tracing::info!(%activity_id, body, "Activity already settled elsewhere")
                        }
                        Err(err) => {
                            tracing::error!(%activity_id, error = %err, "Failed to re-queue activity during shutdown")
                        }
                    }
                }
            }
        }
    }
}

/// Everything one spawned activity task needs.
struct ActivityRunner {
    config: WorkerConfig,
    client: WorkerApiClient,
    executor: Arc<dyn ActivityExecutor>,
    in_flight: Arc<Mutex<HashMap<Uuid, AbortHandle>>>,
}

impl ActivityRunner {
    async fn run(self, activity: PendingActivity) {
        let span = tracing::info_span!(
            "execute_activity",
            worker_id = %self.config.worker_id,
            activity_id = %activity.activity_id,
            workflow_id = %activity.workflow_id,
            activity_key = %activity.activity_key,
            worker = %activity.worker,
            activity_name = %activity.activity_name,
        );
        let activity_id = activity.activity_id;
        tracing::Instrument::instrument(self.execute(activity), span).await;
        self.in_flight
            .lock()
            .expect("in_flight lock poisoned")
            .remove(&activity_id);
    }

    async fn execute(&self, activity: PendingActivity) {
        tracing::debug!("Executing activity");

        let timeout = activity
            .timeout_seconds
            .map(|seconds| Duration::from_secs(seconds.max(0) as u64))
            .unwrap_or(self.config.activity_timeout);

        // Cancelled when a heartbeat gets a 409: the activity was completed
        // or reassigned elsewhere, so local execution must stop.
        let reassigned = CancellationToken::new();

        let heartbeat_handle = (timeout > HEARTBEAT_THRESHOLD).then(|| {
            tokio::spawn(heartbeat_loop(
                self.client.clone(),
                activity.activity_id,
                self.config.worker_id.clone(),
                self.config.heartbeat_interval,
                reassigned.clone(),
            ))
        });

        let ctx = ActivityContext::new(
            activity.workflow_id,
            activity.activity_id,
            activity.activity_key.clone(),
        )
        .with_signal(activity.signal_data.clone())
        .with_client(self.client.clone(), self.config.worker_id.clone());

        // Run the handler in its own task so a panic is caught (reported as
        // a retryable failure) instead of unwinding through the worker.
        let mut handler = tokio::spawn({
            let executor = Arc::clone(&self.executor);
            let activity = activity.clone();
            let ctx = ctx.clone();
            async move { executor.execute(&activity, &ctx, timeout).await }
        });

        let outcome: Option<Result<crate::ActivityResult, ActivityError>> = tokio::select! {
            result = tokio::time::timeout(timeout, &mut handler) => Some(match result {
                Ok(Ok(handler_result)) => handler_result,
                Ok(Err(join_error)) => Err(ActivityError::retryable(
                    "PANIC",
                    format!("Activity handler panicked: {join_error}"),
                )),
                Err(_elapsed) => {
                    handler.abort();
                    Err(ActivityError::retryable(
                        "TIMEOUT",
                        format!("Activity execution timed out after {:.0?}", timeout),
                    ))
                }
            }),
            _ = reassigned.cancelled() => {
                handler.abort();
                tracing::warn!("Activity reassigned or completed elsewhere; canceling local execution");
                None
            }
        };

        // Report BEFORE canceling the heartbeat: if heartbeats stopped first,
        // a delayed report could let another worker reclaim the activity as
        // stale in between.
        if let Some(result) = outcome {
            self.report(activity.activity_id, result).await;
        }

        if let Some(handle) = heartbeat_handle {
            handle.abort();
        }
    }

    async fn report(
        &self,
        activity_id: Uuid,
        result: Result<crate::ActivityResult, ActivityError>,
    ) {
        match result {
            Ok(activity_result) => {
                let output = activity_result.to_json_value();
                match self
                    .client
                    .complete_activity(
                        activity_id,
                        &self.config.worker_id,
                        output,
                        activity_result.cost_usd,
                        &activity_result.usage,
                    )
                    .await
                {
                    Ok(ack) => {
                        for warning in &ack.warnings {
                            tracing::warn!(warning, "Usage warning from server");
                        }
                        tracing::debug!("Activity completed");
                    }
                    Err(ClientError::Conflict { body }) => {
                        tracing::warn!(body, "Completion conflict (already settled); ignoring")
                    }
                    Err(err) => {
                        tracing::error!(error = %err, "Failed to report activity completion")
                    }
                }
            }
            Err(error) => {
                tracing::warn!(code = %error.code, error = %error.message, retryable = error.retryable, "Activity failed");
                match self
                    .client
                    .fail_activity(activity_id, &self.config.worker_id, &error)
                    .await
                {
                    Ok(ack) => {
                        for warning in &ack.warnings {
                            tracing::warn!(warning, "Usage warning from server");
                        }
                    }
                    Err(ClientError::Conflict { body }) => {
                        tracing::warn!(body, "Failure conflict (already settled); ignoring")
                    }
                    Err(err) => {
                        tracing::error!(error = %err, "Failed to report activity failure")
                    }
                }
            }
        }
    }
}

/// Periodic heartbeats until aborted; a 409 cancels local execution via the
/// `reassigned` token.
async fn heartbeat_loop(
    client: WorkerApiClient,
    activity_id: Uuid,
    worker_id: String,
    interval: Duration,
    reassigned: CancellationToken,
) {
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        ticker.tick().await;
        match client.heartbeat(activity_id, &worker_id).await {
            Ok(()) => {}
            Err(ClientError::Conflict { body }) => {
                tracing::warn!(%activity_id, body, "Heartbeat conflict; activity no longer ours");
                reassigned.cancel();
                break;
            }
            Err(err) => {
                tracing::warn!(%activity_id, error = %err, "Failed to send heartbeat");
            }
        }
    }
}

/// Keep the server's wake stream open for `worker` and pass each wake to the
/// poll loop.
///
/// A stream that drops counts as a wake (wakes may have been missed) and is
/// reopened with backoff. A server without a wake stream is asked again once
/// a minute; until then the poll loop runs on its ordinary timer.
async fn run_wake_stream(
    client: WorkerApiClient,
    worker: String,
    signal: Arc<WakeSignal>,
    shutdown: CancellationToken,
) {
    let mut retry = WAKE_RETRY_MIN;
    let mut reported_unavailable = false;

    loop {
        let opened = tokio::select! {
            _ = shutdown.cancelled() => return,
            opened = client.open_wake_stream(&worker) => opened,
        };

        let delay = match opened {
            Ok(WakeConnect::Stream(mut stream)) => {
                tracing::info!(worker = %worker, "Wake stream connected");
                signal.connected.store(true, Ordering::Relaxed);
                retry = WAKE_RETRY_MIN;
                reported_unavailable = false;

                let reason = loop {
                    tokio::select! {
                        _ = shutdown.cancelled() => {
                            signal.connected.store(false, Ordering::Relaxed);
                            return;
                        }
                        wake = stream.next_wake() => match wake {
                            Ok(()) => signal.wake(),
                            Err(err) => break err,
                        },
                    }
                };

                signal.connected.store(false, Ordering::Relaxed);
                signal.wake();
                tracing::warn!(
                    worker = %worker,
                    reason = %reason,
                    "Wake stream lost; polling on the timer until it reconnects"
                );
                WAKE_RETRY_MIN
            }
            Ok(WakeConnect::Unavailable { status }) => {
                if !reported_unavailable {
                    tracing::info!(
                        worker = %worker,
                        status,
                        "Server offers no wake stream; polling on the timer"
                    );
                    reported_unavailable = true;
                }
                WAKE_UNAVAILABLE_RETRY
            }
            Err(err) => {
                tracing::debug!(worker = %worker, error = %err, "Wake stream connect failed");
                let delay = retry;
                retry = (retry * 2).min(WAKE_RETRY_MAX);
                delay
            }
        };

        tokio::select! {
            _ = shutdown.cancelled() => return,
            _ = tokio::time::sleep(delay) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::ActivityRegistry;

    fn test_config() -> WorkerConfig {
        WorkerConfig {
            worker: "test".to_string(),
            worker_id: "test_worker".to_string(),
            ..WorkerConfig::default()
        }
    }

    #[test]
    fn semaphore_matches_max_concurrency() {
        let config = WorkerConfig {
            max_concurrent_activities: 32,
            ..test_config()
        };
        let poller = WorkerPoller::new(
            config,
            WorkerApiClient::new("http://localhost:8080"),
            Arc::new(ActivityRegistry::new()),
        );
        assert_eq!(poller.semaphore.available_permits(), 32);
    }

    #[test]
    fn timeout_determination() {
        let activity: PendingActivity = serde_json::from_value(serde_json::json!({
            "activity_id": Uuid::now_v7(),
            "workflow_id": Uuid::now_v7(),
            "activity_key": "k",
            "worker": "test",
            "activity_name": "a",
            "parameters": {},
            "timeout_seconds": 10,
        }))
        .unwrap();
        let config = test_config();
        let timeout = activity
            .timeout_seconds
            .map(|s| Duration::from_secs(s.max(0) as u64))
            .unwrap_or(config.activity_timeout);
        assert_eq!(timeout, Duration::from_secs(10));
    }

    #[tokio::test]
    async fn shutdown_before_work_drains_immediately() {
        let poller = WorkerPoller::new(
            test_config(),
            WorkerApiClient::new("http://localhost:1"), // unreachable; never polled
            Arc::new(ActivityRegistry::new()),
        );
        poller.shutdown_token().cancel();
        // Completes without hanging: loop exits before polling, drain has
        // nothing in flight.
        tokio::time::timeout(Duration::from_secs(5), poller.run())
            .await
            .expect("run() should return promptly after shutdown");
    }
}
