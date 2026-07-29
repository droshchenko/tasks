use std::collections::BTreeSet;

use ahash::AHashMap;
use service_sdk::my_telemetry::MyTelemetryContext;

use crate::app::AppContext;
use crate::board::{BoardInner, ColumnTemplateModel, ProjectModel, TaskModel, UserModel};

/// Read the whole product out of Postgres and install it in memory.
///
/// Called once, before the HTTP server starts serving. Everything after this point reads from memory;
/// Postgres is only ever written again. If it fails, it panics through the repos' `.expect` — which is
/// correct: a service that cannot read its own state has nothing useful to serve, and the SDK turns
/// the panic into a FatalError rather than leaving an empty board looking like a fresh install.
pub async fn load_state(app: &AppContext) {
    let ctx = MyTelemetryContext::create_empty();

    let project_rows = app.projects_repo.get_all(&ctx).await;
    let member_rows = app.project_members_repo.get_all(&ctx).await;
    let task_rows = app.tasks_repo.get_all(&ctx).await;
    let user_rows = app.users_repo.get_all(&ctx).await;
    let template_rows = app.column_templates_repo.get_all(&ctx).await;

    // Membership lives in its own table, so it is folded back onto the projects here — the only place
    // the two halves are joined.
    let mut members_by_project: AHashMap<String, BTreeSet<String>> = AHashMap::new();
    for row in &member_rows {
        members_by_project
            .entry(row.project_id.clone())
            .or_default()
            .insert(row.email.trim().to_lowercase());
    }

    let projects: Vec<ProjectModel> = project_rows
        .iter()
        .map(|row| {
            let mut project: ProjectModel = row.into();

            if let Some(members) = members_by_project.remove(&project.id) {
                project.members = members;
            }

            project
        })
        .collect();

    let tasks: Vec<TaskModel> = task_rows.iter().map(|row| row.into()).collect();
    let users: Vec<UserModel> = user_rows.iter().map(|row| row.into()).collect();

    let column_templates: Vec<ColumnTemplateModel> =
        template_rows.iter().map(|row| row.into()).collect();

    app.board.replace_all(BoardInner::from_loaded(
        projects,
        tasks,
        users,
        column_templates,
    ));
}
