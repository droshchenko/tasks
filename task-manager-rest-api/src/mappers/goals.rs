use task_manager_shared::goals::GoalResponse;

use crate::board::{CommentModel, GoalModel, SubtaskModel, compose_goal_handle};
use crate::postgres::{GoalCommentJsonModel, GoalDto, GoalSubtaskJsonModel};

impl From<&GoalDto> for GoalModel {
    fn from(src: &GoalDto) -> Self {
        Self {
            project_id: src.project_id.clone(),
            number: src.number,
            name: src.name.clone(),
            description: src.description.clone(),
            // Unknown reads as the default swatch rather than failing the load, the same leniency a task
            // type's colour gets.
            color: task_manager_shared::kind_color::KindColor::parse_or_default(
                src.color.as_deref().unwrap_or_default(),
            ),
            // A NULL column is a goal written before checklists existed, and it reads as having none.
            subtasks: src
                .subtasks
                .as_deref()
                .unwrap_or_default()
                .iter()
                .map(|itm| itm.into())
                .collect(),
            comments: src.comments.iter().map(|itm| itm.into()).collect(),
            created: src.created,
            updated: src.updated,
            close_moment: src.close_moment,
        }
    }
}

impl From<&GoalModel> for GoalDto {
    fn from(src: &GoalModel) -> Self {
        Self {
            // Dead column, written because the deployed table still has it NOT NULL. See `GoalDto`.
            id: crate::postgres::dead_id(&src.project_id, src.number),
            project_id: src.project_id.clone(),
            number: src.number,
            name: src.name.clone(),
            description: src.description.clone(),
            color: Some(rust_extensions::AsStr::as_str(&src.color).to_string()),
            // Always a real array, even when empty — the column is nullable only so it could be added to a
            // populated table.
            subtasks: Some(src.subtasks.iter().map(|itm| itm.into()).collect()),
            comments: src.comments.iter().map(|itm| itm.into()).collect(),
            created: src.created,
            updated: src.updated,
            close_moment: src.close_moment,
        }
    }
}

impl From<&GoalSubtaskJsonModel> for SubtaskModel {
    fn from(src: &GoalSubtaskJsonModel) -> Self {
        Self {
            id: src.id.clone(),
            title: src.title.clone(),
            text: src.text.clone(),
            done: src.done,
        }
    }
}

impl From<&SubtaskModel> for GoalSubtaskJsonModel {
    fn from(src: &SubtaskModel) -> Self {
        Self {
            id: src.id.clone(),
            title: src.title.clone(),
            text: src.text.clone(),
            done: src.done,
        }
    }
}

impl From<&GoalCommentJsonModel> for CommentModel {
    fn from(src: &GoalCommentJsonModel) -> Self {
        Self {
            moment: rust_extensions::date_time::DateTimeAsMicroseconds::new(
                src.moment_unix_seconds * 1_000_000,
            ),
            who: src.who.clone(),
            text: src.text.clone(),
        }
    }
}

impl From<&CommentModel> for GoalCommentJsonModel {
    fn from(src: &CommentModel) -> Self {
        Self {
            moment_unix_seconds: src.moment.unix_microseconds / 1_000_000,
            who: src.who.clone(),
            text: src.text.clone(),
        }
    }
}

/// Memory -> wire.
///
/// `prefix` and the progress both come from outside the goal: the handle is composed from the project's
/// current prefix, and the counts are taken against the whole board, which a goal knows nothing about.
pub fn goal_to_response(
    src: &GoalModel,
    prefix: &str,
    tasks_amount: usize,
    done_amount: usize,
) -> GoalResponse {
    GoalResponse {
        id: compose_goal_handle(prefix, src.number),
        project_id: src.project_id.clone(),
        name: src.name.clone(),
        description: src.description.clone(),
        color: rust_extensions::AsStr::as_str(&src.color).to_string(),
        status: src.status().to_string(),
        tasks_amount: tasks_amount as i32,
        done_amount: done_amount as i32,
        subtasks: super::subtasks_to_response(&src.subtasks),
        comments: src
            .comments
            .iter()
            .map(|itm| task_manager_shared::tasks::TaskCommentResponse {
                moment_unix_seconds: itm.moment.unix_microseconds / 1_000_000,
                who: itm.who.clone(),
                text: itm.text.clone(),
            })
            .collect(),
        created_unix_seconds: src.created.unix_microseconds / 1_000_000,
        updated_unix_seconds: src.updated.unix_microseconds / 1_000_000,
        closed_unix_seconds: src
            .close_moment
            .map(|itm| itm.unix_microseconds / 1_000_000),
    }
}
