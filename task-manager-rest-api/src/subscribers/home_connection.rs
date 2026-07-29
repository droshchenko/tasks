use std::sync::Arc;

use arc_swap::ArcSwap;
use service_sdk::my_http_server::web_sockets::{MyWebSocket, WsMessage};

/// One connected Home.
///
/// `watching` is swappable rather than fixed at connect time: Home has a project dropdown, and
/// picking another board should re-target the subscription without tearing down the socket.
pub struct HomeConnection {
    pub ws: Arc<MyWebSocket>,
    pub email: String,
    pub is_admin: bool,
    watching: ArcSwap<String>,
}

impl HomeConnection {
    pub fn new(ws: Arc<MyWebSocket>, email: String, is_admin: bool) -> Self {
        Self {
            ws,
            email,
            is_admin,
            watching: ArcSwap::from_pointee(String::new()),
        }
    }

    pub fn watch(&self, project_id: String) {
        self.watching.store(Arc::new(project_id));
    }

    pub fn is_watching(&self, project_id: &str) -> bool {
        self.watching.load().as_str() == project_id
    }

    /// Tell this Home that its board changed, so it re-reads.
    ///
    /// The payload is an invalidation signal and nothing more — no task, no delta. There is no second
    /// copy of the state on the client, so there is nothing that can drift out of sync.
    pub async fn send_project_changed(&self, project_id: &str) {
        let payload = format!("{{\"projectChanged\":\"{project_id}\"}}");
        self.ws
            .send_message(std::iter::once(WsMessage::Text(payload.into())))
            .await;
    }

    pub async fn send_error(&self, message: &str) {
        // Escaped so a message containing a quote cannot produce a payload the client fails to parse.
        let payload = format!(
            "{{\"error\":{}}}",
            serde_json::to_string(message).unwrap_or_else(|_| "\"error\"".to_string())
        );
        self.ws
            .send_message(std::iter::once(WsMessage::Text(payload.into())))
            .await;
    }
}
