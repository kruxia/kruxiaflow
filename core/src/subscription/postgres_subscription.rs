//! PostgreSQL implementation of the subscription service.

use super::delivery::activity_signaled_event;
use super::models::{
    ActivitySubscription, ExpiredSubscription, MAX_HELD_SIGNALS, NewSubscription, SignalOutcome,
    SignalRefusal, SignalRequest,
};
use super::service::{Result, SubscriptionError, SubscriptionService};
use crate::events::{Announcer, insert_event};
use crate::workflow::{OnTimeout, WaitForSignalSettings};
use async_trait::async_trait;
use chrono::{Duration, Utc};
use serde_json::Value;
use sqlx::{PgConnection, PgExecutor, PgPool};
use uuid::Uuid;

/// PostgreSQL-based subscription service
pub struct PostgresSubscriptionService {
    pool: PgPool,
    /// Announces each signal event once its transaction commits
    announcer: Announcer,
}

impl PostgresSubscriptionService {
    /// Create a new PostgresSubscriptionService
    ///
    /// It announces nothing: the orchestrator finds its signal events on its
    /// poll timer. Production callers attach the process's announcer with
    /// [`with_announcer`](Self::with_announcer).
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            announcer: Announcer::none(),
        }
    }

    /// Wake the orchestrator when a signal's event commits
    pub fn with_announcer(mut self, announcer: Announcer) -> Self {
        self.announcer = announcer;
        self
    }
}

#[async_trait]
impl SubscriptionService for PostgresSubscriptionService {
    async fn create_subscription(&self, subscription: NewSubscription) -> Result<Uuid> {
        let timeout_at = Utc::now() + Duration::seconds(subscription.timeout_seconds as i64);
        let on_timeout_str = match subscription.on_timeout {
            OnTimeout::Continue => "continue",
            OnTimeout::Skip => "skip",
            OnTimeout::Fail => "fail",
        };

        let row = sqlx::query!(
            r#"
            INSERT INTO activity_event_subscriptions
                (workflow_id, activity_key, event_name, on_timeout, timeout_at, iteration)
            VALUES ($1, $2, $3, $4, $5, $6)
            RETURNING id
            "#,
            subscription.workflow_id,
            subscription.activity_key,
            subscription.event_name,
            on_timeout_str,
            timeout_at,
            subscription.iteration
        )
        .fetch_one(&self.pool)
        .await
        .map_err(|e| {
            if let sqlx::Error::Database(ref db_err) = e
                && db_err.is_unique_violation()
            {
                return SubscriptionError::AlreadyExists(
                    subscription.workflow_id,
                    subscription.activity_key.clone(),
                );
            }
            SubscriptionError::Database(e)
        })?;

        Ok(row.id)
    }

    async fn signal_activity(
        &self,
        request: SignalRequest,
    ) -> Result<Option<ActivitySubscription>> {
        let mut tx = self.pool.begin().await?;
        let subscription = mark_signaled(&mut *tx, &request).await?;
        let mut inserted = false;
        if let Some(subscription) = &subscription {
            inserted =
                insert_event(&mut *tx, &activity_signaled_event(subscription, &request)).await?;
        }
        tx.commit().await?;
        if inserted {
            self.announcer.events();
        }
        Ok(subscription)
    }

    async fn hold_signal(&self, request: SignalRequest) -> Result<SignalOutcome> {
        let mut tx = self.pool.begin().await?;
        lock_wait(&mut tx, request.workflow_id, &request.activity_key).await?;

        // The wait may have opened since the caller's signal_activity
        if let Some(subscription) = mark_signaled(&mut *tx, &request).await? {
            let inserted =
                insert_event(&mut *tx, &activity_signaled_event(&subscription, &request)).await?;
            tx.commit().await?;
            if inserted {
                self.announcer.events();
            }
            return Ok(SignalOutcome::Signaled(subscription));
        }

        // What the workflow's definition says about this activity's wait
        let row = sqlx::query!(
            r#"
            SELECT w.status::text AS "status!",
                   (SELECT a.value -> 'settings' -> 'wait_for_signal'
                    FROM jsonb_array_elements(d.activities) AS a
                    WHERE a.value ->> 'key' = $2
                    LIMIT 1) AS wait
            FROM workflows w
            JOIN workflow_definitions d ON d.id = w.workflow_definition_id
            WHERE w.id = $1
            "#,
            request.workflow_id,
            request.activity_key
        )
        .fetch_optional(&mut *tx)
        .await?;

        let Some(row) = row else {
            return Ok(SignalOutcome::Refused(SignalRefusal::UnknownWorkflow));
        };
        if matches!(row.status.as_str(), "completed" | "failed") {
            return Ok(SignalOutcome::Refused(SignalRefusal::WorkflowEnded));
        }
        let wait = row
            .wait
            .and_then(|settings| serde_json::from_value::<WaitForSignalSettings>(settings).ok())
            .filter(|wait| wait.event_name == request.event_name);
        let Some(wait) = wait else {
            return Ok(SignalOutcome::Refused(SignalRefusal::UnknownActivity));
        };
        if !wait.hold_early {
            return Ok(SignalOutcome::Refused(SignalRefusal::NotWaiting));
        }

        let held = sqlx::query_scalar!(
            r#"
            SELECT COUNT(*) AS "count!"
            FROM held_signals
            WHERE workflow_id = $1 AND activity_key = $2 AND event_name = $3
            "#,
            request.workflow_id,
            request.activity_key,
            request.event_name
        )
        .fetch_one(&mut *tx)
        .await?;
        if held >= MAX_HELD_SIGNALS {
            return Ok(SignalOutcome::Refused(SignalRefusal::HoldLimit));
        }

        sqlx::query!(
            r#"
            INSERT INTO held_signals (workflow_id, activity_key, event_name, signal_data)
            VALUES ($1, $2, $3, $4)
            "#,
            request.workflow_id,
            request.activity_key,
            request.event_name,
            request.data
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;

        Ok(SignalOutcome::Held)
    }

    async fn get_signal_data(
        &self,
        workflow_id: Uuid,
        activity_key: &str,
    ) -> Result<Option<Value>> {
        let row = sqlx::query!(
            r#"
            SELECT signal_data
            FROM activity_event_subscriptions
            WHERE workflow_id = $1 AND activity_key = $2
            "#,
            workflow_id,
            activity_key
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.and_then(|r| r.signal_data))
    }

    async fn expire_subscriptions(&self, limit: i64) -> Result<Vec<ExpiredSubscription>> {
        let now = Utc::now();

        // Mark expired subscriptions (past timeout, not yet signaled or expired).
        // We SET expired_at rather than DELETE so that if the server crashes before
        // the orchestrator publishes the corresponding events, recovery can find
        // these rows and re-process them.
        //
        // The outer WHERE repeats the open-wait test: if a signal commits after
        // the subquery picked a row, PostgreSQL re-checks the outer conditions
        // on the row's new version (but not the subquery's), so the signal wins
        // and the row is skipped instead of being both signaled and expired.
        let rows = sqlx::query!(
            r#"
            UPDATE activity_event_subscriptions
            SET expired_at = $1
            WHERE id IN (
                SELECT id FROM activity_event_subscriptions
                WHERE timeout_at <= $1
                  AND signaled_at IS NULL
                  AND expired_at IS NULL
                ORDER BY timeout_at
                LIMIT $2
            )
              AND signaled_at IS NULL
              AND expired_at IS NULL
            RETURNING id, workflow_id, activity_key, event_name, on_timeout, iteration
            "#,
            now,
            limit
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| ExpiredSubscription {
                id: r.id,
                workflow_id: r.workflow_id,
                activity_key: r.activity_key,
                event_name: r.event_name,
                on_timeout: parse_on_timeout(&r.on_timeout),
                iteration: r.iteration,
            })
            .collect())
    }

    async fn recover_expired(&self, limit: i64) -> Result<Vec<ExpiredSubscription>> {
        let cutoff = Utc::now() - Duration::seconds(1);

        // Find subscriptions that were marked expired at least 1 second ago but never
        // deleted. The grace period avoids racing with another orchestrator that is
        // still processing its own freshly-expired batch.
        let rows = sqlx::query!(
            r#"
            SELECT id, workflow_id, activity_key, event_name, on_timeout, iteration
            FROM activity_event_subscriptions
            WHERE expired_at IS NOT NULL
              AND expired_at <= $1
            ORDER BY expired_at
            LIMIT $2
            "#,
            cutoff,
            limit
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| ExpiredSubscription {
                id: r.id,
                workflow_id: r.workflow_id,
                activity_key: r.activity_key,
                event_name: r.event_name,
                on_timeout: parse_on_timeout(&r.on_timeout),
                iteration: r.iteration,
            })
            .collect())
    }

    async fn delete_subscription(&self, workflow_id: Uuid, activity_key: &str) -> Result<()> {
        sqlx::query!(
            r#"
            DELETE FROM activity_event_subscriptions
            WHERE workflow_id = $1 AND activity_key = $2
            "#,
            workflow_id,
            activity_key
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn delete_subscription_by_id(&self, id: Uuid) -> Result<()> {
        sqlx::query!("DELETE FROM activity_event_subscriptions WHERE id = $1", id)
            .execute(&self.pool)
            .await?;

        Ok(())
    }
}

/// Mark an activity's open wait signaled, if it has one for this event.
///
/// Takes the signal only while the wait is still open: not signaled
/// (`signaled_at`, not `signal_data`, which stays NULL for a signal without
/// data) and not expired (the timeout already ended the wait).
async fn mark_signaled<'e, E>(
    executor: E,
    request: &SignalRequest,
) -> Result<Option<ActivitySubscription>>
where
    E: PgExecutor<'e>,
{
    let row = sqlx::query!(
        r#"
        UPDATE activity_event_subscriptions
        SET signal_data = $4, signaled_at = NOW()
        WHERE workflow_id = $1
          AND activity_key = $2
          AND event_name = $3
          AND signaled_at IS NULL
          AND expired_at IS NULL
        RETURNING id, workflow_id, activity_key, event_name, on_timeout, timeout_at, signal_data, created_at, iteration
        "#,
        request.workflow_id,
        request.activity_key,
        request.event_name,
        request.data
    )
    .fetch_optional(executor)
    .await?;

    Ok(row.map(|r| ActivitySubscription {
        id: r.id,
        workflow_id: r.workflow_id,
        activity_key: r.activity_key,
        event_name: r.event_name,
        on_timeout: parse_on_timeout(&r.on_timeout),
        timeout_at: r.timeout_at,
        signal_data: r.signal_data,
        created_at: r.created_at,
        iteration: r.iteration,
    }))
}

/// Serialize, until the transaction ends, everything that decides between
/// "this activity's wait takes the signal" and "the signal is held for its
/// next wait": the signal API holding a signal ([`SubscriptionService::hold_signal`])
/// and the orchestrator opening the wait ([`take_held_signal`]). Without it a
/// signal could be held just after the wait that should have taken it opened,
/// and stay held while the activity waits.
async fn lock_wait(
    conn: &mut PgConnection,
    workflow_id: Uuid,
    activity_key: &str,
) -> std::result::Result<(), sqlx::Error> {
    // Not a query! macro: the function returns void
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::uuid::text || ':' || $2, 0))")
        .bind(workflow_id)
        .bind(activity_key)
        .execute(conn)
        .await?;
    Ok(())
}

/// Take the oldest signal held for an activity that is about to wait
/// (`wait_for_signal.hold_early`), if there is one (#42).
///
/// Returns the signal's data (JSON null when it had none). The caller runs
/// the activity with it instead of opening a wait.
///
/// Call it on the transaction that saves the workflow's state: the held
/// signal is then consumed only if that state is saved, and the advisory lock
/// taken here is held until then, so a signal sent meanwhile waits and finds
/// either the wait this pass opens or nothing to race with.
pub async fn take_held_signal(
    conn: &mut PgConnection,
    workflow_id: Uuid,
    activity_key: &str,
    event_name: &str,
) -> std::result::Result<Option<Value>, sqlx::Error> {
    lock_wait(conn, workflow_id, activity_key).await?;

    let row = sqlx::query!(
        r#"
        DELETE FROM held_signals
        WHERE id = (
            SELECT id
            FROM held_signals
            WHERE workflow_id = $1 AND activity_key = $2 AND event_name = $3
            ORDER BY created_at, id
            LIMIT 1
        )
        RETURNING signal_data
        "#,
        workflow_id,
        activity_key,
        event_name
    )
    .fetch_optional(conn)
    .await?;

    Ok(row.map(|r| r.signal_data.unwrap_or(Value::Null)))
}

/// Discard every signal still held for a workflow that has ended.
/// Returns how many were discarded.
pub async fn discard_held_signals(
    conn: &mut PgConnection,
    workflow_id: Uuid,
) -> std::result::Result<u64, sqlx::Error> {
    let result = sqlx::query!(
        "DELETE FROM held_signals WHERE workflow_id = $1",
        workflow_id
    )
    .execute(conn)
    .await?;
    Ok(result.rows_affected())
}

fn parse_on_timeout(s: &str) -> OnTimeout {
    match s {
        "continue" => OnTimeout::Continue,
        "skip" => OnTimeout::Skip,
        "fail" => OnTimeout::Fail,
        _ => OnTimeout::Fail, // Default to fail for unknown values
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_parse_on_timeout_continue() {
        assert!(matches!(parse_on_timeout("continue"), OnTimeout::Continue));
    }

    #[test]
    fn test_parse_on_timeout_skip() {
        assert!(matches!(parse_on_timeout("skip"), OnTimeout::Skip));
    }

    #[test]
    fn test_parse_on_timeout_fail() {
        assert!(matches!(parse_on_timeout("fail"), OnTimeout::Fail));
    }

    #[test]
    fn test_parse_on_timeout_unknown_defaults_to_fail() {
        assert!(matches!(parse_on_timeout("unknown"), OnTimeout::Fail));
    }

    #[test]
    fn test_parse_on_timeout_empty_string_defaults_to_fail() {
        assert!(matches!(parse_on_timeout(""), OnTimeout::Fail));
    }

    #[test]
    fn test_parse_on_timeout_case_sensitive() {
        // Uppercase should not match — defaults to Fail
        assert!(matches!(parse_on_timeout("Continue"), OnTimeout::Fail));
        assert!(matches!(parse_on_timeout("SKIP"), OnTimeout::Fail));
        assert!(matches!(parse_on_timeout("FAIL"), OnTimeout::Fail));
    }

    #[test]
    fn test_new_subscription_fields() {
        let sub = NewSubscription {
            workflow_id: Uuid::nil(),
            activity_key: "step_1".to_string(),
            event_name: "approval".to_string(),
            on_timeout: OnTimeout::Continue,
            timeout_seconds: 300,
            iteration: None,
        };
        assert_eq!(sub.activity_key, "step_1");
        assert_eq!(sub.event_name, "approval");
        assert_eq!(sub.timeout_seconds, 300);
    }

    #[test]
    fn test_signal_request_serialization() {
        let request = SignalRequest {
            workflow_id: Uuid::nil(),
            activity_key: "step_1".to_string(),
            event_name: "approval".to_string(),
            data: Some(json!({"approved": true})),
        };

        let json_str = serde_json::to_string(&request).unwrap();
        let deserialized: SignalRequest = serde_json::from_str(&json_str).unwrap();

        assert_eq!(deserialized.activity_key, "step_1");
        assert_eq!(deserialized.event_name, "approval");
        assert_eq!(deserialized.data, Some(json!({"approved": true})));
    }

    #[test]
    fn test_signal_request_serialization_no_data() {
        let request = SignalRequest {
            workflow_id: Uuid::nil(),
            activity_key: "step_1".to_string(),
            event_name: "approval".to_string(),
            data: None,
        };

        let json_str = serde_json::to_string(&request).unwrap();
        let deserialized: SignalRequest = serde_json::from_str(&json_str).unwrap();

        assert!(deserialized.data.is_none());
    }

    #[test]
    fn test_expired_subscription_fields() {
        let expired = ExpiredSubscription {
            id: Uuid::nil(),
            workflow_id: Uuid::nil(),
            activity_key: "step_2".to_string(),
            event_name: "timeout_event".to_string(),
            on_timeout: OnTimeout::Skip,
            iteration: None,
        };
        assert_eq!(expired.activity_key, "step_2");
        assert!(matches!(expired.on_timeout, OnTimeout::Skip));
    }

    #[test]
    fn test_activity_subscription_serialization() {
        let sub = ActivitySubscription {
            id: Uuid::nil(),
            workflow_id: Uuid::nil(),
            activity_key: "step_1".to_string(),
            event_name: "signal".to_string(),
            on_timeout: OnTimeout::Fail,
            timeout_at: Utc::now(),
            signal_data: Some(json!({"key": "value"})),
            created_at: Utc::now(),
            iteration: None,
        };

        let json_str = serde_json::to_string(&sub).unwrap();
        let deserialized: ActivitySubscription = serde_json::from_str(&json_str).unwrap();

        assert_eq!(deserialized.activity_key, "step_1");
        assert_eq!(deserialized.signal_data, Some(json!({"key": "value"})));
    }

    #[test]
    fn test_activity_subscription_without_signal_data() {
        let sub = ActivitySubscription {
            id: Uuid::nil(),
            workflow_id: Uuid::nil(),
            activity_key: "step_1".to_string(),
            event_name: "signal".to_string(),
            on_timeout: OnTimeout::Continue,
            timeout_at: Utc::now(),
            signal_data: None,
            created_at: Utc::now(),
            iteration: None,
        };

        let json_str = serde_json::to_string(&sub).unwrap();
        let deserialized: ActivitySubscription = serde_json::from_str(&json_str).unwrap();

        assert!(deserialized.signal_data.is_none());
        assert!(matches!(deserialized.on_timeout, OnTimeout::Continue));
    }

    #[test]
    fn test_subscription_error_display_not_found() {
        let err = SubscriptionError::NotFound;
        assert_eq!(format!("{}", err), "Subscription not found");
    }

    #[test]
    fn test_subscription_error_display_already_exists() {
        let wf_id = Uuid::nil();
        let err = SubscriptionError::AlreadyExists(wf_id, "step_1".to_string());
        let msg = format!("{}", err);
        assert!(msg.contains("already exists"));
        assert!(msg.contains("step_1"));
    }

    #[test]
    fn test_subscription_error_display_event_name_mismatch() {
        let err = SubscriptionError::EventNameMismatch {
            expected: "approval".to_string(),
            actual: "rejection".to_string(),
        };
        let msg = format!("{}", err);
        assert!(msg.contains("approval"));
        assert!(msg.contains("rejection"));
    }

    #[test]
    fn test_expired_subscription_clone() {
        let expired = ExpiredSubscription {
            id: Uuid::nil(),
            workflow_id: Uuid::nil(),
            activity_key: "step_1".to_string(),
            event_name: "event".to_string(),
            on_timeout: OnTimeout::Fail,
            iteration: None,
        };
        let cloned = expired.clone();
        assert_eq!(cloned.activity_key, expired.activity_key);
    }
}
