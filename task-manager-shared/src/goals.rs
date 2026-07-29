use my_http_utils::macros::{MyHttpInput, MyHttpObjectStructure};
use serde::{Deserialize, Serialize};

use crate::tasks::TaskCommentResponse;

// Never put `///` doc comments on fields of a struct deriving MyHttpInput or
// MyHttpObjectStructure: the macro's attribute parser panics with `Somehow we got Punct here: =`.

// A goal, as the Goals screen draws it.
//
// The container work is done around — an epic. `id` is its handle, `RMS-G7`: the number comes from the
// same per-project counter task numbers come from, so no number names both a task and a goal, and the `G`
// says which of the two you are holding.
//
// `status` is derived from whether the goal is closed, not stored, so it cannot disagree with
// `closed_unix_seconds`. Progress is `done_amount` out of `tasks_amount`, also derived on every read, and
// it counts ARCHIVED tasks too: a goal can only be closed once all its tasks are done, and by then the
// oldest of them have aged off the board — a count that skipped those would report finished work as
// half-done. Which is why the Goals screen must not recompute these numbers from the tasks it holds.
//
// The link lives on the TASK, not here: a task says which goal it is part of.
#[derive(Serialize, Deserialize, MyHttpObjectStructure, Clone, Debug, PartialEq)]
pub struct GoalResponse {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub description: String,
    pub status: String,
    pub tasks_amount: i32,
    pub done_amount: i32,
    #[serde(default)]
    pub comments: Vec<TaskCommentResponse>,
    pub created_unix_seconds: i64,
    pub updated_unix_seconds: i64,
    // When the goal was closed, and absent while it is open. What the archive window is measured from: a
    // goal closed longer ago than the project's window is not returned unless asked for.
    pub closed_unix_seconds: Option<i64>,
}

#[derive(Serialize, Deserialize, MyHttpObjectStructure, Clone, Debug, PartialEq)]
pub struct GoalsResponse {
    pub goals: Vec<GoalResponse>,
}

#[derive(MyHttpInput)]
pub struct GetGoalsInputModel {
    #[http_body(name: "projectId", description: "Which project's goals to read")]
    pub project_id: String,
    #[http_body(
        name: "includeArchived",
        description: "Include goals closed longer ago than the project's archive window. Omitted means the live list"
    )]
    pub include_archived: Option<bool>,
}

// Which goal's work to read, for the Goals screen expanding one.
//
// Separate from the board read on purpose. The board stops at the archive window; a goal's list must not,
// because a goal closes only once every task is done and by then the oldest of them have aged off the
// board — the list under it would then disagree with the counter beside it.
#[derive(MyHttpInput)]
pub struct GetGoalTasksInputModel {
    #[http_body(name: "projectId", description: "Which project the goal is on")]
    pub project_id: String,
    #[http_body(name: "goal", description: "Which goal, by id — RMS-G7 — or by its bare number")]
    pub goal: String,
}
