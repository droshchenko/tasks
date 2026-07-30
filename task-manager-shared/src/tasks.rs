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
// `who` is a user's email or the literal `AI`. It is a plain string rather than a reference to
// the roster: MCP has no session to derive an author from, so it passes one, and an author whose
// user row was removed still has to render.
#[derive(Serialize, Deserialize, MyHttpObjectStructure, Clone, Debug, PartialEq)]
pub struct TaskCommentResponse {
    pub moment_unix_seconds: i64,
    pub who: String,
    pub text: String,
}

// A task named by another task, and what it is doing.
//
// A handle on its own is not enough for a reader looking at a dependency: the question a dependency
// raises is "is that done yet", and the answer is the whole reason it is on screen. The status is
// `effective_status` — the same leniency a task's own status gets, so a blocker sitting in a column
// its project no longer has reads as Todo rather than as a column nobody recognises.
#[derive(Serialize, Deserialize, MyHttpObjectStructure, Clone, Debug, PartialEq)]
pub struct TaskLinkResponse {
    pub id: String,
    pub status: String,
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
    // Which goal this task is part of — its handle, `RMS-G7` — and its name. Both absent for a standalone
    // task, which is a normal state and not an unfinished one. Also both absent if the stored number names
    // no goal, which reads the same way on purpose: such a task is standalone rather than dangling.
    pub goal: Option<String>,
    pub goal_name: Option<String>,
    // The goal's palette colour, so the board can mark the card with it without joining a goal list. Absent
    // exactly when `goal` is.
    pub goal_color: Option<String>,
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
    // The status of every task named in `depends_on` or `blocks`, so a reader shown a dependency can be
    // shown what it is doing without a second call per id. Derived, never stored. An id matching no task
    // has no entry here — the same case that keeps `blocked` true, and a reader showing the handle with
    // no status beside it is telling the truth about it.
    #[serde(default)]
    pub link_statuses: Vec<TaskLinkResponse>,
    pub blocked: bool,
    #[serde(default)]
    pub comments: Vec<TaskCommentResponse>,
    pub created_unix_seconds: i64,
    pub updated_unix_seconds: i64,
    // When the task landed in Done, and absent whenever it is not there. Home shows it, and it is what
    // the seven-day archive window is measured from — a task closed longer ago than that is not returned
    // at all.
    pub closed_unix_seconds: Option<i64>,
}

// A whole board in one response, oldest task first.
//
// Not paged and not capped: a project's tasks are a hand-written list held entirely in memory, and
// Home renders all the columns at once anyway.
#[derive(Serialize, Deserialize, MyHttpObjectStructure, Clone, Debug, PartialEq)]
pub struct TasksResponse {
    pub tasks: Vec<TaskResponse>,
}

// The exact task a handle names, plus which board it is on.
//
// Answered by the server rather than found in the loaded board, and that is the point: work closed more
// than seven days ago is not in the board read at all, and a handle can name a project other than the one
// on screen. Both are exactly when looking a number up is worth doing.
#[derive(Serialize, Deserialize, MyHttpObjectStructure, Clone, Debug, PartialEq)]
pub struct FindTaskResponse {
    pub task: Option<TaskResponse>,
    // Set instead of `task` when the query named a GOAL — either written `RMS-G7`, or a bare number that
    // turned out to be a goal's. One counter serves both kinds per project, so a number is one or the
    // other and never both. Search is the only way to reach an archived goal, which is why it answers for
    // goals at all.
    pub goal: Option<crate::goals::GoalResponse>,
    pub project_id: String,
    pub project_prefix: String,
    pub project_name: String,
    // True when the hit is older than the project's archive window, so it is NOT on the board. Said out
    // loud, or somebody goes hunting through the columns for a card that is not drawn.
    pub archived: bool,
    // Why there is no task, in words. Empty on a hit — an id that is not there has to come back as a
    // message, never as an empty result that reads like "there is nothing there".
    pub not_found: String,
}

#[derive(MyHttpInput)]
pub struct FindTaskInputModel {
    #[http_body(name: "query", description: "A task id such as RMS-42 or RMS-000042, or a goal id such as RMS-G7")]
    pub query: String,
}

#[derive(MyHttpInput)]
pub struct GetTasksInputModel {
    #[http_body(name: "projectId", description: "Which project's board to read")]
    pub project_id: String,
}
