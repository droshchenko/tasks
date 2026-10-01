use crate::{app::AppContext, http_server::errors::bad_request};
use service_sdk::macros::use_my_http_server;
use std::sync::Arc;
use task_manager_shared::ai_settings::*;
use_my_http_server!();
#[http_route(
    method: "POST", route: "/api/settings/knowledge/v1/graph", controller: "KnowledgeSettings",
    summary: "UploadKnowledgeGraph", description: "Authenticated administrators only. Sources remain scoped to the selected project.",
    input_data: "UploadGraphInput",
    result: [
        {status_code: 200, description: "Result", model: "KnowledgeSettingsResponse"},
        {status_code: 400, description: "Invalid source or operation"},
        {status_code: 401, description: "Not authenticated"},
        {status_code: 403, description: "Admin only"},
    ]
)]
pub struct UploadKnowledgeGraphAction {
    app: Arc<AppContext>,
}
impl UploadKnowledgeGraphAction {
    pub fn new(app: Arc<AppContext>) -> Self {
        Self { app }
    }
}
async fn handle_request(
    action: &UploadKnowledgeGraphAction,
    input_data: UploadGraphInput,
    ctx: &HttpContext,
) -> Result<HttpOkResult, HttpFailResult> {
    let user = crate::auth::require_admin(&action.app, ctx).await?;
    let result = crate::intelligence::knowledge::upload_graph(&action.app, input_data, &user.email)
        .await
        .map_err(bad_request)?;
    HttpOutput::as_json(result).into_ok_result(true)
}
