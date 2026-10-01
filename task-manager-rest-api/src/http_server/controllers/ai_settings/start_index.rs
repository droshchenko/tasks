use crate::{app::AppContext, http_server::errors::bad_request};
use service_sdk::macros::use_my_http_server;
use std::sync::Arc;
use task_manager_shared::ai_settings::*;
use_my_http_server!();

#[http_route(
    method: "POST",
    route: "/api/settings/ai/v1/index/start",
    controller: "AiSettings",
    summary: "StartIndex",
    description: "Authenticated administrators only. Provider credentials are never returned.",
    input_data: "StartIndexInput",
    result: [
        {status_code: 200, description: "Result", model: "IndexStatusResponse"},
        {status_code: 400, description: "Invalid settings or operation"},
        {status_code: 401, description: "Not authenticated"},
        {status_code: 403, description: "Admin only"},
    ]
)]
pub struct StartIndexAction {
    app: Arc<AppContext>,
}
impl StartIndexAction {
    pub fn new(app: Arc<AppContext>) -> Self {
        Self { app }
    }
}

async fn handle_request(
    action: &StartIndexAction,
    input_data: StartIndexInput,
    ctx: &HttpContext,
) -> Result<HttpOkResult, HttpFailResult> {
    let _user = crate::auth::require_admin(&action.app, ctx).await?;
    let result = crate::intelligence::jobs::start_index(
        action.app.clone(),
        &input_data.project,
        input_data.force,
    )
    .map_err(bad_request)?;
    HttpOutput::as_json(result).into_ok_result(true)
}
