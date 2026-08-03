use std::sync::Arc;

use service_sdk::macros::use_my_http_server;
use task_manager_shared::github::PullGithubConnectionInputModel;

use crate::app::AppContext;
use crate::http_server::errors::{bad_request, not_found};

use_my_http_server!();

#[http_route(
    method: "POST",
    route: "/api/github/v1/pull",
    controller: "GitHub",
    summary: "Refresh one connection now",
    description: "Every connection is pulled on a ten-minute timer anyway; this is the button for somebody who has just pushed and does not want to wait for it. IT RETURNS BEFORE THE PULL FINISHES — a repository can take half a minute to arrive, and a request held open for that is a request that times out somewhere in between. Read the state back from the connections list. A pull that is already running is not started twice: the second caller gets the same answer and the same pull.",
    input_data: "PullGithubConnectionInputModel",
    result: [
        {status_code: 200, description: "A pull was asked for"},
        {status_code: 400, description: "No such connection"},
        {status_code: 401, description: "Not authenticated"},
        {status_code: 403, description: "No access to this project"},
        {status_code: 404, description: "No such project"},
    ]
)]
pub struct PullGithubConnectionAction {
    app: Arc<AppContext>,
}

impl PullGithubConnectionAction {
    pub fn new(app: Arc<AppContext>) -> Self {
        Self { app }
    }
}

async fn handle_request(
    action: &PullGithubConnectionAction,
    input_data: PullGithubConnectionInputModel,
    ctx: &HttpContext,
) -> Result<HttpOkResult, HttpFailResult> {
    let project_id = {
        let board = action.app.board.read();

        match crate::scripts::resolve_project_by_prefix(&board, &input_data.project) {
            Ok(project) => project.id.clone(),
            Err(err) => return Err(not_found(err)),
        }
    };

    crate::auth::require_project_access(&action.app, ctx, &project_id).await?;

    crate::scripts::pull_github_connection(&action.app, &input_data.project, &input_data.name)
        .await
        .map_err(bad_request)?;

    HttpOutput::Empty.into_ok_result(true)
}
