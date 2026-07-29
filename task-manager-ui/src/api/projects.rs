use flurl::{EmptyRequestModel, HttpVerb};
use task_manager_shared::projects::*;

use crate::models::RequestError;

use super::{authed, handle_http_empty, handle_http_response};

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

    // The path segment is filled from the model's `#[http_path]` field, so the URL carries only the
    // static part.
    handle_http_empty(authed("/api/projects/v1", HttpVerb::Put, request).await).await
}

pub async fn add_column(
    project_id: &str,
    id: &str,
    name: &str,
    description: &str,
    order: i32,
) -> Result<(), RequestError> {
    let request = AddProjectColumnInputModel {
        project_id: project_id.to_string(),
        id: id.to_string(),
        name: name.to_string(),
        description: description.to_string(),
        order,
    };

    handle_http_empty(authed("/api/projects/v1", HttpVerb::Post, request).await).await
}

pub async fn update_column(
    project_id: &str,
    column_id: &str,
    name: &str,
    description: &str,
    order: i32,
) -> Result<(), RequestError> {
    let request = UpdateProjectColumnInputModel {
        project_id: project_id.to_string(),
        column_id: column_id.to_string(),
        name: name.to_string(),
        description: description.to_string(),
        order,
    };

    handle_http_empty(authed("/api/projects/v1", HttpVerb::Put, request).await).await
}

pub async fn delete_column(project_id: &str, column_id: &str) -> Result<(), RequestError> {
    let request = DeleteProjectColumnInputModel {
        project_id: project_id.to_string(),
        column_id: column_id.to_string(),
    };

    handle_http_empty(authed("/api/projects/v1", HttpVerb::Delete, request).await).await
}

pub async fn add_kind(
    project_id: &str,
    id: &str,
    name: &str,
    description: &str,
    color: &str,
) -> Result<(), RequestError> {
    let request = AddProjectKindInputModel {
        project_id: project_id.to_string(),
        id: id.to_string(),
        name: name.to_string(),
        description: description.to_string(),
        color: color.to_string(),
    };

    handle_http_empty(authed("/api/projects/v1", HttpVerb::Post, request).await).await
}

pub async fn update_kind(
    project_id: &str,
    kind_id: &str,
    name: &str,
    description: &str,
    color: &str,
) -> Result<(), RequestError> {
    let request = UpdateProjectKindInputModel {
        project_id: project_id.to_string(),
        kind_id: kind_id.to_string(),
        name: name.to_string(),
        description: description.to_string(),
        color: color.to_string(),
    };

    handle_http_empty(authed("/api/projects/v1", HttpVerb::Put, request).await).await
}

pub async fn delete_kind(project_id: &str, kind_id: &str) -> Result<(), RequestError> {
    let request = DeleteProjectKindInputModel {
        project_id: project_id.to_string(),
        kind_id: kind_id.to_string(),
    };

    handle_http_empty(authed("/api/projects/v1", HttpVerb::Delete, request).await).await
}

/// Replaces the whole set — which is how the screen works, and means this side never has to diff.
pub async fn set_members(project_id: &str, members: Vec<String>) -> Result<(), RequestError> {
    let request = SetProjectMembersInputModel {
        project_id: project_id.to_string(),
        members,
    };

    handle_http_empty(authed("/api/projects/v1", HttpVerb::Put, request).await).await
}
