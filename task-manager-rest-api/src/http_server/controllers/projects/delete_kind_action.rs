use std::sync::Arc;

use service_sdk::macros::use_my_http_server;
use task_manager_shared::projects::DeleteProjectKindInputModel;

use crate::app::AppContext;
use crate::http_server::errors::bad_request;

use_my_http_server!();

#[http_route(
    method: "DELETE",
    route: "/api/projects/v1/{projectId}/kinds/{kindId}",
    controller: "Projects",
    summary: "Remove a kind",
    description: "Admin only. Tasks carrying it read as having no kind — the same leniency a deleted column gets.",
    input_data: "DeleteProjectKindInputModel",
    result: [
        {status_code: 200, description: "Removed"},
        {status_code: 400, description: "No such kind"},
        {status_code: 401, description: "Not authenticated"},
        {status_code: 403, description: "Admin only"},
    ]
)]
pub struct DeleteKindAction {
    app: Arc<AppContext>,
}

impl DeleteKindAction {
    pub fn new(app: Arc<AppContext>) -> Self {
        Self { app }
    }
}

async fn handle_request(
    action: &DeleteKindAction,
    input_data: DeleteProjectKindInputModel,
    ctx: &HttpContext,
) -> Result<HttpOkResult, HttpFailResult> {
    crate::auth::require_admin(&action.app, ctx).await?;

    crate::scripts::delete_kind(&action.app, &input_data.project_id, &input_data.kind_id)
        .await
        .map_err(bad_request)?;

    HttpOutput::Empty.into_ok_result(true)
}
