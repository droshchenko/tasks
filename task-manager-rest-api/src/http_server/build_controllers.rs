use std::sync::Arc;

use service_sdk::HttpServerBuilder;

use crate::app::AppContext;

/// Register everything the browser talks to.
///
/// There is deliberately no task-mutating action here: every change to a task arrives through `/mcp`.
/// If you find yourself adding `register_post_action` under `tasks`, the design changed — update
/// README.md first.
pub fn build_controllers(app: &Arc<AppContext>, http_server_builder: &mut HttpServerBuilder) {
    use super::controllers::{auth, projects, system, tasks, users};

    http_server_builder.register_get_action(system::PingAction::new(app.clone()));
    http_server_builder.register_get_action(system::DiagnosticsAction::new(app.clone()));

    http_server_builder.register_get_action(auth::GoogleAuthUrlAction::new(app.clone()));
    http_server_builder.register_get_action(auth::GoogleCallbackAction::new(app.clone()));
    http_server_builder.register_get_action(auth::MeAction::new(app.clone()));
    http_server_builder.register_post_action(auth::LogoutAction::new(app.clone()));

    http_server_builder.register_get_action(projects::ListProjectsAction::new(app.clone()));
    http_server_builder.register_post_action(projects::CreateProjectAction::new(app.clone()));
    http_server_builder.register_post_action(projects::UpdateProjectAction::new(app.clone()));
    http_server_builder.register_post_action(projects::AddColumnAction::new(app.clone()));
    http_server_builder.register_post_action(projects::UpdateColumnAction::new(app.clone()));
    http_server_builder.register_post_action(projects::DeleteColumnAction::new(app.clone()));
    http_server_builder.register_post_action(projects::AddKindAction::new(app.clone()));
    http_server_builder.register_post_action(projects::UpdateKindAction::new(app.clone()));
    http_server_builder.register_post_action(projects::DeleteKindAction::new(app.clone()));
    http_server_builder.register_post_action(projects::SetMembersAction::new(app.clone()));

    http_server_builder.register_get_action(tasks::ListTasksAction::new(app.clone()));

    http_server_builder.register_get_action(users::ListUsersAction::new(app.clone()));
    http_server_builder.register_post_action(users::CreateUserAction::new(app.clone()));
    http_server_builder.register_put_action(users::UpdateUserAction::new(app.clone()));
}
