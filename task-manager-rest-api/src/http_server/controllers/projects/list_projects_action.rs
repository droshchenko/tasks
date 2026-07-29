use std::sync::Arc;

use service_sdk::macros::use_my_http_server;
use task_manager_shared::projects::ProjectsResponse;

use crate::app::AppContext;
use crate::mappers::project_to_response;

use_my_http_server!();

#[http_route(
    method: "GET",
    route: "/api/projects/v1",
    controller: "Projects",
    summary: "The projects I may see",
    description: "Feeds the project dropdown on Home and the list in Projects setup. Membership decides what comes back; an admin sees every project without being a member of any. Someone who is a member of nothing gets an empty list rather than a 403 — the UI shows them a note to ask an admin.",
    result: [
        {status_code: 200, description: "The visible projects", model: "ProjectsResponse"},
        {status_code: 401, description: "Not authenticated"},
    ]
)]
pub struct ListProjectsAction {
    app: Arc<AppContext>,
}

impl ListProjectsAction {
    pub fn new(app: Arc<AppContext>) -> Self {
        Self { app }
    }
}

async fn handle_request(
    action: &ListProjectsAction,
    ctx: &HttpContext,
) -> Result<HttpOkResult, HttpFailResult> {
    let user = crate::auth::resolve_auth_user(&action.app, ctx).await?;

    let board = action.app.board.read();

    let projects = board
        .projects_visible_to(&user.email, user.is_admin)
        .iter()
        .map(|project| project_to_response(project, board.tasks_amount(&project.id)))
        .collect();

    HttpOutput::as_json(ProjectsResponse { projects }).into_ok_result(true)
}
