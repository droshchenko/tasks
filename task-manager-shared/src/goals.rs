use my_http_utils::macros::{MyHttpInput, MyHttpObjectStructure};
use serde::{Deserialize, Serialize};

// Never put `///` doc comments on fields of a struct deriving MyHttpInput or
// MyHttpObjectStructure: the macro's attribute parser panics with `Somehow we got Punct here: =`.

// A goal, as the Goals screen draws it.
//
// A container and nothing more — an epic. It has no status of its own and nothing to move: progress is
// `done_amount` out of `tasks_amount`, derived on every read, so it cannot disagree with the board the way
// a stored "achieved" flag eventually would.
//
// The link lives on the TASK, not here: a task says which goal it is part of. So deleting a goal takes
// nothing with it — its tasks read as standalone from then on, the same leniency a deleted column or task
// type gets, and re-creating the goal with that id would bring them back.
#[derive(Serialize, Deserialize, MyHttpObjectStructure, Clone, Debug, PartialEq)]
pub struct GoalResponse {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub description: String,
    pub tasks_amount: i32,
    pub done_amount: i32,
    pub created_unix_seconds: i64,
}

#[derive(Serialize, Deserialize, MyHttpObjectStructure, Clone, Debug, PartialEq)]
pub struct GoalsResponse {
    pub goals: Vec<GoalResponse>,
}

#[derive(MyHttpInput)]
pub struct GetGoalsInputModel {
    #[http_body(name: "projectId", description: "Which project's goals to read")]
    pub project_id: String,
}
