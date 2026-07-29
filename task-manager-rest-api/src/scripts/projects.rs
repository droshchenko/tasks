use std::collections::BTreeSet;

use rust_extensions::AsStr;
use rust_extensions::date_time::DateTimeAsMicroseconds;
use service_sdk::my_telemetry::MyTelemetryContext;
use task_manager_shared::kind_color::KindColor;
use task_manager_shared::projects::is_anchor_column;

use crate::app::AppContext;
use crate::board::{ColumnModel, KindModel, ProjectModel, is_valid_prefix};
use crate::postgres::ProjectDto;

fn validate_prefix(prefix: &str) -> Result<String, String> {
    let prefix = prefix.trim().to_uppercase();

    if !is_valid_prefix(&prefix) {
        return Err(format!(
            "'{prefix}' is not a usable prefix: use 1-16 ASCII letters, digits or underscores. A '-' is not allowed, because it separates the prefix from the number in a task id"
        ));
    }

    Ok(prefix)
}

/// An id typed in by a person for a column or a kind. Immutable once created, so it is worth being
/// strict here rather than living with a typo forever.
fn validate_config_id(id: &str, what: &str) -> Result<String, String> {
    let id = id.trim().to_lowercase();

    if id.is_empty() {
        return Err(format!("a {what} needs an id"));
    }

    if id.len() > 32 {
        return Err(format!("a {what} id must be 32 characters or fewer"));
    }

    if !id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(format!(
            "'{id}' is not a usable {what} id: use ASCII letters, digits, '-' or '_'"
        ));
    }

    Ok(id)
}

async fn save(app: &AppContext, project: ProjectModel) {
    let ctx = MyTelemetryContext::create_empty();
    let dto: ProjectDto = (&project).into();
    app.projects_repo.upsert(&dto, &ctx).await;

    let project_id = project.id.clone();
    app.board.upsert_project(project);
    app.subscribers.notify_project_changed(&project_id).await;
}

/// Create a project. Returns its internal id — the UI works in ids, MCP in prefixes.
pub async fn create_project(
    app: &AppContext,
    name: &str,
    description: &str,
    prefix: &str,
) -> Result<String, String> {
    if name.trim().is_empty() {
        return Err("a project needs a name".to_string());
    }

    let prefix = validate_prefix(prefix)?;

    // Free means "nobody holds it right now". A prefix that only appears in some project's history is
    // takeable — which is exactly why a task's handle is composed on read rather than stored.
    if !app.board.read().is_prefix_free(&prefix, None) {
        return Err(format!(
            "prefix '{prefix}' is already used by another project"
        ));
    }

    let project = ProjectModel {
        id: rust_extensions::SortableId::generate().to_string(),
        name: name.trim().to_string(),
        description: description.trim().to_string(),
        prefix,
        prefix_history: Vec::new(),
        columns: Vec::new(),
        kinds: Vec::new(),
        members: BTreeSet::new(),
        last_task_number: 0,
        created: DateTimeAsMicroseconds::now(),
    };

    let id = project.id.clone();
    save(app, project).await;

    Ok(id)
}

/// Rename a project, and optionally move it to another prefix.
pub async fn update_project(
    app: &AppContext,
    project_id: &str,
    name: &str,
    description: &str,
    prefix: &str,
) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err("a project needs a name".to_string());
    }

    let prefix = validate_prefix(prefix)?;

    let board = app.board.read();
    let mut project = load(&board, project_id)?;

    if prefix != project.prefix {
        if !board.is_prefix_free(&prefix, Some(project_id)) {
            return Err(format!(
                "prefix '{prefix}' is already used by another project"
            ));
        }

        // The prefix being left behind goes into history, so `tasks_resolve_id` can still say what an
        // old handle used to mean once somebody else takes it over.
        if !project.prefix_history.contains(&project.prefix) {
            project.prefix_history.push(project.prefix.clone());
        }

        project.prefix = prefix;
    }

    project.name = name.trim().to_string();
    project.description = description.trim().to_string();

    save(app, project).await;
    Ok(())
}

fn load(board: &crate::board::BoardInner, project_id: &str) -> Result<ProjectModel, String> {
    board
        .get_project(project_id)
        .map(|itm| itm.as_ref().clone())
        .ok_or_else(|| format!("no project with id '{project_id}'"))
}

/// Add a column between Todo and Done.
pub async fn add_column(
    app: &AppContext,
    project_id: &str,
    id: &str,
    name: &str,
    description: &str,
    order: i32,
) -> Result<(), String> {
    let id = validate_config_id(id, "column")?;

    if is_anchor_column(&id) {
        return Err(format!(
            "'{id}' is one of the two columns every project already has, and cannot be added"
        ));
    }

    let board = app.board.read();
    let mut project = load(&board, project_id)?;

    if project.columns.iter().any(|itm| itm.id == id) {
        return Err(format!("{} already has a column '{id}'", project.prefix));
    }

    project.columns.push(ColumnModel {
        id,
        name: name.trim().to_string(),
        description: description.trim().to_string(),
        order,
    });
    project.columns.sort_by_key(|itm| itm.order);

    save(app, project).await;
    Ok(())
}

/// Rename a column, or move it. The id is not touched — a column id is immutable, because tasks point
/// at it as their status and there is no rename that would not have to rewrite them.
pub async fn update_column(
    app: &AppContext,
    project_id: &str,
    column_id: &str,
    name: &str,
    description: &str,
    order: i32,
) -> Result<(), String> {
    let board = app.board.read();
    let mut project = load(&board, project_id)?;

    let column = project
        .columns
        .iter_mut()
        .find(|itm| itm.id == column_id)
        .ok_or_else(|| format!("{} has no column '{column_id}'", project.prefix))?;

    column.name = name.trim().to_string();
    column.description = description.trim().to_string();
    column.order = order;

    project.columns.sort_by_key(|itm| itm.order);

    save(app, project).await;
    Ok(())
}

/// Remove a column.
///
/// Its tasks are deliberately left alone: their stored status still names this column, and they read
/// as Todo from now on. Re-creating a column with the same id brings them straight back to it.
pub async fn delete_column(
    app: &AppContext,
    project_id: &str,
    column_id: &str,
) -> Result<(), String> {
    let board = app.board.read();
    let mut project = load(&board, project_id)?;

    let before = project.columns.len();
    project.columns.retain(|itm| itm.id != column_id);

    if project.columns.len() == before {
        return Err(format!("{} has no column '{column_id}'", project.prefix));
    }

    save(app, project).await;
    Ok(())
}

pub async fn add_kind(
    app: &AppContext,
    project_id: &str,
    id: &str,
    name: &str,
    description: &str,
    color: &str,
) -> Result<(), String> {
    let id = validate_config_id(id, "kind")?;
    let color = parse_color(color)?;

    let board = app.board.read();
    let mut project = load(&board, project_id)?;

    if project.kinds.iter().any(|itm| itm.id == id) {
        return Err(format!("{} already has a kind '{id}'", project.prefix));
    }

    project.kinds.push(KindModel {
        id,
        name: name.trim().to_string(),
        description: description.trim().to_string(),
        color,
    });

    save(app, project).await;
    Ok(())
}

pub async fn update_kind(
    app: &AppContext,
    project_id: &str,
    kind_id: &str,
    name: &str,
    description: &str,
    color: &str,
) -> Result<(), String> {
    let color = parse_color(color)?;

    let board = app.board.read();
    let mut project = load(&board, project_id)?;

    let kind = project
        .kinds
        .iter_mut()
        .find(|itm| itm.id == kind_id)
        .ok_or_else(|| format!("{} has no kind '{kind_id}'", project.prefix))?;

    kind.name = name.trim().to_string();
    kind.description = description.trim().to_string();
    kind.color = color;

    save(app, project).await;
    Ok(())
}

/// Remove a kind. Tasks carrying it read as having no kind — the same leniency as a deleted column.
pub async fn delete_kind(app: &AppContext, project_id: &str, kind_id: &str) -> Result<(), String> {
    let board = app.board.read();
    let mut project = load(&board, project_id)?;

    let before = project.kinds.len();
    project.kinds.retain(|itm| itm.id != kind_id);

    if project.kinds.len() == before {
        return Err(format!("{} has no kind '{kind_id}'", project.prefix));
    }

    save(app, project).await;
    Ok(())
}

/// Unlike a column or a kind id, a colour is validated strictly: it is chosen from a fixed palette in
/// the UI, so anything else is a caller bug rather than an old value to be lenient about.
fn parse_color(color: &str) -> Result<KindColor, String> {
    color
        .trim()
        .to_lowercase()
        .parse::<KindColor>()
        .map_err(|_| {
            format!(
                "'{color}' is not one of the palette colours: {}",
                KindColor::ALL
                    .iter()
                    .map(|itm| itm.as_str())
                    .collect::<Vec<&str>>()
                    .join(", ")
            )
        })
}

/// Replace a project's membership wholesale.
///
/// Wholesale rather than add/remove because that is how the screen works — a list of checkboxes saved
/// as a set — and because it means the caller never has to know the current state to change it.
pub async fn set_members(
    app: &AppContext,
    project_id: &str,
    emails: &[String],
) -> Result<(), String> {
    let board = app.board.read();
    let mut project = load(&board, project_id)?;

    let members: BTreeSet<String> = emails
        .iter()
        .map(|itm| itm.trim().to_lowercase())
        .filter(|itm| !itm.is_empty())
        .collect();

    let ctx = MyTelemetryContext::create_empty();
    let as_vec: Vec<String> = members.iter().cloned().collect();
    app.project_members_repo
        .replace_for_project(project_id, &as_vec, &ctx)
        .await;

    project.members = members;

    let project_id = project.id.clone();
    app.board.upsert_project(project);
    app.subscribers.notify_project_changed(&project_id).await;

    Ok(())
}
