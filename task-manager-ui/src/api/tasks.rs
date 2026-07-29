use flurl::HttpVerb;
use task_manager_shared::tasks::{
    FindTaskInputModel, FindTaskResponse, GetTasksInputModel, TasksResponse,
};

use crate::models::RequestError;

use super::{authed, handle_http_response};

/// Read a board.
///
/// The only task call there is: every mutation goes through `/mcp`, so there is deliberately no
/// `create_task` or `update_task` here. If one shows up, the design changed.
pub async fn get_tasks(project_id: &str) -> Result<TasksResponse, RequestError> {
    let request = GetTasksInputModel {
        project_id: project_id.to_string(),
    };

    handle_http_response(authed("/api/tasks/v1/list", HttpVerb::Post, request).await).await
}

/// Look one task up by its handle — `RMS-42`.
///
/// Always the server: a task closed more than seven days ago is not in the board read, and a handle names
/// its own project, which may not be the one on screen. A miss comes back as `not_found` text rather than
/// as an error, because "there is no RMS-999" is an answer, not a failure.
pub async fn find_task(query: &str) -> Result<FindTaskResponse, RequestError> {
    let request = FindTaskInputModel {
        query: query.to_string(),
    };

    handle_http_response(authed("/api/tasks/v1/find", HttpVerb::Post, request).await).await
}
