use flurl::{EmptyRequestModel, HttpVerb};
use task_manager_shared::projects::*;

use crate::models::RequestError;

use super::{authed, handle_http_empty, handle_http_response};

// Every mutation here is a POST to a STATIC url, with the ids in the body. Not a style choice — a
// constraint of the request builder: `#[http_path]` fields are APPENDED to the url in declaration order
// (`__url.append_path_segment(...)`), there is no `{name}` substitution. So a route may not carry a
// static segment after a parameter. `/api/projects/v1/{projectId}/columns` cannot be addressed at all:
// the client can only produce `/api/projects/v1/{projectId}`, which is a 404, and that is exactly the
// bug this shape replaced. Param-last (`/api/users/v1/{email}`) is fine and is used where it fits.

pub async fn get_projects() -> Result<ProjectsResponse, RequestError> {
    let response = authed("/api/projects/v1", HttpVerb::Get, EmptyRequestModel).await;

    handle_http_response(response).await
}

pub async fn create_project(
    name: &str,
    description: &str,
    prefix: &str,
) -> Result<(), RequestError> {
    let request = CreateProjectInputModel {
        name: name.to_string(),
        description: description.to_string(),
        prefix: prefix.to_string(),
    };

    handle_http_empty(authed("/api/projects/v1", HttpVerb::Post, request).await).await
}

pub async fn update_project(
    project_id: &str,
    name: &str,
    description: &str,
    prefix: &str,
) -> Result<(), RequestError> {
    let request = UpdateProjectInputModel {
        project_id: project_id.to_string(),
        name: name.to_string(),
        description: description.to_string(),
        prefix: prefix.to_string(),
    };

    handle_http_empty(authed("/api/projects/v1/update", HttpVerb::Post, request).await).await
}

/// Point a project at a column template, or at none with an empty id.
pub async fn set_column_template(
    project_id: &str,
    column_template_id: &str,
) -> Result<(), RequestError> {
    let request = SetProjectColumnTemplateInputModel {
        project_id: project_id.to_string(),
        column_template_id: column_template_id.to_string(),
    };

    handle_http_empty(
        authed(
            "/api/projects/v1/column-template/set",
            HttpVerb::Post,
            request,
        )
        .await,
    )
    .await
}

/// Point a project at a task-type template, or at none with an empty id.
pub async fn set_kind_template(
    project_id: &str,
    kind_template_id: &str,
) -> Result<(), RequestError> {
    let request = SetProjectKindTemplateInputModel {
        project_id: project_id.to_string(),
        kind_template_id: kind_template_id.to_string(),
    };

    handle_http_empty(
        authed(
            "/api/projects/v1/kind-template/set",
            HttpVerb::Post,
            request,
        )
        .await,
    )
    .await
}

/// Replaces the whole set — which is how the screen works, and means this side never has to diff.
pub async fn set_members(project_id: &str, members: Vec<String>) -> Result<(), RequestError> {
    let request = SetProjectMembersInputModel {
        project_id: project_id.to_string(),
        members,
    };

    handle_http_empty(authed("/api/projects/v1/members/set", HttpVerb::Post, request).await).await
}
