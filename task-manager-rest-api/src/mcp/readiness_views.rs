use mcp_server_middleware::*;
use serde::{Deserialize, Serialize};

#[derive(ApplyJsonSchema, Debug, Serialize, Deserialize)]
pub struct DependencyBlockerView {
    #[property(
        description = "The unfinished dependency to complete, or the missing reference to restore or correct"
    )]
    pub task_id: String,
    #[property(description = "Dependency title; empty when the task is missing or deleted")]
    pub title: String,
    #[property(
        description = "Current effective status; absent means missing or deleted, never satisfied"
    )]
    pub status: Option<String>,
    #[property(description = "Who is assigned to the dependency")]
    pub assignee: Option<String>,
}

#[derive(ApplyJsonSchema, Debug, Serialize, Deserialize)]
pub struct TaskReadinessView {
    #[property(
        description = "Exactly the dependencies that currently cause blocked, excluding completed ones"
    )]
    pub dependencies: Vec<DependencyBlockerView>,
    #[property(description = "Required pending human questions that prevent completion")]
    pub required_decisions: i32,
    #[property(
        description = "Analysis-file references are missing and required by current stage/type or Done entry rules"
    )]
    pub analysis_required: bool,
}

impl From<task_manager_shared::readiness::TaskReadiness> for TaskReadinessView {
    fn from(value: task_manager_shared::readiness::TaskReadiness) -> Self {
        Self {
            dependencies: value
                .dependencies
                .into_iter()
                .map(|item| DependencyBlockerView {
                    task_id: item.task_id,
                    title: item.title,
                    status: item.status,
                    assignee: item.assignee,
                })
                .collect(),
            required_decisions: value.required_decisions,
            analysis_required: value.analysis_required,
        }
    }
}

#[derive(ApplyJsonSchema, Debug, Serialize, Deserialize)]
pub struct GoalWaitingTaskView {
    #[property(description = "Unfinished task belonging to this goal")]
    pub task_id: String,
    #[property(description = "Task title")]
    pub title: String,
    #[property(
        description = "Concrete dependencies, human questions and missing analysis links for this task"
    )]
    pub readiness: TaskReadinessView,
}

impl From<task_manager_shared::readiness::GoalWaitingTask> for GoalWaitingTaskView {
    fn from(value: task_manager_shared::readiness::GoalWaitingTask) -> Self {
        Self {
            task_id: value.task_id,
            title: value.title,
            readiness: value.readiness.into(),
        }
    }
}
