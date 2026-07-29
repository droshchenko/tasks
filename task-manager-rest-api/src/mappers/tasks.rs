use task_manager_shared::tasks::{TaskCommentResponse, TaskResponse};

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
            goal_id: src.goal_id.clone(),
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
            goal_id: src.goal_id.clone(),
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

    TaskResponse {
        id: compose_task_handle(&project.prefix, task.number),
        project_id: task.project_id.clone(),
        text: task.text.clone(),
        status: project.effective_status(&task.status),
        kind: project.effective_kind(task.kind.as_deref()),
        // Resolved, so a task pointing at a goal that is gone reads as standalone rather than dangling.
        goal_id: goal.as_ref().map(|itm| itm.id.clone()),
        goal_name: goal.as_ref().map(|itm| itm.name.clone()),
        assignee: task.assignee.clone(),
        assignee_name,
        labels: task.labels.clone(),
        depends_on: task
            .depends_on
            .iter()
            .map(|number| compose_task_handle(&project.prefix, *number))
            .collect(),
        blocks: board
            .blocks(&task.project_id, task.number)
            .iter()
            .map(|number| compose_task_handle(&project.prefix, *number))
            .collect(),
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
