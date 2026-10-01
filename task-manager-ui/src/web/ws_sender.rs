use std::cell::RefCell;

use futures::channel::mpsc::{Sender, channel};

// The write half of the WebSocket, reachable from anywhere in the client.
//
// A channel rather than the socket itself: the socket is owned by the task reading it, and wasm is
// single-threaded, so a `thread_local` sender is both sound and the least machinery that works. It is
// also why a `watch` sent before the socket finishes opening is not lost — it waits in the channel.
thread_local! {
    static WS_SENDER: RefCell<Option<Sender<()>>> = const { RefCell::new(None) };
    static WATCHED_PROJECT: RefCell<String> = const { RefCell::new(String::new()) };
}

/// Notifications coalesce; the selected project itself lives outside the channel.
const CAPACITY: usize = 1;

/// Create the channel and hand the receiving half to the socket task.
pub fn install_ws_sender() -> futures::channel::mpsc::Receiver<()> {
    let (mut sender, receiver) = channel(CAPACITY);
    if watch_payload().is_some() {
        let _ = sender.try_send(());
    }

    WS_SENDER.with(|cell| {
        *cell.borrow_mut() = Some(sender);
    });

    receiver
}

/// Tell the server which board to push about.
///
/// Keeps the selection while disconnected and avoids resubscribing after an unrelated repaint.
pub fn watch_project(prefix: &str) {
    let changed = WATCHED_PROJECT.with(|cell| {
        let mut current = cell.borrow_mut();
        if current.as_str() == prefix {
            return false;
        }
        *current = prefix.to_string();
        true
    });
    if !changed {
        return;
    }

    WS_SENDER.with(|cell| {
        if let Some(sender) = cell.borrow_mut().as_mut() {
            let _ = sender.try_send(());
        }
    });
}

// The channel only wakes the sender. Reading the last choice here coalesces rapid switches and
// preserves the desired project while the connection is down.
pub fn watch_payload() -> Option<String> {
    WATCHED_PROJECT.with(|cell| {
        let prefix = cell.borrow();
        (!prefix.is_empty()).then(|| serde_json::json!({"watch": prefix.as_str()}).to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;
    #[test]
    fn reconnect_and_rapid_switching_keep_the_latest_project() {
        watch_project("OLD");
        let mut outgoing = install_ws_sender();
        for index in 0..100 {
            watch_project(&format!("P{index}"));
        }
        assert_eq!(futures::executor::block_on(outgoing.next()), Some(()));
        assert_eq!(watch_payload().unwrap(), r#"{"watch":"P99"}"#);
        let mut reconnected = install_ws_sender();
        assert_eq!(futures::executor::block_on(reconnected.next()), Some(()));
        watch_project("P99");
        assert!(
            reconnected.try_next().is_err(),
            "re-applying a snapshot must not resubscribe to the same project"
        );
        assert_eq!(watch_payload().unwrap(), r#"{"watch":"P99"}"#);
    }
}
