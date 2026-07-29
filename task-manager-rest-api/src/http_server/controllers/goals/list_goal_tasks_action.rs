use std::sync::Arc;

use service_sdk::macros::use_my_http_server;
use task_manager_shared::goals::GetGoalTasksInputModel;
use task_manager_shared::tasks::TasksResponse;

use crate::app::AppContext;
use crate::http_server::errors::not_found;
use crate::mappers::task_to_response;

use_my_http_server!();

#[http_route(
    method: "POST",
    route: "/api/goals/v1/tasks",
    controller: "Goals",
    summary: "The work under one goal",
    description: "Every task of one goal, oldest first, INCLUDING work already archived off the board. That is the difference from /api/tasks/v1/list and the reason this exists: a goal can only be closed once all of its tasks are done, so by the time it closes the oldest of them have aged out — and a list that stopped at the archive window would contradict the goal's own `done_amount`. Read when a goal is expanded, not up front.",
    input_data: "GetGoalTasksInputModel",
    result: [
        {status_code: 200, description: "The goal's tasks", model: "TasksResponse"},
        {status_code: 401, description: "Not authenticated"},
        {status_code: 403, description: "No access to this project"},
        {status_code: 404, description: "No such project"},
    ]
)]
pub struct ListGoalTasksAction {
    app: Arc<AppContext>,
}

impl ListGoalTasksAction {
    pub fn new(app: Arc<AppContext>) -> Self {
        Self { app }
    }
}

async fn handle_request(
    action: &ListGoalTasksAction,
    input_data: GetGoalTasksInputModel,
    ctx: &HttpContext,
) -> Result<HttpOkResult, HttpFailResult> {
    crate::auth::require_project_access(&action.app, ctx, &input_data.project_id).await?;

    let board = action.app.board.read();

    let project = board
        .get_project(&input_data.project_id)
        .ok_or_else(|| not_found("No such project"))?;

    // The same resolution the tools use, so `RMS-G7` and `7` mean the same thing on every door, and an
    // unknown goal comes back as a message rather than as an empty list that reads like "no work here".
    let number = crate::scripts::resolve_goal_reference(&board, &project, &input_data.goal)
        .map_err(|err| not_found(&err))?;

    let tasks = board
        .tasks_of_goal(&project.id, number)
        .iter()
        .map(|task| task_to_response(task, &project, &board))
        .collect();

    HttpOutput::as_json(TasksResponse { tasks }).into_ok_result(true)
}
