use my_http_utils::macros::{MyHttpInput, MyHttpObjectStructure};
use serde::{Deserialize, Serialize};

// Never put `///` doc comments on fields of a struct deriving MyHttpInput or
// MyHttpObjectStructure: the macro's attribute parser panics with `Somehow we got Punct here: =`.
//
// There are no task *input* models in this file on purpose. Every task mutation arrives through
// `/mcp`; the REST surface reads the board and configures the product, and nothing else. If you
// find yourself adding `CreateTaskInputModel` here, the design changed — update README.md first.

// One comment on a task's thread.
//
// `who` is a user's email or the literal `claude`. It is a plain string rather than a reference to
// the roster: MCP has no session to derive an author from, so it passes one, and an author whose
// user row was removed still has to render.
#[derive(Serialize, Deserialize, MyHttpObjectStructure, Clone, Debug, PartialEq)]
pub struct TaskCommentResponse {
    pub moment_unix_seconds: i64,
    pub who: String,
    pub text: String,
}

// One sticker, as Home draws it.
//
// `status` and `kind` are raw ids, not enums: both vocabularies are configured per project at
// runtime, so a value this build has never heard of must render rather than fail the read. An
// unknown `status` is drawn in Todo; an unknown or absent `kind` is drawn without a kind.
//
// `blocked` and `blocks` are derived server-side against the whole project and are never stored.
// `blocked` is true while any id in `depends_on` names a task that is not Done — including an id
// that matches no task at all, so a typo or a deleted blocker keeps the task blocked instead of
// quietly freeing it. `blocks` is the reverse edge: who is waiting on this one.
#[derive(Serialize, Deserialize, MyHttpObjectStructure, Clone, Debug, PartialEq)]
pub struct TaskResponse {
    pub id: String,
    pub project_id: String,
    pub text: String,
    pub status: String,
    pub kind: Option<String>,
    pub assignee: Option<String>,
    // The assignee's display name, resolved from the roster. Absent when the assignee is `claude`
    // or an email with no user row — Home then shows the raw assignee value.
    pub assignee_name: Option<String>,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub blocks: Vec<String>,
    pub blocked: bool,
    #[serde(default)]
    pub comments: Vec<TaskCommentResponse>,
    pub created_unix_seconds: i64,
    pub updated_unix_seconds: i64,
}

// A whole board in one response, oldest task first.
//
// Not paged and not capped: a project's tasks are a hand-written list held entirely in memory, and
// Home renders all the columns at once anyway.
#[derive(Serialize, Deserialize, MyHttpObjectStructure, Clone, Debug, PartialEq)]
pub struct TasksResponse {
    pub tasks: Vec<TaskResponse>,
}

#[derive(MyHttpInput)]
pub struct GetTasksInputModel {
    #[http_query(name: "projectId", description: "Which project's board to read")]
    pub project_id: String,
}
