use super::{authed, handle_http_empty};
use crate::models::RequestError;
use flurl::HttpVerb;
use task_manager_shared::decisions::AnswerDecisionInputModel;

pub async fn get_pending_decisions()
-> Result<task_manager_shared::decisions::PendingDecisionsResponse, RequestError> {
    super::handle_http_response(
        authed(
            "/api/tasks/v1/decisions/pending",
            HttpVerb::Post,
            flurl::EmptyRequestModel,
        )
        .await,
    )
    .await
}

pub async fn answer_task_decision(
    task_id: &str,
    decision_id: &str,
    option_id: Option<String>,
    text: String,
) -> Result<(), RequestError> {
    let request = AnswerDecisionInputModel {
        task_id: task_id.into(),
        decision_id: decision_id.into(),
        option_id,
        text,
    };
    handle_http_empty(authed("/api/tasks/v1/decision/answer", HttpVerb::Post, request).await).await
}
