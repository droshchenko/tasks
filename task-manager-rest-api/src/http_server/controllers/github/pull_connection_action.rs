use std::sync::Arc;

use service_sdk::macros::use_my_http_server;
use task_manager_shared::github::{PullGithubConnectionInputModel, PullGithubConnectionResponse};

use crate::app::AppContext;
use crate::http_server::errors::{bad_request, not_found};

use_my_http_server!();

#[http_route(
    method: "POST",
    route: "/api/github/v1/pull",
    controller: "GitHub",
    summary: "Refresh one connection now",
    description: "THIS DELETES THE CONNECTION'S FOLDER AND CLONES THE REPOSITORY AGAIN. It is not the ten-minute timer's fetch asked for early: that keeps the clone current, and this replaces it, which is what settles a folder a fetch cannot fix — a force-pushed branch, a file that stopped being tracked, a working copy left on the branch it was cloned on. Nothing on the documents surface writes into a connected repository, so what is thrown away is a copy; what is NOT a copy is anything somebody left in it through `github_git` — an unpushed commit, a stash — and that goes with the folder. IT RETURNS BEFORE THE PULL FINISHES — a repository can take half a minute to arrive, and a request held open for that is a request that times out somewhere in between. What comes back is a receipt: `pull_no`, the connection's finished-listings count as it was when the ask was accepted. Read the connections list until that connection reports a HIGHER one and the run is over, whatever it did — the state and the error on it then describe THIS run rather than the one before. A pull that is already running is not started twice: the second caller watches the listing already under way, which is the same answer it was asking for.",
    input_data: "PullGithubConnectionInputModel",
    result: [
        {status_code: 200, description: "A pull was asked for", model: "PullGithubConnectionResponse"},
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

    let pull_no =
        crate::scripts::pull_github_connection(&action.app, &input_data.project, &input_data.name)
            .await
            .map_err(bad_request)?;

    HttpOutput::as_json(PullGithubConnectionResponse { pull_no }).into_ok_result(true)
}
