use crate::app::AppContext;
use crate::http_server::errors::{bad_request, not_found};
use crate::scripts::{DecisionPatch, TaskPatch};
use service_sdk::macros::use_my_http_server;
use std::sync::Arc;
use task_manager_shared::decisions::{AnswerDecisionInputModel, DecisionAnswer};

use_my_http_server!();

#[http_route(
    method: "POST", route: "/api/tasks/v1/decision/answer", controller: "Tasks",
    summary: "Answer a recorded human question",
    description: "A project member answers one task's pending question. Author and time come from the authenticated session and server, not the form. Original question and choices remain in history; identical retries are idempotent.",
    input_data: "AnswerDecisionInputModel",
    result: [
        {status_code: 202, description: "Answer recorded"},
        {status_code: 400, description: "Invalid answer or the question already resolved"},
        {status_code: 401, description: "Not authenticated"},
        {status_code: 404, description: "No task the user can access"},
    ]
)]
pub struct AnswerDecisionAction {
    app: Arc<AppContext>,
}
impl AnswerDecisionAction {
    pub fn new(app: Arc<AppContext>) -> Self {
        Self { app }
    }
}

async fn handle_request(
    action: &AnswerDecisionAction,
    input_data: AnswerDecisionInputModel,
    ctx: &HttpContext,
) -> Result<HttpOkResult, HttpFailResult> {
    let user = crate::auth::resolve_auth_user(&action.app, ctx).await?;
    {
        let board = action.app.board.read();
        let resolved =
            crate::scripts::resolve_task(&board, &input_data.task_id).map_err(not_found)?;
        if !user.is_admin && !resolved.project.is_member(&user.email) {
            return Err(not_found("no task you can access"));
        }
    }
    crate::scripts::update_task(
        &action.app,
        &input_data.task_id,
        TaskPatch {
            decision: DecisionPatch::Answer {
                id: input_data.decision_id,
                answer: DecisionAnswer {
                    option_id: input_data.option_id,
                    text: input_data.text,
                    answered_by: user.email,
                    answered_unix_seconds: 0,
                    source: "ui".into(),
                },
            },
            ..Default::default()
        },
    )
    .await
    .map_err(bad_request)?;
    HttpOutput::Empty.into_ok_result(true)
}
