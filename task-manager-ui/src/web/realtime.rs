use crate::{models::ServerWsMessage, states::AppState};
use dioxus::prelude::*;
use futures::{FutureExt, SinkExt, StreamExt};
use gloo_net::websocket::{Message, futures::WebSocket};
use std::{cell::Cell, rc::Rc, time::Duration};

pub fn reconnect_delay(attempt: u32) -> Duration {
    Duration::from_secs((1_u64 << attempt.min(5)).min(30))
}

pub fn run_ws(mut app: Signal<AppState>) {
    spawn(async move {
        let settings = dioxus_utils::js::GlobalAppSettings::new();
        let origin = settings.get_origin();
        let base = if origin.starts_with("https") {
            origin.replacen("https", "wss", 1)
        } else {
            origin.replacen("http", "ws", 1)
        };
        let url = format!("{}/ws", base.trim_end_matches('/'));
        let mut attempt = 0;
        loop {
            app.write().ws_status = if attempt == 0 {
                "Connecting"
            } else {
                "Reconnecting"
            }
            .into();
            let started = js_sys::Date::now();
            let outgoing = super::install_ws_sender();
            if let Ok(ws) = WebSocket::open(&url) {
                connection(ws, outgoing, app).await;
            }
            app.write().ws_status = "Reconnecting".into();
            if js_sys::Date::now() - started > 30_000.0 {
                attempt = 0;
            }
            dioxus_utils::js::sleep(reconnect_delay(attempt)).await;
            attempt = attempt.saturating_add(1);
        }
    });
}

async fn connection(
    ws: WebSocket,
    mut outgoing: futures::channel::mpsc::Receiver<()>,
    mut app: Signal<AppState>,
) {
    let (mut write, mut read) = ws.split();
    let last_received = Rc::new(Cell::new(js_sys::Date::now()));
    let received = last_received.clone();
    let reader = async move {
        let mut initialized = false;
        while let Some(Ok(message)) = read.next().await {
            if let Message::Text(text) = message {
                match ServerWsMessage::parse(&text) {
                    ServerWsMessage::BoardSnapshot(snapshot) => {
                        received.set(js_sys::Date::now());
                        let mut state = app.write();
                        if !initialized {
                            state.ws_generation += 1;
                            initialized = true;
                        }
                        state.ws_status = "Live".into();
                        state.last_synced_unix_seconds = Some(js_sys::Date::now() as i64 / 1000);
                        state.board_pushed(snapshot);
                    }
                    ServerWsMessage::ProjectChanged => {
                        received.set(js_sys::Date::now());
                        app.write().board_invalidated();
                    }
                    ServerWsMessage::Pong => received.set(js_sys::Date::now()),
                    ServerWsMessage::Error(error) => {
                        super::console_log(&format!(
                            "Board sync error: {}",
                            error.chars().take(200).collect::<String>()
                        ));
                        app.write().ws_status = "Sync unavailable".into();
                        return;
                    }
                    ServerWsMessage::Unknown(raw) => {
                        super::console_log(&format!(
                            "Ignored unrecognized board message ({} bytes)",
                            raw.len()
                        ));
                    }
                }
            }
        }
    }
    .fuse();
    let writer = async move {
        loop {
            let next = outgoing.next().fuse();
            let heartbeat = dioxus_utils::js::sleep(Duration::from_secs(20)).fuse();
            futures::pin_mut!(next, heartbeat);
            let payload = futures::select! {
                event = next => {
                    if event.is_none() { return; }
                    match super::watch_payload() { Some(payload) => payload, None => continue }
                },
                _ = heartbeat => "{\"ping\":true}".into(),
            };
            if write.send(Message::Text(payload)).await.is_err() {
                return;
            }
        }
    }
    .fuse();
    let watchdog = async move {
        loop {
            dioxus_utils::js::sleep(Duration::from_secs(10)).await;
            if js_sys::Date::now() - last_received.get() > 60_000.0 {
                return;
            }
        }
    }
    .fuse();
    futures::pin_mut!(reader, writer, watchdog);
    futures::select! { _ = reader => {}, _ = writer => {}, _ = watchdog => {} }
}

#[cfg(test)]
mod tests {
    #[test]
    fn retry_backoff_is_bounded_even_after_long_outages() {
        assert_eq!(super::reconnect_delay(0).as_secs(), 1);
        assert_eq!(super::reconnect_delay(3).as_secs(), 8);
        assert_eq!(super::reconnect_delay(u32::MAX).as_secs(), 30);
    }
}
