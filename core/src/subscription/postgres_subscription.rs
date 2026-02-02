//! PostgreSQL implementation of the subscription service.

use super::models::{ActivitySubscription, ExpiredSubscription, NewSubscription, SignalRequest};
use super::service::{Result, SubscriptionError, SubscriptionService};
use crate::workflow::OnTimeout;
use async_trait::async_trait;
use chrono::{Duration, Utc};
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

/// PostgreSQL-based subscription service
pub struct PostgresSubscriptionService {
    pool: PgPool,
}

impl PostgresSubscriptionService {
    /// Create a new PostgresSubscriptionService
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
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
                (workflow_id, activity_key, event_name, on_timeout, timeout_at)
            VALUES ($1, $2, $3, $4, $5)
            RETURNING id
            "#,
            subscription.workflow_id,
            subscription.activity_key,
            subscription.event_name,
            on_timeout_str,
            timeout_at
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
        // Update the subscription with signal data if event_name matches
        let row = sqlx::query!(
            r#"
            UPDATE activity_event_subscriptions
            SET signal_data = $4
            WHERE workflow_id = $1
              AND activity_key = $2
              AND event_name = $3
              AND signal_data IS NULL
            RETURNING id, workflow_id, activity_key, event_name, on_timeout, timeout_at, signal_data, created_at
            "#,
            request.workflow_id,
            request.activity_key,
            request.event_name,
            request.data
        )
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(r) => Ok(Some(ActivitySubscription {
                id: r.id,
                workflow_id: r.workflow_id,
                activity_key: r.activity_key,
                event_name: r.event_name,
                on_timeout: parse_on_timeout(&r.on_timeout),
                timeout_at: r.timeout_at,
                signal_data: r.signal_data,
                created_at: r.created_at,
            })),
            None => Ok(None),
        }
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
        let rows = sqlx::query!(
            r#"
            UPDATE activity_event_subscriptions
            SET expired_at = $1
            WHERE id IN (
                SELECT id FROM activity_event_subscriptions
                WHERE timeout_at <= $1
                  AND signal_data IS NULL
                  AND expired_at IS NULL
                ORDER BY timeout_at
                LIMIT $2
            )
            RETURNING id, workflow_id, activity_key, event_name, on_timeout
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
            SELECT id, workflow_id, activity_key, event_name, on_timeout
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
        };
        let cloned = expired.clone();
        assert_eq!(cloned.activity_key, expired.activity_key);
    }
}
