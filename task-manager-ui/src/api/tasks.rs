use flurl::HttpVerb;
use task_manager_shared::tasks::{GetTasksInputModel, TasksResponse};

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

    handle_http_response(authed("/api/tasks/v1", HttpVerb::Get, request).await).await
}
