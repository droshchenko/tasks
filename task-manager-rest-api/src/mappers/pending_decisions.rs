use crate::board::{BoardInner, compose_task_handle};
use task_manager_shared::decisions::PendingDecision;
use task_manager_shared::task_title::task_title;

pub fn pending_decisions_for(
    board: &BoardInner,
    email: &str,
    is_admin: bool,
) -> Vec<PendingDecision> {
    let mut items = Vec::new();
    for project in board.projects_visible_to(email, is_admin) {
        if project.archived_moment.is_some() {
            continue;
        }
        for task in board.tasks_of_project(&project.id) {
            if task.is_deleted() {
                continue;
            }
            for decision in task
                .decisions
                .iter()
                .filter(|decision| decision.is_waiting())
            {
                items.push(PendingDecision {
                    task_id: compose_task_handle(&project.prefix, task.number),
                    task_title: task_title(&task.text).to_string(),
                    project: project.prefix.clone(),
                    project_name: project.name.clone(),
                    decision: decision.clone(),
                });
            }
        }
    }
    // Required questions first, oldest first within each group. Stable tie-breakers avoid a jumping inbox.
    items.sort_by(|a, b| {
        (
            !a.decision.required,
            a.decision.asked_unix_seconds,
            &a.task_id,
            &a.decision.id,
        )
            .cmp(&(
                !b.decision.required,
                b.decision.asked_unix_seconds,
                &b.task_id,
                &b.decision.id,
            ))
    });
    items
}
