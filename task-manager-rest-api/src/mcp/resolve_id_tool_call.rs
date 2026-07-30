use std::sync::Arc;

use mcp_server_middleware::*;
use serde::{Deserialize, Serialize};

use crate::app::AppContext;
use crate::board::{compose_goal_handle, compose_task_handle, parse_goal_handle, parse_task_handle};
use crate::mcp::{GoalView, TaskView};

#[derive(ApplyJsonSchema, Debug, Serialize, Deserialize)]
pub struct ResolveIdInput {
    #[property(
        description = "The task id to look up, as somebody wrote it, e.g. `RMS-42` or `RMS-000042`"
    )]
    pub id: String,
}

/// A project that once held the prefix, and what the same number means there now.
#[derive(ApplyJsonSchema, Debug, Serialize, Deserialize)]
pub struct ArchivedMatch {
    #[property(
        description = "The prefix this project carries NOW. The id you asked about was written when it carried the one you searched for"
    )]
    pub project: String,
    #[property(description = "The project's name")]
    pub project_name: String,
    #[property(
        description = "What that task is called today, under this project's current prefix — this is the id to use from now on. Absent when the project never had a task with that number"
    )]
    pub id_today: Option<String>,
    #[property(description = "The task itself, when it exists")]
    pub task: Option<TaskView>,
    #[property(
        description = "The goal that number names on that project, when the number turned out to belong to a goal rather than a task"
    )]
    pub goal: Option<GoalView>,
}

#[derive(ApplyJsonSchema, Debug, Serialize, Deserialize)]
pub struct ResolveIdResponse {
    #[property(description = "The id as it was asked about")]
    pub asked: String,
    #[property(
        description = "What this id means right now: the task on the board of whatever project currently holds the prefix. Absent when nobody holds it, or when that board has no such number"
    )]
    pub direct: Option<TaskView>,
    #[property(
        description = "Set instead of `direct` when the id names a GOAL rather than a task — either because it was written `RMS-G7`, or because the bare number turned out to be a goal's. Tasks and goals share one counter per project, so a number is one or the other and never both"
    )]
    pub direct_goal: Option<GoalView>,
    #[property(
        description = "Projects that used to hold this prefix and have since renamed away from it. Non-empty means the id is AMBIGUOUS ACROSS TIME — `direct` is what it means today, and these are what it may have meant when it was written. Say so rather than silently answering about the wrong task"
    )]
    pub archived: Vec<ArchivedMatch>,
    #[property(description = "A one-line summary of the above, safe to repeat to a person as-is")]
    pub verdict: String,
}

pub struct ResolveIdHandler {
    app: Arc<AppContext>,
}

impl ResolveIdHandler {
    pub fn new(app: Arc<AppContext>) -> Self {
        Self { app }
    }
}

impl ToolDefinition for ResolveIdHandler {
    const FUNC_NAME: &'static str = "tasks_resolve_id";
    const DESCRIPTION: &'static str = "Work out what a task id someone QUOTED actually refers to. \
Reach for it when an id comes from outside the current board — an older conversation, a commit \
message, a link a colleague pasted — rather than from a tool call you just made. A project's prefix \
can be renamed, and the freed prefix can then be taken by a different project, so the same string can \
mean one task today and another one last month. This answers both: what it means now, and which \
projects used to answer to it. If `archived` is not empty, tell the person before acting.\
\
Works for goals too, written either way: `RMS-G7` names a goal outright, and a bare `RMS-7` is answered \
with whichever of the two that number turned out to be — one counter serves both, so it is never \
ambiguous. The answer comes back in `direct` for a task and `direct_goal` for a goal.";
}

#[async_trait::async_trait]
impl McpToolCall<ResolveIdInput, ResolveIdResponse> for ResolveIdHandler {
    async fn execute_tool_call(&self, model: ResolveIdInput) -> Result<ResolveIdResponse, String> {
        // Either spelling is accepted. `RMS-G7` says goal outright; `RMS-42` says only "number 42", and
        // since one counter serves both kinds, what that number names is a lookup — which is exactly the
        // question this tool exists to answer.
        let (parsed, marked_as_goal) = match parse_goal_handle(&model.id) {
            Some(parsed) => (parsed, true),
            None => (
                parse_task_handle(&model.id).ok_or_else(|| {
                    format!(
                        "'{}' is not an id — expected something like RMS-42, or RMS-G7 for a goal",
                        model.id
                    )
                })?,
                false,
            ),
        };

        let board = self.app.board.read();

        let current_holder = board.get_project_by_prefix(&parsed.prefix);

        let direct = if marked_as_goal {
            None
        } else {
            current_holder.as_ref().and_then(|project| {
                board
                    .get_task_including_deleted(&project.id, parsed.number)
                    .map(|task| TaskView::from_model(&task, project, &board))
            })
        };

        // Looked up only when no task answers, which costs nothing: a number is a task's or a goal's, never
        // both, so this can never contradict `direct`.
        let direct_goal = if direct.is_some() {
            None
        } else {
            current_holder.as_ref().and_then(|project| {
                board
                    .get_goal_including_deleted(&project.id, parsed.number)
                    .map(|goal| GoalView::from_model(&goal, project, &board))
            })
        };

        // Every project that ever held the prefix, minus the one holding it now — that one is `direct`
        // and listing it twice would read as two different answers.
        let archived: Vec<ArchivedMatch> = board
            .projects_ever_holding_prefix(&parsed.prefix)
            .iter()
            .filter(|project| project.prefix != parsed.prefix)
            .map(|project| {
                let task = if marked_as_goal {
                    None
                } else {
                    board.get_task_including_deleted(&project.id, parsed.number)
                };

                let goal = if task.is_some() {
                    None
                } else {
                    board.get_goal_including_deleted(&project.id, parsed.number)
                };

                // Composed in the spelling of whichever kind was found, so the id handed back is one the
                // caller can use as-is.
                let id_today = match (&task, &goal) {
                    (Some(_), _) => Some(compose_task_handle(&project.prefix, parsed.number)),
                    (None, Some(_)) => Some(compose_goal_handle(&project.prefix, parsed.number)),
                    (None, None) => None,
                };

                ArchivedMatch {
                    project: project.prefix.clone(),
                    project_name: project.name.clone(),
                    id_today,
                    task: task.map(|task| TaskView::from_model(&task, project, &board)),
                    goal: goal.map(|goal| GoalView::from_model(&goal, project, &board)),
                }
            })
            .collect();

        let verdict = build_verdict(
            &model.id,
            &parsed.prefix,
            direct.is_some() || direct_goal.is_some(),
            &archived,
        );

        Ok(ResolveIdResponse {
            asked: model.id,
            direct,
            direct_goal,
            archived,
            verdict,
        })
    }
}

/// The one-line answer, so a caller does not have to reason about the combination of "found now" and
/// "found in history" to say something true.
fn build_verdict(asked: &str, prefix: &str, found_now: bool, archived: &[ArchivedMatch]) -> String {
    let live_archived: Vec<&ArchivedMatch> = archived
        .iter()
        .filter(|itm| itm.id_today.is_some())
        .collect();

    match (found_now, live_archived.is_empty()) {
        (true, true) => {
            format!("{asked} is that one, and it has never meant anything else.")
        }
        (true, false) => {
            let others: Vec<String> = live_archived
                .iter()
                .filter_map(|itm| itm.id_today.clone())
                .collect();
            format!(
                "{asked} points at that one today, but the prefix {prefix} used to belong to another project — the same id once meant {}. Check which one was intended.",
                others.join(" or ")
            )
        }
        (false, false) => {
            let others: Vec<String> = live_archived
                .iter()
                .filter_map(|itm| itm.id_today.clone())
                .collect();
            format!(
                "Nothing answers to {asked} now — no project holds the prefix {prefix}, or its board has no such number. It used to mean {}.",
                others.join(" or ")
            )
        }
        (false, true) => format!(
            "Nothing answers to {asked}, now or before. Either the id was mistyped or the task was deleted — note that goals are never deleted, so a goal id that resolves to nothing was never real."
        ),
    }
}
