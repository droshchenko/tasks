use std::sync::Arc;

use mcp_server_middleware::*;
use serde::{Deserialize, Serialize};

use crate::app::AppContext;
use crate::mcp::TaskView;
use crate::scripts::{NewTask, TaskPatch};

/// What every write tool returns: the task as it now stands.
///
/// Returning the whole task rather than an "ok" means the caller sees the consequences of its own
/// write — most usefully `blocked`, which the write may have changed for other tasks too.
#[derive(ApplyJsonSchema, Debug, Serialize, Deserialize)]
pub struct TaskWriteResponse {
    #[property(description = "The task as it now stands")]
    pub task: TaskView,
}

async fn read_back(app: &AppContext, handle: &str) -> Result<TaskWriteResponse, String> {
    let board = app.board.read();
    let resolved = crate::scripts::resolve_task(&board, handle)?;

    Ok(TaskWriteResponse {
        task: TaskView::from_model(&resolved.task, &resolved.project, &board),
    })
}

// ------------------------------------------------------------------------------------------ create

#[derive(ApplyJsonSchema, Debug, Serialize, Deserialize)]
pub struct TasksCreateInput {
    #[property(description = "Which board to put it on, by project prefix, e.g. `RMS`")]
    pub project: String,
    #[property(
        description = "What the task says, as Markdown — the board renders it. One task, one piece of work: write the thing to be done, not the topic. **bold** for the headline, `code` for identifiers, paths and commands, `-` bullets for a short checklist. Keep it to a sticker's worth — a line or two, not a document"
    )]
    pub text: String,
    #[property(
        description = "What kind of work it is, by kind id. Read the kind descriptions from projects_list first — the meanings are per project. Omit for no kind"
    )]
    pub kind: Option<String>,
    #[property(
        description = "Who takes it: an email from users_list, or the literal `claude` when this is Claude's to build. Never a first name — a name here names nobody. Omit to leave it unassigned"
    )]
    pub assignee: Option<String>,
    #[property(
        description = "Tags to put on it. Reuse what the project already uses (projects_list returns them) rather than coining a near-duplicate, which splits one idea across two tags and makes both filters lie. A tag that does not exist yet is created simply by using it"
    )]
    pub labels: Option<Vec<String>>,
    #[property(
        description = "Ids of the tasks that block this one, e.g. [\"RMS-7\"]. Bare numbers work too. Must be tasks of the same project. The task then reads as `blocked` and must not be started until every one of them is `done`"
    )]
    pub depends_on: Option<Vec<String>>,
}

pub struct TasksCreateHandler {
    app: Arc<AppContext>,
}

impl TasksCreateHandler {
    pub fn new(app: Arc<AppContext>) -> Self {
        Self { app }
    }
}

impl ToolDefinition for TasksCreateHandler {
    const FUNC_NAME: &'static str = "tasks_create";
    const DESCRIPTION: &'static str = "Put a new task on a board. Use it when you are asked to \
remember a piece of work, or when work is agreed that nobody is doing yet. Read the board first: this \
creates a task unconditionally, so calling it twice for the same work leaves two of them. The \
returned `id` is how the task is named from then on.\
\
A new task ALWAYS starts in `todo` — there is no status to pass. Moving it on is tasks_update's job, \
which is also where landing work has to say what was done.";
}

#[async_trait::async_trait]
impl McpToolCall<TasksCreateInput, TaskWriteResponse> for TasksCreateHandler {
    async fn execute_tool_call(
        &self,
        model: TasksCreateInput,
    ) -> Result<TaskWriteResponse, String> {
        let handle = crate::scripts::create_task(
            &self.app,
            NewTask {
                project_prefix: model.project,
                text: model.text,
                kind: model.kind,
                assignee: model.assignee,
                labels: model.labels.unwrap_or_default(),
                depends_on: model.depends_on.unwrap_or_default(),
            },
        )
        .await?;

        read_back(&self.app, &handle).await
    }
}

// ------------------------------------------------------------------------------------------ update

#[derive(ApplyJsonSchema, Debug, Serialize, Deserialize)]
pub struct TasksUpdateInput {
    #[property(
        description = "Which task to change, by id, e.g. `RMS-42`. Not its text — two tasks may read alike"
    )]
    pub id: String,
    #[property(
        description = "Move it to this column, by column id. This is what to call when work starts and when it lands. Do not move a `blocked` task out of a waiting column until its blockers are done"
    )]
    pub status: Option<String>,
    #[property(
        description = "Rewrite what the task says, as Markdown. Omit to leave the text alone"
    )]
    pub text: Option<String>,
    #[property(
        description = "Reclassify it, by kind id. Pass an empty string to clear the kind; omit to leave it alone"
    )]
    pub kind: Option<String>,
    #[property(
        description = "Reassign it — an email, or `claude`. Pass an empty string to clear the assignee; omit to leave it as it is"
    )]
    pub assignee: Option<String>,
    #[property(
        description = "Tags to add. Added to what the task already carries, so you need not know the current set"
    )]
    pub add_labels: Option<Vec<String>>,
    #[property(
        description = "Tags to take off. Applied after add_labels, so a tag passed to both ends up removed. A tag that is not there is not an error"
    )]
    pub remove_labels: Option<Vec<String>>,
    #[property(
        description = "Replace the WHOLE set of blocking task ids. Pass [] to clear every dependency; omit to leave them untouched"
    )]
    pub depends_on: Option<Vec<String>>,
    #[property(
        description = "A note to put on the task's thread as part of this same change, as Markdown. Optional in general — and REQUIRED when this change moves the task to `done`, where it has to say what was actually done. Write a line or two: what changed, and anything the next person should know"
    )]
    pub comment: Option<String>,
    #[property(
        description = "Who the comment is from: an email, or the literal `claude`. Required whenever `comment` is passed — there is no session here to derive an author from"
    )]
    pub comment_by: Option<String>,
}

pub struct TasksUpdateHandler {
    app: Arc<AppContext>,
}

impl TasksUpdateHandler {
    pub fn new(app: Arc<AppContext>) -> Self {
        Self { app }
    }
}

impl ToolDefinition for TasksUpdateHandler {
    const FUNC_NAME: &'static str = "tasks_update";
    const DESCRIPTION: &'static str = "Move a task between columns, rewrite it, or change who is on \
it. This is what to call when work starts and when it lands. Only the fields you pass change; \
omitted ones keep their value. An update that would change nothing is refused rather than quietly \
doing nothing — which is almost always a status that was meant to be passed and was not.\
\
MOVING A TASK TO `done` REQUIRES A COMMENT saying what was actually done — pass `comment` and \
`comment_by` in the same call. Without one the move is refused. The Done column is what the board is \
worth reading for later, and \"moved to done\" on its own records nothing. Only the transition into \
Done needs this: editing a task that is already there does not.";
}

#[async_trait::async_trait]
impl McpToolCall<TasksUpdateInput, TaskWriteResponse> for TasksUpdateHandler {
    async fn execute_tool_call(
        &self,
        model: TasksUpdateInput,
    ) -> Result<TaskWriteResponse, String> {
        let handle = crate::scripts::update_task(
            &self.app,
            &model.id,
            TaskPatch {
                text: model.text,
                status: model.status,
                kind: model.kind,
                assignee: model.assignee,
                add_labels: model.add_labels.unwrap_or_default(),
                remove_labels: model.remove_labels.unwrap_or_default(),
                depends_on: model.depends_on,
                comment: model.comment,
                comment_by: model.comment_by,
            },
        )
        .await?;

        read_back(&self.app, &handle).await
    }
}

// ------------------------------------------------------------------------------------------ delete

#[derive(ApplyJsonSchema, Debug, Serialize, Deserialize)]
pub struct TasksDeleteInput {
    #[property(
        description = "Which task to remove, by id. Confirm with the user first — this cannot be undone"
    )]
    pub id: String,
}

#[derive(ApplyJsonSchema, Debug, Serialize, Deserialize)]
pub struct TasksDeleteResponse {
    #[property(description = "The id of the task that was removed")]
    pub id: String,
}

pub struct TasksDeleteHandler {
    app: Arc<AppContext>,
}

impl TasksDeleteHandler {
    pub fn new(app: Arc<AppContext>) -> Self {
        Self { app }
    }
}

impl ToolDefinition for TasksDeleteHandler {
    const FUNC_NAME: &'static str = "tasks_delete";
    const DESCRIPTION: &'static str = "Take a task off a board for good. Finished work is NOT \
deleted — it moves to `done` with tasks_update, which is what keeps a board readable as a history. \
Delete only what should never have been there. The number is not reused, so ids stay stable.";
}

#[async_trait::async_trait]
impl McpToolCall<TasksDeleteInput, TasksDeleteResponse> for TasksDeleteHandler {
    async fn execute_tool_call(
        &self,
        model: TasksDeleteInput,
    ) -> Result<TasksDeleteResponse, String> {
        let id = crate::scripts::delete_task(&self.app, &model.id).await?;

        Ok(TasksDeleteResponse { id })
    }
}
