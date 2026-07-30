use std::sync::Arc;

use mcp_server_middleware::*;
use serde::{Deserialize, Serialize};

use crate::app::AppContext;
use crate::mcp::{CommentView, GoalView, SubtaskEditInput, SubtaskInput, SubtaskOps};
use crate::scripts::{GoalPatch, NewGoal};

/// What every goal write returns: the goal as it now stands.
///
/// The whole goal rather than an "ok", because the progress counters are derived and a caller that just
/// landed a task wants to see whether the goal became closable.
#[derive(ApplyJsonSchema, Debug, Serialize, Deserialize)]
pub struct GoalWriteResponse {
    #[property(description = "The goal as it now stands")]
    pub goal: GoalView,
}

async fn read_back(app: &AppContext, handle: &str) -> Result<GoalWriteResponse, String> {
    let board = app.board.read();
    let resolved = crate::scripts::resolve_goal_by_handle(&board, handle)?;

    Ok(GoalWriteResponse {
        goal: GoalView::from_model(&resolved.goal, &resolved.project, &board),
    })
}

// -------------------------------------------------------------------------------------------- list

#[derive(ApplyJsonSchema, Debug, Serialize, Deserialize)]
pub struct GoalsListInput {
    #[property(description = "Which board to read, by project prefix, e.g. `RMS`")]
    pub project: String,
    #[property(
        description = "Include goals closed longer ago than the project's archive window. Omitted gives the live list, which is almost always what you want; pass true when you are deliberately looking back at finished epics"
    )]
    pub include_archived: Option<bool>,
}

#[derive(ApplyJsonSchema, Debug, Serialize, Deserialize)]
pub struct GoalsListResponse {
    #[property(description = "The goals, oldest first")]
    pub goals: Vec<GoalView>,
    #[property(description = "Number of rows in `goals`")]
    pub amount: i32,
}

pub struct GoalsListHandler {
    app: Arc<AppContext>,
}

impl GoalsListHandler {
    pub fn new(app: Arc<AppContext>) -> Self {
        Self { app }
    }
}

impl ToolDefinition for GoalsListHandler {
    const FUNC_NAME: &'static str = "goals_list";
    const DESCRIPTION: &'static str = "The goals of one project — the containers work is organised \
around. Read this before creating or picking up work: a task usually belongs under a goal, and this is \
where the `goal` ids come from. Each goal reports its progress as `done_amount` of `tasks_amount`, \
counting archived work, so a finished epic reads as finished rather than as half-done.\
\
Goals closed longer ago than the project's archive window are left out unless you ask for them. A \
project with no goals is a legitimate answer, not an error — tasks may stand on their own.";
}

#[async_trait::async_trait]
impl McpToolCall<GoalsListInput, GoalsListResponse> for GoalsListHandler {
    async fn execute_tool_call(&self, model: GoalsListInput) -> Result<GoalsListResponse, String> {
        let board = self.app.board.read();
        let project = crate::scripts::resolve_project_by_prefix(&board, &model.project)?;

        let include_archived = model.include_archived.unwrap_or(false);

        let goals: Vec<GoalView> = board
            .goals_of_project(&project.id)
            .iter()
            .filter(|goal| include_archived || !board.is_goal_archived(goal))
            .map(|goal| GoalView::from_model(goal, &project, &board))
            .collect();

        Ok(GoalsListResponse {
            amount: goals.len() as i32,
            goals,
        })
    }
}

// ------------------------------------------------------------------------------------------ create

#[derive(ApplyJsonSchema, Debug, Serialize, Deserialize)]
pub struct GoalsCreateInput {
    #[property(description = "Which board to put it on, by project prefix, e.g. `RMS`")]
    pub project: String,
    #[property(
        description = "What the goal is called. A short line naming the outcome, not a description of the work — the tasks under it are the work"
    )]
    pub name: String,
    #[property(
        description = "What the goal is about, as Markdown. Where the shape of it goes: what is in scope, what is not, what it depends on. The reasoning as it develops belongs on the thread instead, with goals_add_comment"
    )]
    pub description: Option<String>,
    #[property(
        description = "A palette colour: gray, red, orange, amber, green, teal, blue or purple. Purely visual — it is how the board marks which goal a card belongs to, so pick one that is not already in use on that project. Omit for gray; a person can also recolour it in the browser"
    )]
    pub color: Option<String>,
    #[property(
        description = "A checklist for the goal itself, each item a `title` and optionally a longer `text`. For the small things an epic drags along that are not worth a card — NOT for the work, which is tasks under the goal. An unticked item does not hold the goal open. Usually omitted"
    )]
    pub subtasks: Option<Vec<SubtaskInput>>,
}

pub struct GoalsCreateHandler {
    app: Arc<AppContext>,
}

impl GoalsCreateHandler {
    pub fn new(app: Arc<AppContext>) -> Self {
        Self { app }
    }
}

impl ToolDefinition for GoalsCreateHandler {
    const FUNC_NAME: &'static str = "goals_create";
    const DESCRIPTION: &'static str = "Open a goal — a container for work, an epic. Use it when a \
conversation is about an outcome rather than a single task: create the goal, keep the reasoning on its \
thread with goals_add_comment, and create the tasks under it as they become clear.\
\
Read goals_list first: this creates a goal unconditionally, and two goals for the same outcome split \
the work in half where a person reads it by eye. A goal always starts open — there is no state to pass, \
and closing it is goals_update's job, which is where the resolution has to be written.";
}

#[async_trait::async_trait]
impl McpToolCall<GoalsCreateInput, GoalWriteResponse> for GoalsCreateHandler {
    async fn execute_tool_call(&self, model: GoalsCreateInput) -> Result<GoalWriteResponse, String> {
        let handle = crate::scripts::create_goal(
            &self.app,
            NewGoal {
                project_prefix: model.project,
                name: model.name,
                description: model.description.unwrap_or_default(),
                color: model.color,
                subtasks: SubtaskInput::into_new(model.subtasks),
            },
        )
        .await?;

        read_back(&self.app, &handle).await
    }
}

// ------------------------------------------------------------------------------------------ update

#[derive(ApplyJsonSchema, Debug, Serialize, Deserialize)]
pub struct GoalsUpdateInput {
    #[property(description = "Which goal to change, by id, e.g. `RMS-G7`")]
    pub id: String,
    #[property(description = "Rename it. Omit to leave the name alone")]
    pub name: Option<String>,
    #[property(description = "Rewrite what the goal is about, as Markdown. Omit to leave it alone")]
    pub description: Option<String>,
    #[property(
        description = "Recolour it: gray, red, orange, amber, green, teal, blue or purple. Visual only — the colour the board marks this goal's cards with. Omit to leave it alone"
    )]
    pub color: Option<String>,
    #[property(
        description = "Pass true to CLOSE the goal, which is only allowed once every one of its tasks is `done` and always requires `comment` — the resolution. Pass false to re-open a closed goal. Omit to leave its state alone"
    )]
    pub close: Option<bool>,
    #[property(
        description = "Checklist items to add to the goal, each a `title` and optionally a longer `text`. Added to what it already carries. For the goal's own loose ends — the work belongs in tasks under it"
    )]
    pub add_subtasks: Option<Vec<SubtaskInput>>,
    #[property(
        description = "TICK the goal's checklist items off, by the `id` each one reports. An id that names no item is refused rather than ignored"
    )]
    pub check_subtasks: Option<Vec<String>>,
    #[property(
        description = "Un-tick items, by id. Applied after check_subtasks, so an id passed to both ends up unticked"
    )]
    pub uncheck_subtasks: Option<Vec<String>>,
    #[property(
        description = "Reword items in place: each entry an `id` plus the `title` and/or `text` to replace. Keeps the item's id and its place in the list"
    )]
    pub edit_subtasks: Option<Vec<SubtaskEditInput>>,
    #[property(
        description = "Take items off the list, by id. Applied last. A done item is normally left ticked rather than removed — it is the record of what the goal involved"
    )]
    pub remove_subtasks: Option<Vec<String>>,
    #[property(
        description = "A note for the goal's thread as part of this same change, as Markdown. REQUIRED when closing, where it is the resolution: what came of the goal and anything the next person should know. Optional otherwise"
    )]
    pub comment: Option<String>,
    #[property(
        description = "Who the comment is from: an email, or the literal `AI`. Required whenever `comment` is passed"
    )]
    pub comment_by: Option<String>,
}

pub struct GoalsUpdateHandler {
    app: Arc<AppContext>,
}

impl GoalsUpdateHandler {
    pub fn new(app: Arc<AppContext>) -> Self {
        Self { app }
    }
}

impl ToolDefinition for GoalsUpdateHandler {
    const FUNC_NAME: &'static str = "goals_update";
    const DESCRIPTION: &'static str = "Rename a goal, rewrite it, or close it. Only the fields you \
pass change.\
\
CLOSING A GOAL HAS TWO CONDITIONS, and both are enforced. Every task under it must be `done` — the \
call is refused with the unfinished ones named, and the fix is to land them or take them out of the \
goal. And it needs `comment` (with `comment_by`): the resolution, which is what somebody reads months \
later to find out how the goal went. A closed goal leaves the screen once the project's archive window \
has passed and is then reachable by its id.\
\
There is NO delete. A goal that ran its course is closed; one that should never have existed stays \
until deletion exists.";
}

#[async_trait::async_trait]
impl McpToolCall<GoalsUpdateInput, GoalWriteResponse> for GoalsUpdateHandler {
    async fn execute_tool_call(&self, model: GoalsUpdateInput) -> Result<GoalWriteResponse, String> {
        let handle = crate::scripts::update_goal(
            &self.app,
            &model.id,
            GoalPatch {
                name: model.name,
                description: model.description,
                color: model.color,
                close: model.close,
                subtasks: SubtaskOps {
                    add: model.add_subtasks,
                    edit: model.edit_subtasks,
                    check: model.check_subtasks,
                    uncheck: model.uncheck_subtasks,
                    remove: model.remove_subtasks,
                }
                .into_patch(),
                comment: model.comment,
                comment_by: model.comment_by,
            },
        )
        .await?;

        read_back(&self.app, &handle).await
    }
}

// ---------------------------------------------------------------------------------------- comments

#[derive(ApplyJsonSchema, Debug, Serialize, Deserialize)]
pub struct GoalsAddCommentInput {
    #[property(description = "Which goal to add to, by id, e.g. `RMS-G7`")]
    pub id: String,
    #[property(
        description = "Who it is from: an email from users_list, or the literal `AI` when it is an agent's own note"
    )]
    pub who: String,
    #[property(
        description = "The note, as Markdown. What was decided, what was learned, what was ruled out — the reasoning behind the work rather than the work itself"
    )]
    pub text: String,
}

#[derive(ApplyJsonSchema, Debug, Serialize, Deserialize)]
pub struct GoalsGetCommentsInput {
    #[property(description = "Which goal's thread to read, by id, e.g. `RMS-G7`")]
    pub id: String,
}

#[derive(ApplyJsonSchema, Debug, Serialize, Deserialize)]
pub struct GoalCommentsResponse {
    #[property(description = "The goal the thread belongs to, by id")]
    pub id: String,
    #[property(description = "The notes, oldest first")]
    pub comments: Vec<CommentView>,
    #[property(description = "Number of rows in `comments`")]
    pub amount: i32,
}

pub struct GoalsAddCommentHandler {
    app: Arc<AppContext>,
}

impl GoalsAddCommentHandler {
    pub fn new(app: Arc<AppContext>) -> Self {
        Self { app }
    }
}

impl ToolDefinition for GoalsAddCommentHandler {
    const FUNC_NAME: &'static str = "goals_add_comment";
    const DESCRIPTION: &'static str = "Put a note on a goal's thread. THIS IS WHERE A CONVERSATION \
ABOUT WORK BELONGS: what was decided and why, what was tried, what was ruled out, what is still open. \
The goal's own text says what the outcome is; the thread says how the thinking got there, and it is \
what makes the tasks underneath make sense to whoever picks them up.\
\
A comment does not move the goal's `updated` — a busy thread does not make a goal look like changing \
work.";
}

#[async_trait::async_trait]
impl McpToolCall<GoalsAddCommentInput, GoalCommentsResponse> for GoalsAddCommentHandler {
    async fn execute_tool_call(
        &self,
        model: GoalsAddCommentInput,
    ) -> Result<GoalCommentsResponse, String> {
        let handle =
            crate::scripts::add_goal_comment(&self.app, &model.id, &model.who, &model.text).await?;

        read_comments(&self.app, &handle)
    }
}

pub struct GoalsGetCommentsHandler {
    app: Arc<AppContext>,
}

impl GoalsGetCommentsHandler {
    pub fn new(app: Arc<AppContext>) -> Self {
        Self { app }
    }
}

impl ToolDefinition for GoalsGetCommentsHandler {
    const FUNC_NAME: &'static str = "goals_get_comments";
    const DESCRIPTION: &'static str = "Read a goal's thread, oldest first. Worth reading whenever \
`comments_amount` is not zero and you are about to work on the goal or on a task under it: the reason \
the work is shaped the way it is usually lives here rather than in the goal's text.";
}

#[async_trait::async_trait]
impl McpToolCall<GoalsGetCommentsInput, GoalCommentsResponse> for GoalsGetCommentsHandler {
    async fn execute_tool_call(
        &self,
        model: GoalsGetCommentsInput,
    ) -> Result<GoalCommentsResponse, String> {
        read_comments(&self.app, &model.id)
    }
}

fn read_comments(app: &AppContext, handle: &str) -> Result<GoalCommentsResponse, String> {
    let board = app.board.read();
    let resolved = crate::scripts::resolve_goal_by_handle(&board, handle)?;

    let comments: Vec<CommentView> = resolved
        .goal
        .comments
        .iter()
        .map(|itm| CommentView {
            moment_unix_seconds: itm.moment.unix_microseconds / 1_000_000,
            who: itm.who.clone(),
            text: itm.text.clone(),
        })
        .collect();

    Ok(GoalCommentsResponse {
        id: crate::board::compose_goal_handle(&resolved.project.prefix, resolved.goal.number),
        amount: comments.len() as i32,
        comments,
    })
}
