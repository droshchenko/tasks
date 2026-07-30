use flurl::HttpVerb;
use task_manager_shared::goals::{
    GetGoalTasksInputModel, GetGoalsInputModel, GoalsResponse, SetGoalColorInputModel,
};
use task_manager_shared::tasks::TasksResponse;

use crate::models::RequestError;

use super::{authed, handle_http_empty, handle_http_response};

/// Read a project's goals.
///
/// Reads only, like the board: goals are opened, renamed and closed through `/mcp`. Archived ones are left
/// out — a goal closed longer ago than the project's window is history, and history is reached by searching
/// for its id.
pub async fn get_goals(project_id: &str) -> Result<GoalsResponse, RequestError> {
    let request = GetGoalsInputModel {
        project_id: project_id.to_string(),
        include_archived: None,
    };

    handle_http_response(authed("/api/goals/v1/list", HttpVerb::Post, request).await).await
}

/// The work under one goal, archived tasks included.
///
/// A second call rather than a filter over the board: the board stops at the archive window and a goal's
/// list must not, or the list would disagree with the counter above it. Made when a goal is expanded,
/// which is also the only moment anybody wants it.
pub async fn get_goal_tasks(project_id: &str, goal: &str) -> Result<TasksResponse, RequestError> {
    let request = GetGoalTasksInputModel {
        project_id: project_id.to_string(),
        goal: goal.to_string(),
    };

    handle_http_response(authed("/api/goals/v1/tasks", HttpVerb::Post, request).await).await
}

/// Recolour a goal.
///
/// The one thing the browser writes about the board, and only because a colour is presentation rather than
/// state — the same argument that puts a task type's colour behind a mouse. Everything else about a goal
/// still arrives through `/mcp`.
pub async fn set_goal_color(
    project_id: &str,
    goal: &str,
    color: &str,
) -> Result<(), RequestError> {
    let request = SetGoalColorInputModel {
        project_id: project_id.to_string(),
        goal: goal.to_string(),
        color: color.to_string(),
    };

    handle_http_empty(authed("/api/goals/v1/color", HttpVerb::Post, request).await).await
}
