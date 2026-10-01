use crate::app::AppContext;
use service_sdk::macros::use_my_http_server;
use std::sync::Arc;
use task_manager_shared::decisions::PendingDecisionsResponse;

use_my_http_server!();

#[http_route(
    method: "POST", route: "/api/tasks/v1/decisions/pending", controller: "Tasks",
    summary: "Questions awaiting a human answer",
    description: "Pending required and optional questions from non-deleted tasks in active projects visible to the authenticated user. Required questions first, oldest first. Reads never change task state.",
    result: [
        {status_code: 200, description: "Pending questions", model: "PendingDecisionsResponse"},
        {status_code: 401, description: "Not authenticated"},
    ]
)]
pub struct PendingDecisionsAction {
    app: Arc<AppContext>,
}

impl PendingDecisionsAction {
    pub fn new(app: Arc<AppContext>) -> Self {
        Self { app }
    }
}

async fn handle_request(
    action: &PendingDecisionsAction,
    ctx: &HttpContext,
) -> Result<HttpOkResult, HttpFailResult> {
    let user = crate::auth::resolve_auth_user(&action.app, ctx).await?;
    let board = action.app.board.read();
    let items = crate::mappers::pending_decisions_for(&board, &user.email, user.is_admin);
    HttpOutput::as_json(PendingDecisionsResponse { items }).into_ok_result(true)
}
