use std::sync::Arc;

use service_sdk::macros::use_my_http_server;
use task_manager_shared::projects::UpdateProjectKindInputModel;

use crate::app::AppContext;
use crate::http_server::errors::bad_request;

use_my_http_server!();

#[http_route(
    method: "PUT",
    route: "/api/projects/v1/{projectId}/kinds/{kindId}",
    controller: "Projects",
    summary: "Rename a kind or recolour it",
    description: "Admin only. The id is immutable for the same reason a column id is — tasks point at it.",
    input_data: "UpdateProjectKindInputModel",
    result: [
        {status_code: 200, description: "Updated"},
        {status_code: 400, description: "No such kind, or a bad colour"},
        {status_code: 401, description: "Not authenticated"},
        {status_code: 403, description: "Admin only"},
    ]
)]
pub struct UpdateKindAction {
    app: Arc<AppContext>,
}

impl UpdateKindAction {
    pub fn new(app: Arc<AppContext>) -> Self {
        Self { app }
    }
}

async fn handle_request(
    action: &UpdateKindAction,
    input_data: UpdateProjectKindInputModel,
    ctx: &HttpContext,
) -> Result<HttpOkResult, HttpFailResult> {
    crate::auth::require_admin(&action.app, ctx).await?;

    crate::scripts::update_kind(
        &action.app,
        &input_data.project_id,
        &input_data.kind_id,
        &input_data.name,
        &input_data.description,
        &input_data.color,
    )
    .await
    .map_err(bad_request)?;

    HttpOutput::Empty.into_ok_result(true)
}
