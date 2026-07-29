use std::sync::Arc;

use service_sdk::macros::use_my_http_server;
use task_manager_shared::projects::AddProjectColumnInputModel;

use crate::app::AppContext;
use crate::http_server::errors::bad_request;

use_my_http_server!();

#[http_route(
    method: "POST",
    route: "/api/projects/v1/columns/add",
    controller: "Projects",
    summary: "Add a column",
    description: "Admin only. Sits between Todo and Done, which exist in every project and cannot be added. The id is typed in once and never renamed afterwards — tasks point at it as their status.",
    input_data: "AddProjectColumnInputModel",
    result: [
        {status_code: 200, description: "Added"},
        {status_code: 400, description: "Bad id, or the project already has that column"},
        {status_code: 401, description: "Not authenticated"},
        {status_code: 403, description: "Admin only"},
    ]
)]
pub struct AddColumnAction {
    app: Arc<AppContext>,
}

impl AddColumnAction {
    pub fn new(app: Arc<AppContext>) -> Self {
        Self { app }
    }
}

async fn handle_request(
    action: &AddColumnAction,
    input_data: AddProjectColumnInputModel,
    ctx: &HttpContext,
) -> Result<HttpOkResult, HttpFailResult> {
    crate::auth::require_admin(&action.app, ctx).await?;

    crate::scripts::add_column(
        &action.app,
        &input_data.project_id,
        &input_data.id,
        &input_data.name,
        &input_data.description,
        input_data.order,
    )
    .await
    .map_err(bad_request)?;

    HttpOutput::Empty.into_ok_result(true)
}
