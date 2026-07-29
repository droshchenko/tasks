use std::sync::Arc;

use service_sdk::macros::use_my_http_server;
use task_manager_shared::projects::DeleteProjectColumnInputModel;

use crate::app::AppContext;
use crate::http_server::errors::bad_request;

use_my_http_server!();

#[http_route(
    method: "POST",
    route: "/api/projects/v1/columns/delete",
    controller: "Projects",
    summary: "Remove a column",
    description: "Admin only. Its tasks are left alone: their stored status still names this column and they read as Todo from now on, so re-creating a column with the same id brings them straight back to it.",
    input_data: "DeleteProjectColumnInputModel",
    result: [
        {status_code: 200, description: "Removed"},
        {status_code: 400, description: "No such column"},
        {status_code: 401, description: "Not authenticated"},
        {status_code: 403, description: "Admin only"},
    ]
)]
pub struct DeleteColumnAction {
    app: Arc<AppContext>,
}

impl DeleteColumnAction {
    pub fn new(app: Arc<AppContext>) -> Self {
        Self { app }
    }
}

async fn handle_request(
    action: &DeleteColumnAction,
    input_data: DeleteProjectColumnInputModel,
    ctx: &HttpContext,
) -> Result<HttpOkResult, HttpFailResult> {
    crate::auth::require_admin(&action.app, ctx).await?;

    crate::scripts::delete_column(&action.app, &input_data.project_id, &input_data.column_id)
        .await
        .map_err(bad_request)?;

    HttpOutput::Empty.into_ok_result(true)
}
