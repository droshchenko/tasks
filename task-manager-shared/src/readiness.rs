use my_http_utils::macros::MyHttpObjectStructure;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, MyHttpObjectStructure, Clone, Debug, PartialEq)]
pub struct DependencyBlocker {
    pub task_id: String,
    pub title: String,
    // None means missing or deleted, not a completed dependency.
    pub status: Option<String>,
    pub assignee: Option<String>,
}

#[derive(Serialize, Deserialize, MyHttpObjectStructure, Clone, Debug, PartialEq, Default)]
pub struct TaskReadiness {
    #[serde(default)]
    pub dependencies: Vec<DependencyBlocker>,
    #[serde(default)]
    pub required_decisions: i32,
    #[serde(default)]
    pub analysis_required: bool,
}

impl TaskReadiness {
    pub fn needs_attention(&self) -> bool {
        !self.dependencies.is_empty() || self.required_decisions > 0 || self.analysis_required
    }
}

#[derive(Serialize, Deserialize, MyHttpObjectStructure, Clone, Debug, PartialEq)]
pub struct GoalWaitingTask {
    pub task_id: String,
    pub title: String,
    pub readiness: TaskReadiness,
}
