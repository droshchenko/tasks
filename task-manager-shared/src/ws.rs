//! What travels down `/ws`.
//!
//! Plain serde and no `MyHttpObjectStructure`: none of this appears in an HTTP contract, so there is no
//! OpenAPI shape to describe. It is shared all the same, because the two sides have to agree and a payload
//! written by hand on the server and parsed by hand on the client is two things to keep in step.
//!
//! Envelope keys are camelCase, matching the `{"watch":…}` the client sends. The models inside are the same
//! wire models the REST endpoints return, field names and all — a board that arrived through the socket has
//! to be indistinguishable from one that arrived through `/api/tasks/v1/list`, or the client would need two
//! ways to read the same thing.

use serde::{Deserialize, Serialize};

use crate::tasks::TaskResponse;

// A whole board, pushed because it changed.
//
// The snapshot itself rather than a signal to go and re-read: a re-read empties the screen while it is in
// flight, and what the reader sees for that moment is a spinner where their board was. It is not a delta
// either — a delta is only correct if the client's copy is, and a wholesale replacement cannot drift.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct BoardSnapshot {
    pub project_id: String,
    pub tasks: Vec<TaskResponse>,
}

// One message from the server. Exactly one of the fields is set on any given message.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct ServerWsPayload {
    // Kept alongside `board_snapshot` for one reason only: a tab opened before the snapshot existed knows
    // this key and nothing else, and would otherwise sit there live-looking and never repaint until somebody
    // reloaded it. It can go once no such tab can still be open.
    #[serde(rename = "projectChanged", skip_serializing_if = "Option::is_none")]
    pub project_changed: Option<String>,
    #[serde(rename = "boardSnapshot", skip_serializing_if = "Option::is_none")]
    pub board_snapshot: Option<BoardSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl ServerWsPayload {
    /// A board that changed, carrying the board.
    pub fn board(snapshot: BoardSnapshot) -> Self {
        Self {
            project_changed: Some(snapshot.project_id.clone()),
            board_snapshot: Some(snapshot),
            error: None,
        }
    }

    /// A board that changed with no snapshot to send — the project is not in memory to build one from.
    /// The client re-reads, which is what it did before snapshots existed.
    pub fn project_changed(project_id: &str) -> Self {
        Self {
            project_changed: Some(project_id.to_string()),
            ..Default::default()
        }
    }

    pub fn error(message: &str) -> Self {
        Self {
            error: Some(message.to_string()),
            ..Default::default()
        }
    }
}
