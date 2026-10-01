use crate::{app::AppContext, http_server::errors::bad_request};
use service_sdk::macros::use_my_http_server;
use std::sync::Arc;
use task_manager_shared::ai_settings::*;
use_my_http_server!();

#[http_route(
    method: "POST",
    route: "/api/settings/ai/v1/index/status",
    controller: "AiSettings",
    summary: "IndexStatus",
    description: "Authenticated administrators only. Provider credentials are never returned.",
    input_data: "ProjectAiInput",
    result: [
        {status_code: 200, description: "Result", model: "IndexStatusResponse"},
        {status_code: 400, description: "Invalid settings or operation"},
        {status_code: 401, description: "Not authenticated"},
        {status_code: 403, description: "Admin only"},
    ]
)]
pub struct IndexStatusAction {
    app: Arc<AppContext>,
}
impl IndexStatusAction {
    pub fn new(app: Arc<AppContext>) -> Self {
        Self { app }
    }
}

async fn handle_request(
    action: &IndexStatusAction,
    input_data: ProjectAiInput,
    ctx: &HttpContext,
) -> Result<HttpOkResult, HttpFailResult> {
    let _user = crate::auth::require_admin(&action.app, ctx).await?;
    let result = crate::intelligence::jobs::index_status(&action.app, &input_data.project)
        .map_err(bad_request)?;
    HttpOutput::as_json(result).into_ok_result(true)
}
