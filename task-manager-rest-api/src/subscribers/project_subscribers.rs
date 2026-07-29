use std::sync::Arc;

use ahash::AHashMap;
use arc_swap::ArcSwap;
use parking_lot::Mutex;
use service_sdk::my_http_server::web_sockets::MyWebSocket;

use super::HomeConnection;

type ConnectionsMap = AHashMap<i64, Arc<HomeConnection>>;

/// Every Home currently connected.
///
/// Same shape as the board state and for the same reason: read on every mutation, written only when a
/// browser tab opens or closes. `ArcSwap` means a notification takes no lock at all, and the
/// `Mutex<()>` only stops two concurrent connects from losing one of the two.
///
/// Sends happen against a snapshot with no lock held — one client on a stalled socket must not be able
/// to hold up every other watcher, and a `parking_lot` guard is `!Send`, so the compiler refuses the
/// mistake rather than leaving it to review.
pub struct ProjectSubscribers {
    connections: ArcSwap<ConnectionsMap>,
    write_lock: Mutex<()>,
}

impl ProjectSubscribers {
    pub fn new() -> Self {
        Self {
            connections: ArcSwap::from_pointee(AHashMap::new()),
            write_lock: Mutex::new(()),
        }
    }

    pub fn add(&self, ws: Arc<MyWebSocket>, email: String, is_admin: bool) -> Arc<HomeConnection> {
        let id = ws.id;
        let connection = Arc::new(HomeConnection::new(ws, email, is_admin));

        let _guard = self.write_lock.lock();
        let mut next: ConnectionsMap = (**self.connections.load()).clone();
        next.insert(id, connection.clone());
        self.connections.store(Arc::new(next));

        connection
    }

    pub fn remove(&self, id: i64) -> Option<Arc<HomeConnection>> {
        let _guard = self.write_lock.lock();
        let mut next: ConnectionsMap = (**self.connections.load()).clone();
        let removed = next.remove(&id);
        self.connections.store(Arc::new(next));

        removed
    }

    pub fn get_by_id(&self, id: i64) -> Option<Arc<HomeConnection>> {
        self.connections.load().get(&id).cloned()
    }

    pub fn amount(&self) -> usize {
        self.connections.load().len()
    }

    /// Tell everyone watching this board that it changed.
    ///
    /// Called from `scripts/` after the change is in Postgres **and** in memory — notifying any earlier
    /// would send a client to re-read a board that does not show the change yet.
    pub async fn notify_project_changed(&self, project_id: &str) {
        let snapshot = self.connections.load_full();

        for connection in snapshot.values() {
            if connection.is_watching(project_id) {
                connection.send_project_changed(project_id).await;
            }
        }
    }
}

impl Default for ProjectSubscribers {
    fn default() -> Self {
        Self::new()
    }
}
