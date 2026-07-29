use flurl::HttpVerb;
use task_manager_shared::projects::ProjectResponse;
use task_manager_shared::tasks::{
    FindTaskInputModel, FindTaskResponse, GetTasksInputModel, TaskResponse, TasksResponse,
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

/// The answer a lookup by id would have given, assembled from what is already on screen.
///
/// No round trip: whichever screen is drawing the task already holds it and the project it is on, so the
/// dialog opens instantly. `archived` is false by definition of being drawn — the one caller that can show
/// archived work (a goal's own list) is showing it deliberately, and the dialog does not use the flag to
/// decide anything, only to say so.
pub fn find_task_locally(task: &TaskResponse, project: &ProjectResponse) -> FindTaskResponse {
    FindTaskResponse {
        task: Some(task.clone()),
        goal: None,
        project_id: project.id.clone(),
        project_prefix: project.prefix.clone(),
        project_name: project.name.clone(),
        archived: false,
        not_found: String::new(),
    }
}
