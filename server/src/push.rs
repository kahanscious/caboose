//! Push notification service for sending FCM messages to mobile devices.
//!
//! Currently a stub that logs pushes instead of sending them — a Firebase
//! project server key is needed to send real pushes via the FCM API.

use std::collections::HashMap;

/// Service for sending push notifications to mobile devices via FCM.
#[derive(Default)]
pub struct PushService;

impl PushService {
    pub fn new() -> Self {
        Self
    }

    /// Send a push notification to a device.
    ///
    /// Currently a stub — logs the push payload instead of actually sending it.
    /// To enable real pushes, configure a Firebase server key and POST to
    /// `https://fcm.googleapis.com/fcm/send` with the legacy API, or use the
    /// FCM HTTP v1 API with a service account.
    pub async fn send_push(
        &self,
        token: &str,
        title: &str,
        body: &str,
        data: HashMap<String, String>,
    ) {
        tracing::info!(
            "push stub: token={}... title={:?} body={:?} data={:?}",
            &token[..token.len().min(10)],
            title,
            body,
            data,
        );

        // TODO: When Firebase is configured, uncomment and use:
        //
        // let payload = serde_json::json!({
        //     "to": token,
        //     "notification": {
        //         "title": title,
        //         "body": body,
        //     },
        //     "data": data,
        // });
        //
        // let client = reqwest::Client::new();
        // let res = client
        //     .post("https://fcm.googleapis.com/fcm/send")
        //     .header("Authorization", format!("key={}", server_key))
        //     .header("Content-Type", "application/json")
        //     .json(&payload)
        //     .send()
        //     .await;
        //
        // match res {
        //     Ok(resp) if resp.status().is_success() => {
        //         tracing::info!("push sent successfully to {}", &token[..10]);
        //     }
        //     Ok(resp) => {
        //         tracing::warn!("push send failed: status={}", resp.status());
        //     }
        //     Err(e) => {
        //         tracing::warn!("push send error: {e}");
        //     }
        // }
    }

    /// Send a tool approval push notification.
    pub async fn notify_tool_approval(&self, token: &str, tool_name: &str, tool_use_id: &str) {
        let mut data = HashMap::new();
        data.insert("type".to_string(), "tool_approval".to_string());
        data.insert("tool_use_id".to_string(), tool_use_id.to_string());

        self.send_push(
            token,
            "Tool Approval",
            &format!("Caboose wants to run: {tool_name}"),
            data,
        )
        .await;
    }

    /// Send an agent completion push notification.
    pub async fn notify_agent_complete(&self, token: &str, output_tokens: u32) {
        let mut data = HashMap::new();
        data.insert("type".to_string(), "agent_complete".to_string());

        self.send_push(
            token,
            "Agent Complete",
            &format!("Task complete ({output_tokens} tokens)"),
            data,
        )
        .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn push_stub_does_not_panic() {
        let service = PushService::new();
        service
            .notify_tool_approval("fake-token-12345", "shell", "tool-1")
            .await;
        service.notify_agent_complete("fake-token-12345", 2847).await;
    }
}
