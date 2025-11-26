#[cfg(test)]
mod tests {
    use crate::client::*;
    use serde_json::json;
    use uuid::Uuid;

    // =========================================================================
    // WorkerApiClient creation tests
    // =========================================================================

    #[test]
    fn test_worker_api_client_stores_credentials() {
        let client = WorkerApiClient::new(
            "http://api.example.com".to_string(),
            "my_client_id".to_string(),
            "my_secret".to_string(),
        );

        // Verify all fields are stored correctly
        assert_eq!(client.api_url, "http://api.example.com");
        assert_eq!(client.client_id, "my_client_id");
        assert_eq!(client.client_secret, "my_secret");
    }

    #[test]
    fn test_worker_api_client_clone() {
        let client = WorkerApiClient::new(
            "http://localhost:8080".to_string(),
            "test_client".to_string(),
            "test_secret".to_string(),
        );

        // Verify client can be cloned (needed for spawning tasks)
        let cloned = client.clone();
        assert_eq!(cloned.api_url, client.api_url);
        assert_eq!(cloned.client_id, client.client_id);
        assert_eq!(cloned.client_secret, client.client_secret);
    }

    #[tokio::test]
    async fn test_token_starts_as_none() {
        let client = WorkerApiClient::new(
            "http://localhost:8080".to_string(),
            "test_client".to_string(),
            "test_secret".to_string(),
        );

        // Token should start as None
        let token_lock = client.token.read().await;
        assert!(token_lock.is_none());
    }

    // =========================================================================
    // Response parsing tests
    // =========================================================================

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
                    "worker": "builtin",
                    "activity_name": "echo",
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
        assert_eq!(response.activities[0].worker, "builtin");
        assert_eq!(response.activities[0].activity_name, "echo");
    }

    #[tokio::test]
    async fn test_pending_activity_parsing() {
        let activity_id = Uuid::now_v7();
        let workflow_id = Uuid::now_v7();

        let json_activity = json!({
            "activity_id": activity_id,
            "workflow_id": workflow_id,
            "activity_key": "test_activity",
            "worker": "payments",
            "activity_name": "authorize",
            "parameters": {"amount": 100.50},
            "settings": {"retry_limit": 3},
            "timeout_seconds": 600
        });

        let activity: PendingActivity =
            serde_json::from_value(json_activity).expect("Should parse activity");

        assert_eq!(activity.activity_id, activity_id);
        assert_eq!(activity.workflow_id, workflow_id);
        assert_eq!(activity.activity_key, "test_activity");
        assert_eq!(activity.worker, "payments");
        assert_eq!(activity.activity_name, "authorize");
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

    #[tokio::test]
    async fn test_pending_activity_with_output_definitions() {
        let activity_id = Uuid::now_v7();
        let workflow_id = Uuid::now_v7();

        let json_activity = json!({
            "activity_id": activity_id,
            "workflow_id": workflow_id,
            "activity_key": "test_activity",
            "worker": "data",
            "activity_name": "process",
            "parameters": {"data": [1, 2, 3]},
            "settings": null,
            "timeout_seconds": null,
            "output_definitions": [
                {"name": "result", "type": "value"},
                {"name": "report", "type": "file"}
            ]
        });

        let activity: PendingActivity = serde_json::from_value(json_activity)
            .expect("Should parse activity with output_definitions");

        assert_eq!(activity.activity_id, activity_id);
        assert!(activity.output_definitions.is_some());
        assert!(activity.timeout_seconds.is_none());
        assert!(activity.settings.is_none());
    }

    #[tokio::test]
    async fn test_pending_activity_minimal_fields() {
        let activity_id = Uuid::now_v7();
        let workflow_id = Uuid::now_v7();

        // Minimal activity with only required fields
        let json_activity = json!({
            "activity_id": activity_id,
            "workflow_id": workflow_id,
            "activity_key": "minimal",
            "worker": "test",
            "activity_name": "noop",
            "parameters": {}
        });

        let activity: PendingActivity =
            serde_json::from_value(json_activity).expect("Should parse minimal activity");

        assert_eq!(activity.activity_id, activity_id);
        assert_eq!(activity.activity_key, "minimal");
        assert!(activity.settings.is_none());
        assert!(activity.timeout_seconds.is_none());
        assert!(activity.output_definitions.is_none());
    }

    #[tokio::test]
    async fn test_poll_response_multiple_activities() {
        let json_response = json!({
            "activities": [
                {
                    "activity_id": Uuid::now_v7(),
                    "workflow_id": Uuid::now_v7(),
                    "activity_key": "step1",
                    "worker": "test",
                    "activity_name": "process",
                    "parameters": {"step": 1}
                },
                {
                    "activity_id": Uuid::now_v7(),
                    "workflow_id": Uuid::now_v7(),
                    "activity_key": "step2",
                    "worker": "test",
                    "activity_name": "process",
                    "parameters": {"step": 2}
                },
                {
                    "activity_id": Uuid::now_v7(),
                    "workflow_id": Uuid::now_v7(),
                    "activity_key": "step3",
                    "worker": "test",
                    "activity_name": "process",
                    "parameters": {"step": 3}
                }
            ],
            "count": 3
        });

        let response: PollActivitiesResponse =
            serde_json::from_value(json_response).expect("Should parse response");

        assert_eq!(response.count, 3);
        assert_eq!(response.activities.len(), 3);
        assert_eq!(response.activities[0].activity_key, "step1");
        assert_eq!(response.activities[1].activity_key, "step2");
        assert_eq!(response.activities[2].activity_key, "step3");
    }

    #[test]
    fn test_pending_activity_debug_format() {
        let activity_id = Uuid::now_v7();
        let workflow_id = Uuid::now_v7();

        let activity = PendingActivity {
            activity_id,
            workflow_id,
            activity_key: "debug_test".to_string(),
            worker: "test".to_string(),
            activity_name: "debug".to_string(),
            parameters: json!({"key": "value"}),
            settings: None,
            timeout_seconds: Some(30),
            output_definitions: None,
        };

        // Verify Debug trait is implemented
        let debug_str = format!("{:?}", activity);
        assert!(debug_str.contains("debug_test"));
        assert!(debug_str.contains("PendingActivity"));
    }

    #[test]
    fn test_poll_response_debug_format() {
        let response = PollActivitiesResponse {
            activities: vec![],
            count: 0,
        };

        // Verify Debug trait is implemented
        let debug_str = format!("{:?}", response);
        assert!(debug_str.contains("PollActivitiesResponse"));
        assert!(debug_str.contains("count: 0"));
    }
}
