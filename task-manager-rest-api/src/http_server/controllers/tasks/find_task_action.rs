use std::sync::Arc;

use service_sdk::macros::use_my_http_server;
use task_manager_shared::tasks::{FindTaskInputModel, FindTaskResponse};

use crate::app::AppContext;
use crate::mappers::task_to_response;

use_my_http_server!();

#[http_route(
    method: "GET",
    route: "/api/tasks/v1/find",
    controller: "Tasks",
    summary: "Look one task up by its id",
    description: "Resolves a handle — RMS-42, or RMS-000042 — to the exact task, whichever project it belongs to. Worth its own endpoint rather than a search of the loaded board for two reasons: work closed more than seven days ago is not in the board read at all, and a handle names its own project, which may not be the one on screen. An id that resolves to a project the caller may not see answers as not found rather than as a refusal, because saying \"no access\" would confirm the task exists.",
    input_data: "FindTaskInputModel",
    result: [
        {status_code: 200, description: "The hit, or a message saying why there is none", model: "FindTaskResponse"},
        {status_code: 401, description: "Not authenticated"},
    ]
)]
pub struct FindTaskAction {
    app: Arc<AppContext>,
}

impl FindTaskAction {
    pub fn new(app: Arc<AppContext>) -> Self {
        Self { app }
    }
}

/// Nothing found, with the reason in words.
fn nothing(reason: String) -> FindTaskResponse {
    FindTaskResponse {
        task: None,
        project_id: String::new(),
        project_prefix: String::new(),
        project_name: String::new(),
        archived: false,
        not_found: reason,
    }
}

async fn handle_request(
    action: &FindTaskAction,
    input_data: FindTaskInputModel,
    ctx: &HttpContext,
) -> Result<HttpOkResult, HttpFailResult> {
    let user = crate::auth::resolve_auth_user(&action.app, ctx).await?;

    let board = action.app.board.read();

    // `resolve_task` already says what would fix a miss — a bad shape, an unknown prefix with the known
    // ones listed, or a number that is not on that board. Passing its message through beats inventing a
    // flatter one here.
    let found = match crate::scripts::resolve_task(&board, &input_data.query) {
        Ok(found) => found,
        Err(err) => return HttpOutput::as_json(nothing(err)).into_ok_result(true),
    };

    // Not a 403: refusing would confirm that RMS-42 exists, which is the one thing somebody probing ids
    // for a project they cannot see would learn. "Not found" tells them nothing either way.
    if !user.is_admin && !found.project.is_member(&user.email) {
        return HttpOutput::as_json(nothing(format!(
            "no task {} you can see",
            input_data.query.trim()
        )))
        .into_ok_result(true);
    }

    let archived = board.is_archived(&found.task);
    let task = task_to_response(&found.task, &found.project, &board);

    HttpOutput::as_json(FindTaskResponse {
        task: Some(task),
        project_id: found.project.id.clone(),
        project_prefix: found.project.prefix.clone(),
        project_name: found.project.name.clone(),
        archived,
        not_found: String::new(),
    })
    .into_ok_result(true)
}
