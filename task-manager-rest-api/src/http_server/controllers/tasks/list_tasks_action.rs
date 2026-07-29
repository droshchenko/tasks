use std::sync::Arc;

use service_sdk::macros::use_my_http_server;
use task_manager_shared::tasks::{GetTasksInputModel, TasksResponse};

use crate::app::AppContext;
use crate::http_server::errors::not_found;
use crate::mappers::task_to_response;

use_my_http_server!();

#[http_route(
    method: "GET",
    route: "/api/tasks/v1",
    controller: "Tasks",
    summary: "Read a board",
    description: "The only task endpoint there is, and it only reads. Every task mutation arrives through /mcp — Home is a viewer, and nothing on it is edited with a mouse. Tasks come back oldest first with `blocked` and `blocks` derived, their handles composed from the project's current prefix, and an unknown status already folded into Todo.",
    input_data: "GetTasksInputModel",
    result: [
        {status_code: 200, description: "The board", model: "TasksResponse"},
        {status_code: 401, description: "Not authenticated"},
        {status_code: 403, description: "No access to this project"},
        {status_code: 404, description: "No such project"},
    ]
)]
pub struct ListTasksAction {
    app: Arc<AppContext>,
}

impl ListTasksAction {
    pub fn new(app: Arc<AppContext>) -> Self {
        Self { app }
    }
}

async fn handle_request(
    action: &ListTasksAction,
    input_data: GetTasksInputModel,
    ctx: &HttpContext,
) -> Result<HttpOkResult, HttpFailResult> {
    crate::auth::require_project_access(&action.app, ctx, &input_data.project_id).await?;

    let board = action.app.board.read();

    let project = board
        .get_project(&input_data.project_id)
        .ok_or_else(|| not_found("No such project"))?;

    let tasks = board
        .tasks_of_project(&project.id)
        .iter()
        .map(|task| task_to_response(task, &project, &board))
        .collect();

    HttpOutput::as_json(TasksResponse { tasks }).into_ok_result(true)
}
