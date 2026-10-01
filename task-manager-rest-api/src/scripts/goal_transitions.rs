use crate::board::{BoardInner, CommentModel, GoalModel, TaskModel};
use rust_extensions::date_time::DateTimeAsMicroseconds;

pub async fn persist_task_change(
    app: &crate::app::AppContext,
    before: &BoardInner,
    task: TaskModel,
    ctx: &service_sdk::my_telemetry::MyTelemetryContext,
) {
    let goals = goal_transitions(before, &task, DateTimeAsMicroseconds::now());
    let goal_rows: Vec<crate::postgres::GoalDto> = goals.iter().map(Into::into).collect();
    app.tasks_repo
        .upsert_workflow(&(&task).into(), &goal_rows, ctx)
        .await;
    app.board.upsert_goals_and_tasks(goals, vec![task]);
}

// State stays derived. The transition time is durable because deletion/detachment cannot be
// reconstructed from the remaining tasks' completion dates.
pub fn goal_transitions(
    board: &BoardInner,
    after: &TaskModel,
    now: DateTimeAsMicroseconds,
) -> Vec<GoalModel> {
    let before_task = board.get_task(&after.project_id, after.number);
    let mut affected = vec![after.goal_number];
    if let Some(before) = before_task {
        affected.push(before.goal_number);
    }
    affected.sort_unstable();
    affected.dedup();
    let next = board.project_task_change(after.clone());
    let mut changes = Vec::new();
    for number in affected.into_iter().flatten() {
        let Some(current) = board.get_goal(&after.project_id, number) else {
            continue;
        };
        let (before_state, before_closed) = board.goal_state(&current);
        let (after_state, _) = next.goal_state(&current);
        let mut goal = current.as_ref().clone();
        let closed = if after_state == "done" {
            if before_state == "done" {
                before_closed
            } else {
                Some(now)
            }
        } else {
            None
        };
        if before_state != after_state || goal.close_moment != closed {
            goal.close_moment = closed;
            goal.auto_completed = after_state == "done"
                && (current.auto_completed
                    || current.close_moment.is_none()
                    || before_state != "done");
            goal.updated = now;
            if before_state != after_state && (before_state == "done" || after_state == "done") {
                let handle = board
                    .get_project(&after.project_id)
                    .map(|project| crate::board::compose_task_handle(&project.prefix, after.number))
                    .unwrap_or_else(|| after.number.to_string());
                let (total, done) = next.goal_progress(&after.project_id, number);
                goal.comments.push(CommentModel { moment: now, who: "AI".into(),
                    text: format!("**Goal state: {after_state}** after {handle} changed ({done}/{total} tasks done).\n\n_Task Manager, automatic goal tracking._") });
            }
            changes.push(goal);
        }
    }
    changes
}
