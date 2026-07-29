use std::sync::Arc;

use service_sdk::macros::use_my_http_server;
use task_manager_shared::projects::AddProjectKindInputModel;

use crate::app::AppContext;
use crate::http_server::errors::bad_request;

use_my_http_server!();

#[http_route(
    method: "POST",
    route: "/api/projects/v1/kinds/add",
    controller: "Projects",
    summary: "Add a kind",
    description: "Admin only. The description is what an agent reads before classifying a task, so write the rule for applying it rather than a synonym of the name. The colour is one of the fixed palette values.",
    input_data: "AddProjectKindInputModel",
    result: [
        {status_code: 200, description: "Added"},
        {status_code: 400, description: "Bad id or colour, or the project already has that kind"},
        {status_code: 401, description: "Not authenticated"},
        {status_code: 403, description: "Admin only"},
    ]
)]
pub struct AddKindAction {
    app: Arc<AppContext>,
}

impl AddKindAction {
    pub fn new(app: Arc<AppContext>) -> Self {
        Self { app }
    }
}

async fn handle_request(
    action: &AddKindAction,
    input_data: AddProjectKindInputModel,
    ctx: &HttpContext,
) -> Result<HttpOkResult, HttpFailResult> {
    crate::auth::require_admin(&action.app, ctx).await?;

    crate::scripts::add_kind(
        &action.app,
        &input_data.project_id,
        &input_data.id,
        &input_data.name,
        &input_data.description,
        &input_data.color,
    )
    .await
    .map_err(bad_request)?;

    HttpOutput::Empty.into_ok_result(true)
}
