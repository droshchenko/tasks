use std::sync::Arc;

use service_sdk::macros::use_my_http_server;
use task_manager_shared::projects::SetProjectKindsInputModel;

use crate::app::AppContext;
use crate::http_server::errors::bad_request;

use_my_http_server!();

#[http_route(
    method: "POST",
    route: "/api/projects/v1/kinds/set",
    controller: "Projects",
    summary: "Replace this project's task types",
    description: "Admin only. The complete set in one call, replacing whatever was there. There is no add or delete: the dialog that edits this builds the whole new list and sends it, so a half-finished edit is never visible and the server never reconciles a sequence of small writes. Validated as a whole — a duplicate id or a bad colour fails the call and leaves the project exactly as it was. A task pointing at a type this call removed keeps its stored value and reads as having none, so putting the type back brings those tasks home.",
    input_data: "SetProjectKindsInputModel",
    result: [
        {status_code: 200, description: "Set"},
        {status_code: 400, description: "No such project, a duplicate id, a missing name or an unknown colour"},
        {status_code: 401, description: "Not authenticated"},
        {status_code: 403, description: "Admin only"},
    ]
)]
pub struct SetKindsAction {
    app: Arc<AppContext>,
}

impl SetKindsAction {
    pub fn new(app: Arc<AppContext>) -> Self {
        Self { app }
    }
}

async fn handle_request(
    action: &SetKindsAction,
    input_data: SetProjectKindsInputModel,
    ctx: &HttpContext,
) -> Result<HttpOkResult, HttpFailResult> {
    crate::auth::require_admin(&action.app, ctx).await?;

    crate::scripts::set_kinds(&action.app, &input_data.project_id, &input_data.kinds)
        .await
        .map_err(bad_request)?;

    HttpOutput::Empty.into_ok_result(true)
}
