use std::sync::Arc;

use mcp_server_middleware::*;
use serde::{Deserialize, Serialize};

use crate::app::AppContext;
use crate::mcp::ProjectView;

#[derive(ApplyJsonSchema, Debug, Serialize, Deserialize)]
pub struct ProjectsListInput {
    #[property(
        description = "Return only this project, by prefix. Omit to get every project, which is what you want the first time"
    )]
    pub project: Option<String>,
}

#[derive(ApplyJsonSchema, Debug, Serialize, Deserialize)]
pub struct ProjectsListResponse {
    #[property(description = "The projects, by name")]
    pub projects: Vec<ProjectView>,
    #[property(description = "Number of rows in `projects`")]
    pub amount: i32,
}

pub struct ProjectsListHandler {
    app: Arc<AppContext>,
}

impl ProjectsListHandler {
    pub fn new(app: Arc<AppContext>) -> Self {
        Self { app }
    }
}

impl ToolDefinition for ProjectsListHandler {
    const FUNC_NAME: &'static str = "projects_list";
    const DESCRIPTION: &'static str = "Call this FIRST, before anything else. It is the only tool \
that reveals which projects exist, and it returns the four things every other call needs: the \
project prefixes that name a board, the column ids a task's status must be one of, the kind ids and \
what each one means on this particular board, and the people who may be assigned work on it. \
Everything after this is named by an id that came from here.";
}

#[async_trait::async_trait]
impl McpToolCall<ProjectsListInput, ProjectsListResponse> for ProjectsListHandler {
    async fn execute_tool_call(
        &self,
        model: ProjectsListInput,
    ) -> Result<ProjectsListResponse, String> {
        let board = self.app.board.read();

        // A named project that does not exist is an error with the real prefixes in it, never an empty
        // list — an empty list reads as "there are no projects", which is a different fact.
        let projects = match &model.project {
            Some(prefix) => {
                let project = crate::scripts::resolve_project_by_prefix(&board, prefix)?;
                vec![ProjectView::from_model(&project, &board)]
            }
            None => board
                .projects()
                .iter()
                .map(|project| ProjectView::from_model(project, &board))
                .collect(),
        };

        Ok(ProjectsListResponse {
            amount: projects.len() as i32,
            projects,
        })
    }
}
