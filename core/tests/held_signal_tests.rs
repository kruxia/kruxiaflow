//! `wait_for_signal.hold_early` (#42): a signal that arrives while its
//! activity is not waiting is held, and the activity takes the oldest held
//! signal when it next waits, without waiting.
//!
//! Events are processed by `process_workflow_event` directly, so each test
//! decides exactly when a step completes and when a signal arrives. Signals
//! go through `deliver_signal`, the path the REST endpoint and the MCP tool
//! share.
mod common;

use kruxiaflow_core::events::{EventSource, NewWorkflowEvent, WorkflowEventType};
use kruxiaflow_core::orchestrator::{
    OrchestratorConfig,
    workflow_state::{WorkflowActivityStatus, WorkflowState, load_materialized_state},
};
use kruxiaflow_core::queue::{ActivityQueue, PostgresQueue, QueueConfig};
use kruxiaflow_core::subscription::{
    MAX_HELD_SIGNALS, SignalOutcome, SignalRefusal, SignalRequest, deliver_signal, take_held_signal,
};
use kruxiaflow_core::workflow::{
    WorkflowDefinition, WorkflowDefinitionRepository, WorkflowService,
};
use kruxiaflow_core::{PostgresSubscriptionService, SubscriptionService};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

/// `open`, then a loop of three rounds: `receive` waits for `message` and
/// holds early signals, `answer` follows it and leads back. `gate` waits for
/// `go` after `open`, without holding.
const LOOP: &str = r#"
name: held_signal_loop
activities:
  - key: open
    activity_name: http_request
  - key: receive
    activity_name: http_request
    iteration_limit: 3
    settings:
      wait_for_signal:
        event_name: message
        timeout_seconds: 3600
        hold_early: true
    depends_on:
      - open
      - activity_key: answer
  - key: answer
    activity_name: http_request
    depends_on:
      - receive
  - key: gate
    activity_name: http_request
    settings:
      wait_for_signal:
        event_name: go
        timeout_seconds: 3600
    depends_on:
      - open
"#;

struct Run {
    pool: PgPool,
    wf: Uuid,
    es: Arc<dyn EventSource>,
    q: Arc<dyn ActivityQueue>,
    subs: Arc<dyn SubscriptionService>,
    cfg: OrchestratorConfig,
}

impl Run {
    /// Store the definition, submit a workflow and process `WorkflowCreated`:
    /// `open` is queued, nothing waits yet
    async fn submit(pool: PgPool) -> Run {
        let def = WorkflowDefinition::from_yaml(LOOP).expect("definition parses");
        WorkflowDefinitionRepository::new(pool.clone())
            .store(def.clone())
            .await
            .expect("store definition");
        let created = WorkflowService::new(pool.clone())
            .submit_workflow(&def.name, None, json!({}), None, None)
            .await
            .expect("submit workflow");
        let run = Run {
            es: Arc::new(common::settled_event_source(pool.clone())),
            q: Arc::new(PostgresQueue::new(pool.clone(), QueueConfig::default())),
            subs: Arc::new(PostgresSubscriptionService::new(pool.clone())),
            cfg: OrchestratorConfig::new(pool.clone()),
            wf: created.id,
            pool,
        };
        run.process_all().await;
        run
    }

    /// Process every event published so far
    async fn process_all(&self) {
        loop {
            let events = self.es.poll("orchestrator").await.unwrap();
            if events.is_empty() {
                return;
            }
            for e in events {
                kruxiaflow_core::orchestrator::orchestrator::process_workflow_event(
                    &e, &self.es, &self.q, &self.subs, &self.cfg,
                )
                .await
                .expect("process event");
                self.es
                    .update_position("orchestrator", e.cursor())
                    .await
                    .unwrap();
            }
        }
    }

    async fn publish(&self, event_type: WorkflowEventType, key: Option<&str>, round: Option<i32>) {
        let payload = match event_type {
            WorkflowEventType::WorkflowFailed => json!({ "reason": "ended by the test" }),
            _ => json!({ "outputs": {} }),
        };
        self.es
            .publish(NewWorkflowEvent {
                workflow_id: self.wf,
                event_type,
                activity_key: key.map(String::from),
                payload,
                iteration: round,
            })
            .await
            .unwrap();
        self.process_all().await;
    }

    /// Complete `key` (in loop round `round`, None outside the loop) and
    /// process what follows
    async fn complete(&self, key: &str, round: Option<i32>) {
        self.publish(WorkflowEventType::ActivityCompleted, Some(key), round)
            .await;
    }

    fn request(&self, key: &str, event: &str, data: Option<Value>) -> SignalRequest {
        SignalRequest {
            workflow_id: self.wf,
            activity_key: key.to_string(),
            event_name: event.to_string(),
            data,
        }
    }

    /// Send a signal the way the API does, without processing its event
    async fn send(&self, key: &str, event: &str, data: Option<Value>) -> SignalOutcome {
        deliver_signal(
            self.subs.as_ref(),
            self.es.as_ref(),
            self.request(key, event, data),
        )
        .await
        .expect("deliver signal")
    }

    /// Send a `message` signal to `receive` carrying `{"text": text}`, then
    /// process whatever it published
    async fn message(&self, text: &str) -> SignalOutcome {
        let outcome = self
            .send("receive", "message", Some(json!({ "text": text })))
            .await;
        self.process_all().await;
        outcome
    }

    async fn state(&self) -> WorkflowState {
        let mut tx = self.pool.begin().await.unwrap();
        let s = load_materialized_state(&mut tx, self.wf).await.unwrap();
        tx.commit().await.unwrap();
        s
    }

    async fn status(&self, key: &str) -> WorkflowActivityStatus {
        self.state().await.activities[key].status
    }

    /// The signal data `receive` was queued with, per loop round, in order
    async fn queued_messages(&self) -> Vec<Option<Value>> {
        sqlx::query_scalar(
            "SELECT signal_data FROM activity_queue \
             WHERE workflow_id = $1 AND activity_key = 'receive' ORDER BY iteration",
        )
        .bind(self.wf)
        .fetch_all(&self.pool)
        .await
        .unwrap()
    }

    /// How many signals are held for the workflow
    async fn held(&self) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM held_signals WHERE workflow_id = $1")
            .bind(self.wf)
            .fetch_one(&self.pool)
            .await
            .unwrap()
    }

    /// The loop round of `key`'s open subscription, if it has one
    async fn subscription(&self, key: &str) -> Option<Option<i32>> {
        sqlx::query_scalar(
            "SELECT iteration FROM activity_event_subscriptions \
             WHERE workflow_id = $1 AND activity_key = $2",
        )
        .bind(self.wf)
        .bind(key)
        .fetch_optional(&self.pool)
        .await
        .unwrap()
    }

    /// `(iteration, payload)` of every `event_type` event for `key`, in order
    async fn events(&self, event_type: &str, key: &str) -> Vec<(Option<i32>, Value)> {
        sqlx::query_as(
            "SELECT iteration, payload FROM workflow_events \
             WHERE workflow_id = $1 AND event_type::text = $2 AND activity_key = $3 \
             ORDER BY position",
        )
        .bind(self.wf)
        .bind(event_type)
        .bind(key)
        .fetch_all(&self.pool)
        .await
        .unwrap()
    }
}

fn text(text: &str) -> Option<Value> {
    Some(json!({ "text": text }))
}

/// A signal sent before the step is reached is held, publishes nothing, and
/// is what the step runs with when it is reached: no wait, no subscription
#[sqlx::test(migrations = "../migrations")]
async fn test_early_signal_is_held_then_runs_the_step_without_waiting(pool: PgPool) {
    let run = Run::submit(pool).await;

    assert!(run.message("m1").await.is_held());
    assert_eq!(run.held().await, 1);
    assert!(run.events("ActivitySignaled", "receive").await.is_empty());
    assert_eq!(
        run.status("receive").await,
        WorkflowActivityStatus::NotScheduled
    );

    run.complete("open", None).await;

    let receive = &run.state().await.activities["receive"];
    assert_eq!(receive.status, WorkflowActivityStatus::Pending);
    assert_eq!(receive.signal_data, text("m1"));
    assert_eq!(run.queued_messages().await, vec![text("m1")]);
    assert_eq!(run.held().await, 0);
    assert_eq!(run.subscription("receive").await, None);
    assert!(run.events("ActivityWaiting", "receive").await.is_empty());

    // A wait without hold_early is unaffected: it opened as usual
    assert_eq!(run.status("gate").await, WorkflowActivityStatus::Waiting);
    assert_eq!(run.subscription("gate").await, Some(None));
}

/// Taking a held signal publishes its `ActivitySignaled` event, with the
/// loop round and `held: true`; reading that event back changes nothing
#[sqlx::test(migrations = "../migrations")]
async fn test_taking_a_held_signal_publishes_its_event(pool: PgPool) {
    let run = Run::submit(pool).await;
    run.message("m1").await;
    run.complete("open", None).await;

    assert_eq!(
        run.events("ActivitySignaled", "receive").await,
        vec![(
            Some(0),
            json!({ "event_name": "message", "signal_data": { "text": "m1" }, "held": true })
        )]
    );
    // `complete` processed that event too: the step is still queued, once
    assert_eq!(run.status("receive").await, WorkflowActivityStatus::Pending);
    assert_eq!(run.queued_messages().await, vec![text("m1")]);
    assert_eq!(run.events("ActivityScheduled", "receive").await.len(), 1);
}

/// Held signals are taken oldest first, one per round; with none left the
/// step waits, and the next signal is delivered to that wait directly
#[sqlx::test(migrations = "../migrations")]
async fn test_held_signals_are_delivered_oldest_first_then_the_step_waits(pool: PgPool) {
    let run = Run::submit(pool).await;
    assert!(run.message("m1").await.is_held());
    assert!(run.message("m2").await.is_held());

    run.complete("open", None).await;
    assert_eq!(run.queued_messages().await, vec![text("m1")]);

    run.complete("receive", Some(0)).await;
    run.complete("answer", Some(0)).await;
    assert_eq!(run.queued_messages().await, vec![text("m1"), text("m2")]);
    assert_eq!(run.held().await, 0);

    // Round 2: nothing held, so the step waits, on that round's subscription
    run.complete("receive", Some(1)).await;
    run.complete("answer", Some(1)).await;
    assert_eq!(run.status("receive").await, WorkflowActivityStatus::Waiting);
    assert_eq!(run.subscription("receive").await, Some(Some(2)));

    assert!(run.message("m3").await.is_signaled());
    let receive = &run.state().await.activities["receive"];
    assert_eq!(receive.status, WorkflowActivityStatus::Pending);
    assert_eq!(receive.signal_data, text("m3"));
    assert_eq!(
        run.queued_messages().await,
        vec![text("m1"), text("m2"), text("m3")]
    );
    assert_eq!(run.held().await, 0);

    // One signal event per round: two held, one delivered to the wait
    let held_flags: Vec<bool> = run
        .events("ActivitySignaled", "receive")
        .await
        .iter()
        .map(|(_, payload)| payload["held"] == json!(true))
        .collect();
    assert_eq!(held_flags, vec![true, true, false]);
}

/// A signal that arrives while the round is busy is held for the next round:
/// both while the step runs and in the moment after its wait took a signal
/// that the orchestrator has not processed yet
#[sqlx::test(migrations = "../migrations")]
async fn test_signal_while_the_round_is_busy_is_held_for_the_next(pool: PgPool) {
    let run = Run::submit(pool).await;
    run.complete("open", None).await;
    assert_eq!(run.status("receive").await, WorkflowActivityStatus::Waiting);

    // The wait takes m1; before the orchestrator processes it, m2 arrives
    assert!(
        run.send("receive", "message", text("m1"))
            .await
            .is_signaled()
    );
    assert!(run.send("receive", "message", text("m2")).await.is_held());
    run.process_all().await;
    assert_eq!(run.status("receive").await, WorkflowActivityStatus::Pending);

    // While the step runs, m3 arrives
    assert!(run.message("m3").await.is_held());
    assert_eq!(run.held().await, 2);

    run.complete("receive", Some(0)).await;
    run.complete("answer", Some(0)).await;
    run.complete("receive", Some(1)).await;
    run.complete("answer", Some(1)).await;

    assert_eq!(
        run.queued_messages().await,
        vec![text("m1"), text("m2"), text("m3")]
    );
    assert_eq!(run.held().await, 0);
}

/// A signal without data is held too, and runs the step with no signal
/// data, as a signal without data does on an open wait
#[sqlx::test(migrations = "../migrations")]
async fn test_held_signal_without_data_runs_the_step_with_none(pool: PgPool) {
    let run = Run::submit(pool).await;
    assert!(run.send("receive", "message", None).await.is_held());

    run.complete("open", None).await;

    let receive = &run.state().await.activities["receive"];
    assert_eq!(receive.status, WorkflowActivityStatus::Pending);
    assert_eq!(receive.signal_data, None);
    assert!(
        matches!(
            run.queued_messages().await.as_slice(),
            [None] | [Some(Value::Null)]
        ),
        "queued with data"
    );
    assert_eq!(run.held().await, 0);
    assert_eq!(
        run.events("ActivitySignaled", "receive").await,
        vec![(
            Some(0),
            json!({ "event_name": "message", "signal_data": null, "held": true })
        )]
    );
}

/// A signal that is neither delivered nor held says why
#[sqlx::test(migrations = "../migrations")]
async fn test_refused_signals_say_why(pool: PgPool) {
    let run = Run::submit(pool).await;
    let refusal = |outcome: SignalOutcome| match outcome {
        SignalOutcome::Refused(refusal) => refusal,
        other => panic!("expected a refusal, got {other:?}"),
    };

    // A wait without hold_early that is not open yet
    assert_eq!(
        refusal(run.send("gate", "go", None).await),
        SignalRefusal::NotWaiting
    );
    // No such activity, an activity that never waits, a different event name
    for (key, event) in [
        ("nosuch", "message"),
        ("open", "message"),
        ("receive", "go"),
    ] {
        assert_eq!(
            refusal(run.send(key, event, None).await),
            SignalRefusal::UnknownActivity,
            "{key} / {event}"
        );
    }
    // No such workflow
    let mut request = run.request("receive", "message", None);
    request.workflow_id = Uuid::now_v7();
    let outcome = deliver_signal(run.subs.as_ref(), run.es.as_ref(), request)
        .await
        .unwrap();
    assert_eq!(refusal(outcome), SignalRefusal::UnknownWorkflow);

    assert_eq!(run.held().await, 0);
    assert_eq!(SignalRefusal::NotWaiting.code(), "not_waiting");
    assert_eq!(SignalRefusal::UnknownWorkflow.code(), "unknown_workflow");
    assert_eq!(SignalRefusal::UnknownActivity.code(), "unknown_activity");
    assert_eq!(SignalRefusal::WorkflowEnded.code(), "workflow_ended");
    assert_eq!(SignalRefusal::HoldLimit.code(), "hold_limit");
}

/// Signals still held when the workflow ends are discarded, and a signal
/// sent afterwards is refused
#[sqlx::test(migrations = "../migrations")]
async fn test_held_signals_are_discarded_when_the_workflow_ends(pool: PgPool) {
    let run = Run::submit(pool).await;
    assert!(run.message("m1").await.is_held());
    assert!(run.message("m2").await.is_held());
    assert_eq!(run.held().await, 2);

    run.publish(WorkflowEventType::WorkflowFailed, None, None)
        .await;

    assert_eq!(run.held().await, 0);
    match run.message("m3").await {
        SignalOutcome::Refused(SignalRefusal::WorkflowEnded) => {}
        other => panic!("expected workflow_ended, got {other:?}"),
    }
    assert_eq!(run.held().await, 0);
}

/// An activity holds at most `MAX_HELD_SIGNALS` signals for an event; the
/// next one is refused, and taking one makes room again
#[sqlx::test(migrations = "../migrations")]
async fn test_hold_limit(pool: PgPool) {
    let run = Run::submit(pool).await;
    for n in 0..MAX_HELD_SIGNALS {
        let outcome = run
            .send("receive", "message", Some(json!({ "n": n })))
            .await;
        assert!(outcome.is_held(), "signal {n}: {outcome:?}");
    }
    match run.send("receive", "message", None).await {
        SignalOutcome::Refused(SignalRefusal::HoldLimit) => {}
        other => panic!("expected hold_limit, got {other:?}"),
    }
    assert_eq!(run.held().await, MAX_HELD_SIGNALS);

    // `receive` takes the oldest; there is room for one more
    run.complete("open", None).await;
    assert_eq!(run.queued_messages().await, vec![Some(json!({ "n": 0 }))]);
    assert!(run.send("receive", "message", None).await.is_held());
    assert_eq!(run.held().await, MAX_HELD_SIGNALS);
}

/// A held signal is consumed only if the transaction that took it commits:
/// a pass that fails after taking it leaves it held
#[sqlx::test(migrations = "../migrations")]
async fn test_a_held_signal_survives_a_rolled_back_take(pool: PgPool) {
    let run = Run::submit(pool).await;
    assert!(run.message("m1").await.is_held());

    let mut tx = run.pool.begin().await.unwrap();
    let taken = take_held_signal(&mut tx, run.wf, "receive", "message")
        .await
        .unwrap();
    assert_eq!(taken, text("m1"));
    tx.rollback().await.unwrap();
    assert_eq!(run.held().await, 1);

    let mut tx = run.pool.begin().await.unwrap();
    let taken = take_held_signal(&mut tx, run.wf, "receive", "message")
        .await
        .unwrap();
    assert_eq!(taken, text("m1"));
    // Held for another event name or activity: not this wait's
    assert_eq!(
        take_held_signal(&mut tx, run.wf, "receive", "go")
            .await
            .unwrap(),
        None
    );
    tx.commit().await.unwrap();
    assert_eq!(run.held().await, 0);

    let mut tx = run.pool.begin().await.unwrap();
    assert_eq!(
        take_held_signal(&mut tx, run.wf, "receive", "message")
            .await
            .unwrap(),
        None
    );
    tx.commit().await.unwrap();
}

/// `hold_signal` runs after `signal_activity` found no open wait. If the
/// wait opened in between, it takes the signal instead of holding it
#[sqlx::test(migrations = "../migrations")]
async fn test_hold_signal_signals_a_wait_that_opened_meanwhile(pool: PgPool) {
    let run = Run::submit(pool).await;
    run.complete("open", None).await;
    assert_eq!(run.status("receive").await, WorkflowActivityStatus::Waiting);

    let outcome = run
        .subs
        .hold_signal(run.request("receive", "message", text("m1")))
        .await
        .unwrap();

    match outcome {
        SignalOutcome::Signaled(subscription) => {
            assert_eq!(subscription.activity_key, "receive");
            assert_eq!(subscription.iteration, Some(0));
            assert_eq!(subscription.signal_data, text("m1"));
        }
        other => panic!("expected the open wait to take the signal, got {other:?}"),
    }
    assert_eq!(run.held().await, 0);
}

/// While a transaction that looked for a held signal is open (the
/// orchestrator's pass, about to open the wait), a signal for that activity
/// waits for it, and then finds the wait the pass opened
#[sqlx::test(migrations = "../migrations")]
async fn test_a_signal_waits_for_the_pass_that_is_opening_the_wait(pool: PgPool) {
    let run = Run::submit(pool).await;

    // The pass: nothing held, so it will open the wait before it commits
    let mut pass = run.pool.begin().await.unwrap();
    assert_eq!(
        take_held_signal(&mut pass, run.wf, "receive", "message")
            .await
            .unwrap(),
        None
    );

    // A signal sent now finds no wait and has to decide whether to hold
    let subs = run.subs.clone();
    let request = run.request("receive", "message", text("m1"));
    let signal = tokio::spawn(async move { subs.hold_signal(request).await.unwrap() });
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert!(
        !signal.is_finished(),
        "the signal decided while the pass was still opening the wait"
    );
    assert_eq!(run.held().await, 0);

    // The pass opens the wait and commits (here: `open` completes)
    pass.commit().await.unwrap();
    run.complete("open", None).await;

    // The signal then decides. Whichever of the two went first, it is not
    // lost: it is on the wait, or held where the wait's pass found it.
    let outcome = signal.await.unwrap();
    assert!(outcome.is_signaled() || outcome.is_held(), "{outcome:?}");
    run.process_all().await;
    let receive = &run.state().await.activities["receive"];
    if outcome.is_held() {
        assert_eq!(receive.status, WorkflowActivityStatus::Pending);
        assert_eq!(receive.signal_data, text("m1"));
    } else {
        assert_eq!(run.held().await, 0);
    }
}

/// `hold_early` is false unless set, and is left out of a stored definition
/// that does not set it
#[test]
fn test_hold_early_defaults_to_false() {
    let def = WorkflowDefinition::from_yaml(LOOP).unwrap();
    let wait = |key: &str| {
        def.activities
            .iter()
            .find(|a| a.key == key)
            .and_then(|a| a.settings.as_ref())
            .and_then(|s| s.wait_for_signal.clone())
            .unwrap()
    };
    assert!(wait("receive").hold_early);
    assert!(!wait("gate").hold_early);

    let stored = serde_json::to_value(wait("gate")).unwrap();
    assert!(stored.get("hold_early").is_none(), "{stored}");
    let stored = serde_json::to_value(wait("receive")).unwrap();
    assert_eq!(stored["hold_early"], json!(true));
}
