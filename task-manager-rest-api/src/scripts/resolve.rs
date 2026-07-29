use std::sync::Arc;

use crate::board::{BoardInner, ProjectModel, TaskModel, parse_task_handle};

/// A task and the project it belongs to, resolved together.
///
/// Named rather than a tuple because almost every caller needs both halves and would otherwise have
/// to remember which came first.
pub struct ResolvedTask {
    pub project: Arc<ProjectModel>,
    pub task: Arc<TaskModel>,
}

/// The project a prefix names right now.
///
/// Errors are text and say what would fix them — an id that is not on the board must come back as a
/// message, never as an empty result that reads like "there is nothing there".
pub fn resolve_project_by_prefix(
    board: &BoardInner,
    prefix: &str,
) -> Result<Arc<ProjectModel>, String> {
    let wanted = prefix.trim().to_uppercase();

    if wanted.is_empty() {
        return Err("no project given — pass a project prefix such as RMS".to_string());
    }

    board.get_project_by_prefix(&wanted).ok_or_else(|| {
        // Owned rather than borrowed: `projects()` hands back an Arc that would not outlive this
        // closure. Listing the real prefixes is the whole value of the message — "no such project" on
        // its own leaves the caller guessing whether it mistyped or the board is empty.
        let known: Vec<String> = board
            .projects()
            .iter()
            .map(|itm| itm.prefix.clone())
            .collect();

        if known.is_empty() {
            format!("no project with prefix '{wanted}'; there are no projects yet")
        } else {
            format!(
                "no project with prefix '{wanted}'; known prefixes: {}",
                known.join(", ")
            )
        }
    })
}

/// The task a handle names right now — `RMS-42`, or `RMS-000042`.
///
/// Resolves against the **current** holder of the prefix. A handle whose prefix has since moved to
/// another project therefore lands on that other project, which is correct for a live tool call and
/// misleading for a handle quoted from an old conversation; `tasks_resolve_id` exists to tell the two
/// apart.
pub fn resolve_task(board: &BoardInner, handle: &str) -> Result<ResolvedTask, String> {
    let parsed = parse_task_handle(handle)
        .ok_or_else(|| format!("'{handle}' is not a task id — expected something like RMS-42"))?;

    let project = resolve_project_by_prefix(board, &parsed.prefix)?;

    let task = board
        .get_task(&project.id, parsed.number)
        .ok_or_else(|| format!("no task {handle} on the {} board", project.prefix))?;

    Ok(ResolvedTask { project, task })
}
