use std::sync::Arc;
use std::time::Duration;

use service_sdk::my_http_server::web_sockets::*;

use crate::app::AppContext;

/// The `/ws` endpoint Home holds open.
///
/// One message shape in each direction, and both are as small as they can be:
///
/// * client -> server: `{"watch":"<projectId>"}` — sent on connect and again whenever the project
///   dropdown changes, so switching boards does not need a reconnect.
/// * server -> client: `{"projectChanged":"<projectId>"}` — an invalidation signal. The client re-reads
///   through the ordinary REST call. Never a delta: with no second copy of the state on the client there
///   is nothing that can drift out of sync.
pub struct HomeWsCallbacks {
    app: Arc<AppContext>,
}

impl HomeWsCallbacks {
    pub fn new(app: Arc<AppContext>) -> Self {
        Self { app }
    }
}

#[async_trait::async_trait]
impl MyWebSocketCallback for HomeWsCallbacks {
    async fn connected(
        &self,
        my_web_socket: Arc<MyWebSocket>,
        http_request: MyWebSocketHttpRequest,
        _disconnect_timeout: Duration,
    ) -> Result<(), WebSocketConnectedFail> {
        // The browser WebSocket API cannot send custom headers, so the session token arrives as a query
        // parameter. The `Authorization` header is honoured too, for anything that is not a browser.
        let token = http_request
            .get_headers()
            .get("Authorization")
            .and_then(|value| value.to_str().ok())
            .map(|value| {
                value
                    .trim()
                    .strip_prefix("Bearer ")
                    .unwrap_or(value.trim())
                    .to_string()
            })
            .or_else(|| {
                http_request.get_uri().query().and_then(|query| {
                    query
                        .split('&')
                        .find_map(|pair| pair.strip_prefix("token="))
                        .map(|itm| itm.to_string())
                })
            })
            .unwrap_or_default();

        let Some(session) = crate::auth::SessionToken::parse(&token, &self.app.session_key) else {
            return Err(WebSocketConnectedFail {
                reason: "Not authenticated".to_string(),
                // Not logged: an expired token on a reconnecting tab is routine, and logging it would
                // turn every overnight laptop into noise.
                write_to_logs: false,
            });
        };

        let admin_in_settings = self.app.is_admin_in_settings(&session.email).await;
        let user = self.app.board.read().get_user(&session.email);

        // Disabling somebody has to close the door here too, not only on the REST side.
        let is_admin = match &user {
            Some(user) if user.disabled => {
                return Err(WebSocketConnectedFail {
                    reason: "Account is disabled".to_string(),
                    write_to_logs: false,
                });
            }
            Some(user) => user.admin || admin_in_settings,
            None if admin_in_settings => true,
            None => {
                return Err(WebSocketConnectedFail {
                    reason: "Not authenticated".to_string(),
                    write_to_logs: false,
                });
            }
        };

        self.app
            .subscribers
            .add(my_web_socket, session.email, is_admin);

        Ok(())
    }

    async fn disconnected(&self, my_web_socket: &MyWebSocket) {
        self.app.subscribers.remove(my_web_socket.id);
    }

    async fn on_message(&self, my_web_socket: Arc<MyWebSocket>, message: WsMessage) {
        let WsMessage::Text(payload) = message else {
            // Anything that is not text is not part of this protocol. Ignored rather than treated as an
            // error: a ping frame or a stray binary message is not worth disconnecting a board over.
            return;
        };

        let Some(connection) = self.app.subscribers.get_by_id(my_web_socket.id) else {
            return;
        };

        let Some(project_id) = parse_watch(payload.as_str()) else {
            connection
                .send_error("expected {\"watch\":\"<projectId>\"}")
                .await;
            return;
        };

        // Membership is re-checked on every subscribe, not only at connect: a tab left open across a
        // membership change must not keep receiving a board it may no longer see.
        let allowed = connection.is_admin
            || self
                .app
                .board
                .read()
                .get_project(&project_id)
                .map(|project| project.is_member(&connection.email))
                .unwrap_or(false);

        if !allowed {
            connection.send_error("no access to this project").await;
            return;
        }

        connection.watch(project_id);
    }
}

/// Read `{"watch":"<projectId>"}`.
///
/// Parsed with serde_json rather than by hand — the id is a `SortableId` and contains a `-`, and a
/// hand-rolled split would go wrong the first time somebody sends a quoted value with an escape in it.
fn parse_watch(payload: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(payload).ok()?;
    let project_id = value.get("watch")?.as_str()?.trim();

    if project_id.is_empty() {
        None
    } else {
        Some(project_id.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_watch_message_is_read() {
        assert_eq!(
            parse_watch("{\"watch\":\"1753800000000000-abc\"}"),
            Some("1753800000000000-abc".to_string())
        );
    }

    #[test]
    fn anything_else_is_refused_rather_than_guessed_at() {
        for payload in [
            "",
            "not json",
            "{}",
            "{\"watch\":\"\"}",
            "{\"watch\":null}",
            "{\"watch\":42}",
            "{\"other\":\"x\"}",
        ] {
            assert_eq!(parse_watch(payload), None, "{payload:?} should not parse");
        }
    }
}
