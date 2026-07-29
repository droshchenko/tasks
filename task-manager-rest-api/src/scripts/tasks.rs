use rust_extensions::date_time::DateTimeAsMicroseconds;
use service_sdk::my_telemetry::MyTelemetryContext;
use task_manager_shared::projects::COLUMN_ID_TODO;

use crate::app::AppContext;
use crate::board::{ProjectModel, TaskModel};
use crate::mappers::parse_dependency;
use crate::postgres::TaskDto;

use super::resolve::{resolve_project_by_prefix, resolve_task};

/// What a caller wants to change about a task. Every field is optional and `None` means "leave it".
///
/// The distinction that matters is between `None` and an *empty* value: `assignee: Some("")` clears
/// the assignee, `depends_on: Some(vec![])` clears the dependencies. Without that, there would be no
/// way to express "remove this" at all.
#[derive(Default)]
pub struct TaskPatch {
    pub text: Option<String>,
    pub status: Option<String>,
    pub kind: Option<String>,
    pub assignee: Option<String>,
    pub add_labels: Vec<String>,
    pub remove_labels: Vec<String>,
    pub depends_on: Option<Vec<String>>,
}

impl TaskPatch {
    fn is_empty(&self) -> bool {
        self.text.is_none()
            && self.status.is_none()
            && self.kind.is_none()
            && self.assignee.is_none()
            && self.add_labels.is_empty()
            && self.remove_labels.is_empty()
            && self.depends_on.is_none()
    }
}

/// Lower-case, trim, drop blanks, de-duplicate, sort.
///
/// Sorting is not cosmetic: it makes a task's label list identical whichever order the tags arrived
/// in, so two boards built by different routes compare equal.
fn normalise_labels(src: &[String]) -> Vec<String> {
    let mut labels: Vec<String> = src
        .iter()
        .map(|itm| itm.trim().to_lowercase())
        .filter(|itm| !itm.is_empty())
        .collect();

    labels.sort();
    labels.dedup();
    labels
}

fn validate_status(project: &ProjectModel, status: &str) -> Result<String, String> {
    let status = status.trim().to_lowercase();

    if project.has_column(&status) {
        return Ok(status);
    }

    Err(format!(
        "'{status}' is not a column of {}; it has: {}",
        project.prefix,
        available_columns(project)
    ))
}

fn available_columns(project: &ProjectModel) -> String {
    let mut ids = vec![COLUMN_ID_TODO.to_string()];
    ids.extend(project.columns.iter().map(|itm| itm.id.clone()));
    ids.push(task_manager_shared::projects::COLUMN_ID_DONE.to_string());
    ids.join(", ")
}

fn validate_kind(project: &ProjectModel, kind: &str) -> Result<Option<String>, String> {
    let kind = kind.trim().to_lowercase();

    // An empty kind is how a caller says "no kind", which is legal — kinds are optional.
    if kind.is_empty() {
        return Ok(None);
    }

    if project.has_kind(&kind) {
        return Ok(Some(kind));
    }

    let known: Vec<&str> = project.kinds.iter().map(|itm| itm.id.as_str()).collect();

    Err(if known.is_empty() {
        format!("{} has no kinds configured", project.prefix)
    } else {
        format!(
            "'{kind}' is not a kind of {}; it has: {}",
            project.prefix,
            known.join(", ")
        )
    })
}

fn parse_dependencies(src: &[String], project: &ProjectModel) -> Result<Vec<i64>, String> {
    let mut numbers = Vec::with_capacity(src.len());

    for entry in src {
        numbers.push(parse_dependency(entry, project)?);
    }

    numbers.sort_unstable();
    numbers.dedup();
    Ok(numbers)
}

/// Everything a caller passes to create a task. A struct rather than eight positional arguments —
/// three of them are `Option<String>` and would be trivially swappable.
pub struct NewTask {
    pub project_prefix: String,
    pub text: String,
    pub status: Option<String>,
    pub kind: Option<String>,
    pub assignee: Option<String>,
    pub labels: Vec<String>,
    pub depends_on: Vec<String>,
}

/// Put a new task on a board. Returns its handle.
pub async fn create_task(app: &AppContext, new_task: NewTask) -> Result<String, String> {
    let board = app.board.read();
    let project = resolve_project_by_prefix(&board, &new_task.project_prefix)?;

    if new_task.text.trim().is_empty() {
        return Err("a task needs some text".to_string());
    }

    // Everything is validated before a number is reserved, so a rejected call leaves the counter
    // exactly where it was and does not burn an id.
    let status = match &new_task.status {
        None => COLUMN_ID_TODO.to_string(),
        Some(status) => validate_status(&project, status)?,
    };

    let kind = match &new_task.kind {
        None => None,
        Some(kind) => validate_kind(&project, kind)?,
    };

    let depends_on = parse_dependencies(&new_task.depends_on, &project)?;
    let labels = normalise_labels(&new_task.labels);

    let number = app.board.reserve_task_number(&project.id).ok_or_else(|| {
        format!(
            "project {} vanished while creating the task",
            project.prefix
        )
    })?;

    let now = DateTimeAsMicroseconds::now();
    let task = TaskModel {
        project_id: project.id.clone(),
        number,
        text: new_task.text.trim().to_string(),
        status,
        kind,
        assignee: normalise_assignee(new_task.assignee.as_deref()),
        labels,
        depends_on,
        comments: Vec::new(),
        created: now,
        updated: now,
    };

    let ctx = MyTelemetryContext::create_empty();
    let dto: TaskDto = (&task).into();
    app.tasks_repo.upsert(&dto, &ctx).await;

    // The counter moved in memory when the number was reserved; persist it so a restart does not
    // hand the same number out again.
    persist_project_counter(app, &project.id, &ctx).await;

    app.board.upsert_task(task);
    app.subscribers.notify_project_changed(&project.id).await;

    Ok(crate::board::compose_task_handle(&project.prefix, number))
}

/// An assignee is stored lower-cased, and blank means nobody. `claude` survives this untouched, which
/// is the point — it is a real assignee value, just not a person.
fn normalise_assignee(src: Option<&str>) -> Option<String> {
    let src = src?.trim().to_lowercase();

    if src.is_empty() { None } else { Some(src) }
}

/// Write the project row purely to save its task counter.
///
/// Reads the project back out of memory first: the copy captured before `reserve_task_number` still
/// has the old counter, and persisting that would undo the reservation.
async fn persist_project_counter(app: &AppContext, project_id: &str, ctx: &MyTelemetryContext) {
    if let Some(project) = app.board.read().get_project(project_id) {
        let dto: crate::postgres::ProjectDto = project.as_ref().into();
        app.projects_repo.upsert(&dto, ctx).await;
    }
}

/// Change a task. Returns its handle.
pub async fn update_task(
    app: &AppContext,
    handle: &str,
    patch: TaskPatch,
) -> Result<String, String> {
    if patch.is_empty() {
        return Err(
            "nothing to update: pass at least one of text, status, kind, assignee, labels or dependencies"
                .to_string(),
        );
    }

    let board = app.board.read();
    let resolved = resolve_task(&board, handle)?;
    let project = resolved.project;
    let mut task = resolved.task.as_ref().clone();

    if let Some(text) = &patch.text {
        if text.trim().is_empty() {
            return Err("a task needs some text".to_string());
        }
        task.text = text.trim().to_string();
    }

    if let Some(status) = &patch.status {
        task.status = validate_status(&project, status)?;
    }

    if let Some(kind) = &patch.kind {
        task.kind = validate_kind(&project, kind)?;
    }

    if let Some(assignee) = &patch.assignee {
        task.assignee = normalise_assignee(Some(assignee));
    }

    if !patch.add_labels.is_empty() || !patch.remove_labels.is_empty() {
        let mut labels = task.labels.clone();
        labels.extend(normalise_labels(&patch.add_labels));

        let removing = normalise_labels(&patch.remove_labels);
        // Removal after addition, so a label passed to both ends up removed. Arbitrary, but it has to
        // be one or the other and this is the order a reader assumes.
        labels.retain(|itm| !removing.contains(itm));

        task.labels = normalise_labels(&labels);
    }

    if let Some(depends_on) = &patch.depends_on {
        task.depends_on = parse_dependencies(depends_on, &project)?;
    }

    task.updated = DateTimeAsMicroseconds::now();

    let ctx = MyTelemetryContext::create_empty();
    let dto: TaskDto = (&task).into();
    app.tasks_repo.upsert(&dto, &ctx).await;

    let handle = crate::board::compose_task_handle(&project.prefix, task.number);
    app.board.upsert_task(task);
    app.subscribers.notify_project_changed(&project.id).await;

    Ok(handle)
}

/// Take a task off a board for good.
pub async fn delete_task(app: &AppContext, handle: &str) -> Result<String, String> {
    let board = app.board.read();
    let resolved = resolve_task(&board, handle)?;
    let project_id = resolved.project.id.clone();
    let number = resolved.task.number;

    let ctx = MyTelemetryContext::create_empty();
    app.tasks_repo.delete(&project_id, number, &ctx).await;

    app.board.remove_task(&project_id, number);
    app.subscribers.notify_project_changed(&project_id).await;

    Ok(crate::board::compose_task_handle(
        &resolved.project.prefix,
        number,
    ))
}

/// Append a comment to a task's thread.
///
/// Does **not** move the task's `updated`: the thread is a record of the conversation about the work,
/// not a change to the work.
pub async fn add_comment(
    app: &AppContext,
    handle: &str,
    who: &str,
    text: &str,
) -> Result<String, String> {
    if who.trim().is_empty() {
        return Err("a comment needs an author — an email, or `claude`".to_string());
    }

    if text.trim().is_empty() {
        return Err("a comment needs some text".to_string());
    }

    let board = app.board.read();
    let resolved = resolve_task(&board, handle)?;
    let project = resolved.project;
    let mut task = resolved.task.as_ref().clone();

    task.comments.push(crate::board::CommentModel {
        moment: DateTimeAsMicroseconds::now(),
        who: who.trim().to_lowercase(),
        text: text.trim().to_string(),
    });

    let ctx = MyTelemetryContext::create_empty();
    let dto: TaskDto = (&task).into();
    app.tasks_repo.upsert(&dto, &ctx).await;

    let handle = crate::board::compose_task_handle(&project.prefix, task.number);
    app.board.upsert_task(task);
    app.subscribers.notify_project_changed(&project.id).await;

    Ok(handle)
}
