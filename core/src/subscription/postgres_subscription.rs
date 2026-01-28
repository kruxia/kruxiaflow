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
            if let sqlx::Error::Database(ref db_err) = e {
                if db_err.is_unique_violation() {
                    return SubscriptionError::AlreadyExists(
                        subscription.workflow_id,
                        subscription.activity_key.clone(),
                    );
                }
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
