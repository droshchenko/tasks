use crate::board::{BoardInner, GoalModel, ProjectModel, TaskModel, compose_task_handle};
use task_manager_shared::projects::COLUMN_ID_DONE;
use task_manager_shared::readiness::{DependencyBlocker, GoalWaitingTask, TaskReadiness};
use task_manager_shared::task_title::task_title;

pub fn task_readiness(
    task: &TaskModel,
    project: &ProjectModel,
    board: &BoardInner,
) -> TaskReadiness {
    let dependencies =
        task.depends_on
            .iter()
            .filter_map(|number| {
                let linked = board.get_task(&task.project_id, *number);
                if linked.as_ref().is_some_and(|linked| {
                    project.effective_status(&linked.status) == COLUMN_ID_DONE
                }) {
                    return None;
                }
                Some(DependencyBlocker {
                    task_id: compose_task_handle(&project.prefix, *number),
                    title: linked
                        .as_ref()
                        .map(|linked| task_title(&linked.text).to_string())
                        .unwrap_or_default(),
                    status: linked
                        .as_ref()
                        .map(|linked| project.effective_status(&linked.status)),
                    assignee: linked.as_ref().and_then(|linked| linked.assignee.clone()),
                })
            })
            .collect();
    let unfinished = project.effective_status(&task.status) != COLUMN_ID_DONE;
    let analysis_required = unfinished
        && task.analysis_documents.is_empty()
        && [None, Some(COLUMN_ID_DONE)].into_iter().any(|target| {
            crate::scripts::resolve_execution_prompts(board, project, task, target)
                .unwrap_or_default()
                .iter()
                .any(|prompt| prompt.requires_analysis_documents)
        });
    TaskReadiness {
        dependencies,
        required_decisions: task
            .decisions
            .iter()
            .filter(|decision| decision.blocks_completion())
            .count() as i32,
        analysis_required,
    }
}

pub fn goal_waiting_tasks(goal: &GoalModel, board: &BoardInner) -> Vec<GoalWaitingTask> {
    let Some(project) = board.get_project(&goal.project_id) else {
        return Vec::new();
    };
    board
        .tasks_of_project(&goal.project_id)
        .iter()
        .filter(|task| {
            !task.is_deleted()
                && task.goal_number == Some(goal.number)
                && project.effective_status(&task.status) != COLUMN_ID_DONE
        })
        .filter_map(|task| {
            let readiness = task_readiness(task, &project, board);
            readiness.needs_attention().then(|| GoalWaitingTask {
                task_id: compose_task_handle(&project.prefix, task.number),
                title: task_title(&task.text).to_string(),
                readiness,
            })
        })
        .collect()
}
