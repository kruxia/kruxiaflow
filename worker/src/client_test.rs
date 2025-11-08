#[cfg(test)]
mod tests {
    use crate::client::*;
    use serde_json::json;
    use uuid::Uuid;

    #[tokio::test]
    async fn test_poll_activities_response_parsing() {
        // Test that we can properly parse a PollActivitiesResponse
        let activity_id = Uuid::now_v7();
        let workflow_id = Uuid::now_v7();

        let json_response = json!({
            "activities": [
                {
                    "activity_id": activity_id,
                    "workflow_id": workflow_id,
                    "activity_key": "test_activity",
                    "namespace": "default",
                    "name": "echo",
                    "parameters": {"test": "value"},
                    "settings": null,
                    "timeout_seconds": 300
                }
            ],
            "count": 1
        });

        let response: PollActivitiesResponse =
            serde_json::from_value(json_response).expect("Should parse response");

        assert_eq!(response.count, 1);
        assert_eq!(response.activities.len(), 1);
        assert_eq!(response.activities[0].activity_id, activity_id);
        assert_eq!(response.activities[0].workflow_id, workflow_id);
        assert_eq!(response.activities[0].namespace, "default");
        assert_eq!(response.activities[0].name, "echo");
    }

    #[tokio::test]
    async fn test_pending_activity_parsing() {
        let activity_id = Uuid::now_v7();
        let workflow_id = Uuid::now_v7();

        let json_activity = json!({
            "activity_id": activity_id,
            "workflow_id": workflow_id,
            "activity_key": "test_activity",
            "namespace": "payments",
            "name": "authorize",
            "parameters": {"amount": 100.50},
            "settings": {"retry_limit": 3},
            "timeout_seconds": 600
        });

        let activity: PendingActivity =
            serde_json::from_value(json_activity).expect("Should parse activity");

        assert_eq!(activity.activity_id, activity_id);
        assert_eq!(activity.workflow_id, workflow_id);
        assert_eq!(activity.activity_key, "test_activity");
        assert_eq!(activity.namespace, "payments");
        assert_eq!(activity.name, "authorize");
        assert_eq!(activity.timeout_seconds, Some(600));
        assert!(activity.settings.is_some());
    }

    #[tokio::test]
    async fn test_worker_api_client_creation() {
        let client = WorkerApiClient::new(
            "http://localhost:8080".to_string(),
            "test_client".to_string(),
            "test_secret".to_string(),
        );

        // Just verify we can create the client without panicking
        assert_eq!(client.api_url, "http://localhost:8080");
    }

    #[tokio::test]
    async fn test_empty_poll_response_parsing() {
        let json_response = json!({
            "activities": [],
            "count": 0
        });

        let response: PollActivitiesResponse =
            serde_json::from_value(json_response).expect("Should parse empty response");

        assert_eq!(response.count, 0);
        assert_eq!(response.activities.len(), 0);
    }
}
