use std::sync::Arc;

use service_sdk::macros::use_my_http_server;
use task_manager_shared::projects::UpdateProjectColumnInputModel;

use crate::app::AppContext;
use crate::http_server::errors::bad_request;

use_my_http_server!();

#[http_route(
    method: "PUT",
    route: "/api/projects/v1/{projectId}/columns/{columnId}",
    controller: "Projects",
    summary: "Rename a column or move it",
    description: "Admin only. The id is not touched: tasks reference it as their status, and there is no rename that would not have to rewrite every one of them.",
    input_data: "UpdateProjectColumnInputModel",
    result: [
        {status_code: 200, description: "Updated"},
        {status_code: 400, description: "No such column"},
        {status_code: 401, description: "Not authenticated"},
        {status_code: 403, description: "Admin only"},
    ]
)]
pub struct UpdateColumnAction {
    app: Arc<AppContext>,
}

impl UpdateColumnAction {
    pub fn new(app: Arc<AppContext>) -> Self {
        Self { app }
    }
}

async fn handle_request(
    action: &UpdateColumnAction,
    input_data: UpdateProjectColumnInputModel,
    ctx: &HttpContext,
) -> Result<HttpOkResult, HttpFailResult> {
    crate::auth::require_admin(&action.app, ctx).await?;

    crate::scripts::update_column(
        &action.app,
        &input_data.project_id,
        &input_data.column_id,
        &input_data.name,
        &input_data.description,
        input_data.order,
    )
    .await
    .map_err(bad_request)?;

    HttpOutput::Empty.into_ok_result(true)
}
