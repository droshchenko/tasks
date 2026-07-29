use std::sync::Arc;

use mcp_server_middleware::*;
use serde::{Deserialize, Serialize};

use crate::app::AppContext;
use crate::board::{compose_task_handle, parse_task_handle};
use crate::mcp::TaskView;

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
projects used to answer to it. If `archived` is not empty, tell the person before acting.";
}

#[async_trait::async_trait]
impl McpToolCall<ResolveIdInput, ResolveIdResponse> for ResolveIdHandler {
    async fn execute_tool_call(&self, model: ResolveIdInput) -> Result<ResolveIdResponse, String> {
        let parsed = parse_task_handle(&model.id).ok_or_else(|| {
            format!(
                "'{}' is not a task id — expected something like RMS-42",
                model.id
            )
        })?;

        let board = self.app.board.read();

        let current_holder = board.get_project_by_prefix(&parsed.prefix);

        let direct = current_holder.as_ref().and_then(|project| {
            board
                .get_task(&project.id, parsed.number)
                .map(|task| TaskView::from_model(&task, project, &board))
        });

        // Every project that ever held the prefix, minus the one holding it now — that one is `direct`
        // and listing it twice would read as two different answers.
        let archived: Vec<ArchivedMatch> = board
            .projects_ever_holding_prefix(&parsed.prefix)
            .iter()
            .filter(|project| project.prefix != parsed.prefix)
            .map(|project| {
                let task = board.get_task(&project.id, parsed.number);

                ArchivedMatch {
                    project: project.prefix.clone(),
                    project_name: project.name.clone(),
                    id_today: task
                        .as_ref()
                        .map(|_| compose_task_handle(&project.prefix, parsed.number)),
                    task: task.map(|task| TaskView::from_model(&task, project, &board)),
                }
            })
            .collect();

        let verdict = build_verdict(&model.id, &parsed.prefix, direct.is_some(), &archived);

        Ok(ResolveIdResponse {
            asked: model.id,
            direct,
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
        (true, true) => format!("{asked} is that task, and it has never meant anything else."),
        (true, false) => {
            let others: Vec<String> = live_archived
                .iter()
                .filter_map(|itm| itm.id_today.clone())
                .collect();
            format!(
                "{asked} points at that task today, but the prefix {prefix} used to belong to another project — the same id once meant {}. Check which one was intended.",
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
            "Nothing answers to {asked}, now or before. Either the id was mistyped or the task was deleted."
        ),
    }
}
