use task_manager_shared::tasks::{TaskCommentResponse, TaskLinkResponse, TaskResponse};

use crate::board::{
    BoardInner, CommentModel, ProjectModel, TaskModel, compose_task_handle, parse_task_handle,
};
use crate::postgres::{TaskCommentJsonModel, TaskDto};

impl From<&TaskCommentJsonModel> for CommentModel {
    fn from(src: &TaskCommentJsonModel) -> Self {
        Self {
            moment: rust_extensions::date_time::DateTimeAsMicroseconds::new(
                src.moment_unix_seconds * 1_000_000,
            ),
            who: src.who.clone(),
            text: src.text.clone(),
        }
    }
}

impl From<&CommentModel> for TaskCommentJsonModel {
    fn from(src: &CommentModel) -> Self {
        Self {
            moment_unix_seconds: src.moment.unix_microseconds / 1_000_000,
            who: src.who.clone(),
            text: src.text.clone(),
        }
    }
}

impl From<&TaskDto> for TaskModel {
    fn from(src: &TaskDto) -> Self {
        Self {
            project_id: src.project_id.clone(),
            number: src.number,
            text: src.task_text.clone(),
            status: src.status.clone(),
            kind: src.kind.clone(),
            goal_number: src.goal_number,
            assignee: src.assignee.clone(),
            labels: src.labels.clone(),
            depends_on: src.depends_on.clone(),
            comments: src.comments.iter().map(|itm| itm.into()).collect(),
            created: src.created,
            updated: src.updated,
            close_moment: src.close_moment,
        }
    }
}

impl From<&TaskModel> for TaskDto {
    fn from(src: &TaskModel) -> Self {
        Self {
            project_id: src.project_id.clone(),
            number: src.number,
            task_text: src.text.clone(),
            status: src.status.clone(),
            kind: src.kind.clone(),
            goal_number: src.goal_number,
            assignee: src.assignee.clone(),
            labels: src.labels.clone(),
            depends_on: src.depends_on.clone(),
            comments: src.comments.iter().map(|itm| itm.into()).collect(),
            created: src.created,
            updated: src.updated,
            close_moment: src.close_moment,
        }
    }
}

/// Memory -> wire, resolving everything a reader should not have to work out for itself.
///
/// Four things happen here that cannot happen in a plain `From`, because all four need the rest of
/// the board:
///
/// * the handle is **composed** from the project's current prefix — it is not stored;
/// * `depends_on` numbers become handles, so a client never sees an internal number;
/// * `blocked` / `blocks` are derived against the current statuses of every other task;
/// * an unknown status reads as Todo and an unknown kind as no kind, so a deleted column or kind
///   renders instead of breaking the read.
pub fn task_to_response(
    task: &TaskModel,
    project: &ProjectModel,
    board: &BoardInner,
) -> TaskResponse {
    let goal = board.effective_goal(task);

    let assignee_name = task
        .assignee
        .as_ref()
        .and_then(|itm| board.display_name_of(itm));

    // Read once: it is a scan of the project's tasks, and both `blocks` and the statuses beside it need
    // the same answer.
    let blocks = board.blocks(&task.project_id, task.number);

    TaskResponse {
        id: compose_task_handle(&project.prefix, task.number),
        project_id: task.project_id.clone(),
        text: task.text.clone(),
        status: project.effective_status(&task.status),
        kind: project.effective_kind(task.kind.as_deref()),
        // Resolved and composed, so a task whose goal number names nothing reads as standalone rather
        // than as a dangling number nobody can look up.
        goal: goal
            .as_ref()
            .map(|itm| crate::board::compose_goal_handle(&project.prefix, itm.number)),
        goal_name: goal.as_ref().map(|itm| itm.name.clone()),
        goal_color: goal
            .as_ref()
            .map(|itm| rust_extensions::AsStr::as_str(&itm.color).to_string()),
        assignee: task.assignee.clone(),
        assignee_name,
        labels: task.labels.clone(),
        depends_on: task
            .depends_on
            .iter()
            .map(|number| compose_task_handle(&project.prefix, *number))
            .collect(),
        blocks: blocks
            .iter()
            .map(|number| compose_task_handle(&project.prefix, *number))
            .collect(),
        link_statuses: link_statuses(task, &blocks, project, board),
        blocked: board.is_blocked(task),
        comments: task
            .comments
            .iter()
            .map(|itm| TaskCommentResponse {
                moment_unix_seconds: itm.moment.unix_microseconds / 1_000_000,
                who: itm.who.clone(),
                text: itm.text.clone(),
            })
            .collect(),
        created_unix_seconds: task.created.unix_microseconds / 1_000_000,
        updated_unix_seconds: task.updated.unix_microseconds / 1_000_000,
        closed_unix_seconds: task
            .close_moment
            .map(|itm| itm.unix_microseconds / 1_000_000),
    }
}

/// What every task on either end of a dependency is doing.
///
/// Both directions in one list rather than two: the reader asks the same question of a blocker and of
/// something blocked — is it done — and an id appears in only one of the two lists anyway, because a task
/// that both waited on and blocked the same task would be a cycle.
///
/// Dependencies never cross projects, which is what makes the lookup a project-local one and this a scan of
/// tasks already in memory rather than a query.
fn link_statuses(
    task: &TaskModel,
    blocks: &[i64],
    project: &ProjectModel,
    board: &BoardInner,
) -> Vec<TaskLinkResponse> {
    task.depends_on
        .iter()
        .chain(blocks.iter())
        .filter_map(|number| {
            // No entry for a number that names no task. It is the case that keeps `blocked` true — a typo
            // or a deleted blocker — and inventing a status for it would hide exactly that.
            let linked = board.get_task(&task.project_id, *number)?;

            Some(TaskLinkResponse {
                id: compose_task_handle(&project.prefix, *number),
                status: project.effective_status(&linked.status),
            })
        })
        .collect()
}

/// Read a `depends_on` entry written by a caller: a handle (`RMS-7`) or a bare number (`7`).
///
/// Bare numbers are accepted because within one project they are unambiguous and a caller editing a
/// dependency list should not have to retype the prefix. A handle whose prefix belongs to a
/// *different* project is refused rather than silently reinterpreted — dependencies do not cross
/// projects, and quietly treating `OTHER-7` as "task 7 here" would create one that looks fine.
pub fn parse_dependency(src: &str, project: &ProjectModel) -> Result<i64, String> {
    let src = src.trim();

    if let Ok(number) = src.parse::<i64>() {
        if number > 0 {
            return Ok(number);
        }
        return Err(format!("'{src}' is not a task number"));
    }

    let parsed = parse_task_handle(src)
        .ok_or_else(|| format!("'{src}' is not a task id — expected something like RMS-42"))?;

    if parsed.prefix != project.prefix {
        return Err(format!(
            "'{src}' belongs to another project; a task can only depend on tasks of {}",
            project.prefix
        ));
    }

    Ok(parsed.number)
}
