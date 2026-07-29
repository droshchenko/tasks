use task_manager_shared::goals::GoalResponse;

use crate::board::GoalModel;
use crate::postgres::GoalDto;

impl From<&GoalDto> for GoalModel {
    fn from(src: &GoalDto) -> Self {
        Self {
            id: src.id.clone(),
            project_id: src.project_id.clone(),
            name: src.name.clone(),
            description: src.description.clone(),
            created: src.created,
        }
    }
}

impl From<&GoalModel> for GoalDto {
    fn from(src: &GoalModel) -> Self {
        Self {
            id: src.id.clone(),
            project_id: src.project_id.clone(),
            name: src.name.clone(),
            description: src.description.clone(),
            created: src.created,
        }
    }
}

/// Memory -> wire, with the progress passed in: it is counted against the whole board, which a goal does
/// not know about.
pub fn goal_to_response(src: &GoalModel, tasks_amount: usize, done_amount: usize) -> GoalResponse {
    GoalResponse {
        id: src.id.clone(),
        project_id: src.project_id.clone(),
        name: src.name.clone(),
        description: src.description.clone(),
        tasks_amount: tasks_amount as i32,
        done_amount: done_amount as i32,
        created_unix_seconds: src.created.unix_microseconds / 1_000_000,
    }
}
