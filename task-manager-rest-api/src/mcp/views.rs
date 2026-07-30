use mcp_server_middleware::*;
use rust_extensions::AsStr;
use serde::{Deserialize, Serialize};

use crate::board::{
    BoardInner, GoalModel, ProjectModel, TaskModel, UserModel, compose_task_handle,
};

/// One column of a board, as a tool sees it.
#[derive(ApplyJsonSchema, Debug, Serialize, Deserialize)]
pub struct ColumnView {
    #[property(
        description = "The column id — this is what a task's `status` is set to. `todo` and `done` exist in every project and are always the first and last column"
    )]
    pub id: String,
    #[property(description = "What the column is called on the board")]
    pub name: String,
    #[property(description = "What belongs in this column, as the person who created it wrote it")]
    pub description: String,
}

/// One kind of work, as a tool sees it.
#[derive(ApplyJsonSchema, Debug, Serialize, Deserialize)]
pub struct KindView {
    #[property(description = "The kind id — this is what a task's `kind` is set to")]
    pub id: String,
    #[property(description = "What the kind is called on the board")]
    pub name: String,
    #[property(
        description = "What qualifies as this kind, as the person who created it wrote it. Read it before classifying a task — the meanings are per project and are not the ones you would guess"
    )]
    pub description: String,
    #[property(description = "The colour the board draws this kind in")]
    pub color: String,
}

/// One project, as a tool sees it.
#[derive(ApplyJsonSchema, Debug, Serialize, Deserialize)]
pub struct ProjectView {
    #[property(
        description = "The project's prefix, e.g. `RMS`. This is how a project is named in every other tool, and the first half of every task id on it"
    )]
    pub prefix: String,
    #[property(description = "What the project is called")]
    pub name: String,
    #[property(description = "What the project is about, as the person who set it up wrote it")]
    pub description: String,
    #[property(
        description = "The board's columns in order, `todo` first and `done` last. A task's `status` must be one of these ids"
    )]
    pub columns: Vec<ColumnView>,
    #[property(
        description = "The kinds of work configured for this project. May be empty — a kind is optional on a task"
    )]
    pub kinds: Vec<KindView>,
    #[property(
        description = "Every label currently in use on this project's tasks, sorted. Not a fixed list: a label exists exactly as long as a task wears it, and a new one is created simply by putting it on a task"
    )]
    pub labels: Vec<String>,
    #[property(
        description = "Emails of the people who may see this board. This is the list to resolve a spoken name against before putting it in `assignee`"
    )]
    pub members: Vec<String>,
    #[property(description = "How many tasks are on the board, in every column")]
    pub tasks_amount: i32,
    #[property(
        description = "The goals of this project that are still open, oldest first. Work here is organised BY GOAL: a goal is the container a conversation happens at and tasks come out of, and this is where you find the ids to pass as `goal`. Closed goals are left out — goals_list with include_archived brings the history back"
    )]
    pub goals: Vec<GoalView>,
}

impl ProjectView {
    pub fn from_model(project: &ProjectModel, board: &BoardInner) -> Self {
        let mut columns = Vec::with_capacity(project.columns.len() + 2);

        columns.push(ColumnView {
            id: task_manager_shared::projects::COLUMN_ID_TODO.to_string(),
            name: "Todo".to_string(),
            description: "Not started. Also where a task with an unrecognised status shows up"
                .to_string(),
        });

        for column in &project.columns {
            columns.push(ColumnView {
                id: column.id.clone(),
                name: column.name.clone(),
                description: column.description.clone(),
            });
        }

        columns.push(ColumnView {
            id: task_manager_shared::projects::COLUMN_ID_DONE.to_string(),
            name: "Done".to_string(),
            description: "Landed. A dependency counts as satisfied only when it is here"
                .to_string(),
        });

        Self {
            prefix: project.prefix.clone(),
            name: project.name.clone(),
            description: project.description.clone(),
            columns,
            kinds: project
                .kinds
                .iter()
                .map(|kind| KindView {
                    id: kind.id.clone(),
                    name: kind.name.clone(),
                    description: kind.description.clone(),
                    color: kind.color.as_str().to_string(),
                })
                .collect(),
            labels: board.labels_of_project(&project.id),
            members: project.members.iter().cloned().collect(),
            tasks_amount: board.tasks_amount(&project.id) as i32,
            // Open ones only. A project that has run for a year would otherwise answer the very first
            // call with every epic it has ever finished, and the one thing this list is for is telling a
            // caller where to put the work it is about to create.
            goals: board
                .goals_of_project(&project.id)
                .iter()
                .filter(|goal| !goal.is_closed())
                .map(|goal| GoalView::from_model(goal, project, board))
                .collect(),
        }
    }
}

/// One goal, as a tool sees it.
///
/// The container work is done around, and the level a conversation happens at: a goal is discussed, tasks
/// come out of the discussion, and the resolution goes back onto it when it lands.
#[derive(ApplyJsonSchema, Debug, Serialize, Deserialize)]
pub struct GoalView {
    #[property(
        description = "The goal id, like `RMS-G7`. This is how a goal is named everywhere — in `goal` on a task, in goals_update, in tasks_list. The number comes from the same counter task numbers come from, so `RMS-7` and `RMS-G7` are never both real; the `G` is what says which kind you are holding"
    )]
    pub id: String,
    #[property(description = "The prefix of the project this goal is on")]
    pub project: String,
    #[property(description = "What the goal is called")]
    pub name: String,
    #[property(description = "What it is about, as Markdown")]
    pub description: String,
    #[property(
        description = "The palette colour the board marks this goal's work with. Visual only — but worth reading before colouring a new goal, so two goals on one board are not the same colour"
    )]
    pub color: String,
    #[property(
        description = "`todo` while the goal is open, `done` once it is closed. Derived from whether it has been closed, so it cannot disagree with `closed_unix_seconds` — a goal has these two states and nothing in between in this version"
    )]
    pub status: String,
    #[property(
        description = "How many tasks are part of this goal, INCLUDING work already archived off the board. Do not recompute this from tasks_list: that leaves archived work out, and an old goal would read as half-done"
    )]
    pub tasks_amount: i32,
    #[property(description = "How many of those tasks are done. The goal is closable when it equals `tasks_amount`")]
    pub done_amount: i32,
    #[property(
        description = "How many notes are on the goal's thread. This is where the reasoning lives — read it with goals_get_comments before acting on a goal somebody else shaped"
    )]
    pub comments_amount: i32,
    #[property(description = "When the goal was created, unix seconds (UTC)")]
    pub created_unix_seconds: i64,
    #[property(description = "When the goal itself last changed, unix seconds (UTC). A comment does not move this")]
    pub updated_unix_seconds: i64,
    #[property(
        description = "When it was closed, unix seconds (UTC), and absent while it is open. A goal closed longer ago than the project's archive window is left out of goals_list unless you ask for it"
    )]
    pub closed_unix_seconds: Option<i64>,
}

impl GoalView {
    pub fn from_model(goal: &GoalModel, project: &ProjectModel, board: &BoardInner) -> Self {
        let (tasks_amount, done_amount) = board.goal_progress(&goal.project_id, goal.number);

        Self {
            id: crate::board::compose_goal_handle(&project.prefix, goal.number),
            project: project.prefix.clone(),
            name: goal.name.clone(),
            description: goal.description.clone(),
            color: rust_extensions::AsStr::as_str(&goal.color).to_string(),
            status: goal.status().to_string(),
            tasks_amount: tasks_amount as i32,
            done_amount: done_amount as i32,
            comments_amount: goal.comments.len() as i32,
            created_unix_seconds: goal.created.unix_microseconds / 1_000_000,
            updated_unix_seconds: goal.updated.unix_microseconds / 1_000_000,
            closed_unix_seconds: goal
                .close_moment
                .map(|itm| itm.unix_microseconds / 1_000_000),
        }
    }
}

/// One comment on a task's thread.
#[derive(ApplyJsonSchema, Debug, Serialize, Deserialize)]
pub struct CommentView {
    #[property(description = "When it was left, unix seconds (UTC)")]
    pub moment_unix_seconds: i64,
    #[property(description = "Who left it — an email, or `AI`")]
    pub who: String,
    #[property(description = "The comment, as Markdown")]
    pub text: String,
}

/// One task, as a tool sees it.
#[derive(ApplyJsonSchema, Debug, Serialize, Deserialize)]
pub struct TaskView {
    #[property(
        description = "The task id, like `RMS-000042`. This is the only way to name a task — never its text, since two tasks may read alike. `RMS-42` is accepted wherever an id is taken"
    )]
    pub id: String,
    #[property(description = "The prefix of the project this task is on")]
    pub project: String,
    #[property(description = "What the task says, as Markdown")]
    pub text: String,
    #[property(
        description = "Which column it sits in. A task whose stored status names a column the project no longer has reads as `todo`"
    )]
    pub status: String,
    #[property(description = "What kind of work it is, or absent when it has no kind")]
    pub kind: Option<String>,
    #[property(
        description = "The goal this task is part of, by goal id — `RMS-G7`. Absent means the task stands on its own, which is a normal state and not an unfinished one. Work is organised by goal: when you are asked what is happening with something, this is the thread to pull"
    )]
    pub goal: Option<String>,
    #[property(description = "What that goal is called, absent for a standalone task")]
    pub goal_name: Option<String>,
    #[property(description = "Who is on it — an email, or `AI`. Absent means nobody yet")]
    pub assignee: Option<String>,
    #[property(
        description = "The assignee's name, when they are on the roster. `AI` resolves to itself; absent for an email with no user"
    )]
    pub assignee_name: Option<String>,
    #[property(description = "Tags on this task, lowercased and sorted")]
    pub labels: Vec<String>,
    #[property(
        description = "Ids of the tasks blocking this one. Always tasks of the same project — dependencies do not cross projects"
    )]
    pub depends_on: Vec<String>,
    #[property(
        description = "THE OTHER DIRECTION, and derived rather than stored: ids of the tasks that name THIS one in their `depends_on`, so finishing this task is what unblocks them. Check it before parking or re-scoping something: `depends_on` says what is in your way, `blocks` says who is waiting on you, and only the second is invisible from the task itself"
    )]
    pub blocks: Vec<String>,
    #[property(
        description = "Derived, not stored: true while any task in `depends_on` is not `done` — including an id matching no task at all, so a mistyped or deleted blocker keeps the task blocked rather than silently freeing it. Do NOT start a blocked task"
    )]
    pub blocked: bool,
    #[property(description = "How many comments are on the thread")]
    pub comments_amount: i32,
    #[property(description = "When the task was created, unix seconds (UTC)")]
    pub created_unix_seconds: i64,
    #[property(
        description = "When the task itself last changed, unix seconds (UTC). A comment does not move this — the thread is a separate record from the work"
    )]
    pub updated_unix_seconds: i64,
    #[property(
        description = "When the task landed in `done`, unix seconds (UTC), and absent whenever it is not there. Work closed more than seven days ago is archived and left out of tasks_list unless you ask for it"
    )]
    pub closed_unix_seconds: Option<i64>,
}

impl TaskView {
    pub fn from_model(task: &TaskModel, project: &ProjectModel, board: &BoardInner) -> Self {
        let goal = board.effective_goal(task);

        Self {
            id: compose_task_handle(&project.prefix, task.number),
            project: project.prefix.clone(),
            text: task.text.clone(),
            status: project.effective_status(&task.status),
            kind: project.effective_kind(task.kind.as_deref()),
            // Resolved rather than echoed, so a number naming no goal reads as standalone instead of as
            // an id the caller cannot look up.
            goal: goal
                .as_ref()
                .map(|itm| crate::board::compose_goal_handle(&project.prefix, itm.number)),
            goal_name: goal.as_ref().map(|itm| itm.name.clone()),
            assignee: task.assignee.clone(),
            assignee_name: task
                .assignee
                .as_ref()
                .and_then(|itm| board.display_name_of(itm)),
            labels: task.labels.clone(),
            depends_on: task
                .depends_on
                .iter()
                .map(|number| compose_task_handle(&project.prefix, *number))
                .collect(),
            blocks: board
                .blocks(&task.project_id, task.number)
                .iter()
                .map(|number| compose_task_handle(&project.prefix, *number))
                .collect(),
            blocked: board.is_blocked(task),
            comments_amount: task.comments.len() as i32,
            created_unix_seconds: task.created.unix_microseconds / 1_000_000,
            updated_unix_seconds: task.updated.unix_microseconds / 1_000_000,
            closed_unix_seconds: task
                .close_moment
                .map(|itm| itm.unix_microseconds / 1_000_000),
        }
    }
}

/// One person on the roster.
#[derive(ApplyJsonSchema, Debug, Serialize, Deserialize)]
pub struct UserView {
    #[property(
        description = "The email. THIS is what goes into a task's `assignee` and a comment's `who` — never the name"
    )]
    pub email: String,
    #[property(
        description = "How this person is called in conversation. Match a spoken first name against this to find the email"
    )]
    pub name: String,
    #[property(
        description = "True when this person can no longer sign in. Do not assign new work to them; their existing tasks and comments are left as they are"
    )]
    pub disabled: bool,
    #[property(
        description = "True for the one entry that is not a person: the reserved assignee AI, meaning an agent does this task. Assignable on every board and never disabled. Put it in `assignee` when the work is yours"
    )]
    pub reserved: bool,
}

impl UserView {
    pub fn from_model(user: &UserModel) -> Self {
        Self {
            email: user.email.clone(),
            name: user.name.clone(),
            disabled: user.disabled,
            reserved: false,
        }
    }

    /// The reserved assignee, which has no user row to build from.
    pub fn ai() -> Self {
        Self {
            email: task_manager_shared::users::ASSIGNEE_AI.to_string(),
            name: "AI".to_string(),
            disabled: false,
            reserved: true,
        }
    }
}
