use super::TaskView;
use crate::app::AppContext;
use crate::intelligence::semantic::KnowledgeContextHit;
use crate::scripts::{DecisionPatch, TaskPatch};
use mcp_server_middleware::*;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use task_manager_shared::decisions::{
    DecisionAnswer, DecisionChoice, DecisionRequest, TaskDecision,
};
use task_manager_shared::execution_prompts::ResolvedExecutionPrompt;

#[derive(ApplyJsonSchema, Debug, Serialize, Deserialize)]
pub struct ExecutionPromptView {
    #[property(
        description = "Whether this stage exit or task-type completion requires analysis file references"
    )]
    pub requires_analysis_documents: bool,
    #[property(description = "The selected fragment scope: column or task-type")]
    pub scope: String,
    #[property(description = "The template supplying this instruction")]
    pub template_id: String,
    #[property(description = "The selected column or task-type id")]
    pub target: String,
    #[property(description = "The exact text, preserved as supplied")]
    pub text: String,
    #[property(description = "SHA-256 of the selected instruction and its scope")]
    pub version: String,
}
impl From<ResolvedExecutionPrompt> for ExecutionPromptView {
    fn from(prompt: ResolvedExecutionPrompt) -> Self {
        Self {
            scope: prompt.scope,
            template_id: prompt.template_id,
            target: prompt.target,
            text: prompt.text,
            version: prompt.version,
            requires_analysis_documents: prompt.requires_analysis_documents,
        }
    }
}

#[derive(ApplyJsonSchema, Debug, Serialize, Deserialize)]
pub struct DecisionChoiceView {
    #[property(description = "The task or decision identifier named by this object")]
    pub id: String,
    #[property(description = "The human-visible choice label")]
    pub label: String,
    #[property(description = "What selecting this choice would do")]
    pub consequence: String,
    #[property(description = "A recommendation only, never a submitted human answer")]
    pub recommended: bool,
}
#[derive(ApplyJsonSchema, Debug, Serialize, Deserialize)]
pub struct DecisionView {
    #[property(description = "The task or decision identifier named by this object")]
    pub id: String,
    #[property(description = "request key")]
    pub request_key: String,
    #[property(description = "The exact proposed action and its scope")]
    pub action: String,
    #[property(description = "The question as presented to the person")]
    pub question: String,
    #[property(
        description = "The original offered choices; an empty list allows a free-text reply"
    )]
    pub options: Vec<DecisionChoiceView>,
    #[property(description = "Whether this pending question prevents task completion")]
    pub required: bool,
    #[property(description = "The recorded question state: pending, answered or cancelled")]
    pub status: String,
    #[property(description = "Who asked the question")]
    pub asked_by: String,
    #[property(description = "Server-recorded question time in UTC unix seconds")]
    pub asked_unix_seconds: i64,
    #[property(description = "answer option id")]
    pub answer_option_id: Option<String>,
    #[property(description = "answer text")]
    pub answer_text: Option<String>,
    #[property(description = "answered by")]
    pub answered_by: Option<String>,
    #[property(description = "answered unix seconds")]
    pub answered_unix_seconds: Option<i64>,
    #[property(description = "answer source")]
    pub answer_source: Option<String>,
    #[property(description = "cancel reason")]
    pub cancel_reason: Option<String>,
    #[property(description = "cancelled by")]
    pub cancelled_by: Option<String>,
    #[property(description = "cancelled unix seconds")]
    pub cancelled_unix_seconds: Option<i64>,
}
impl From<&TaskDecision> for DecisionView {
    fn from(decision: &TaskDecision) -> Self {
        Self {
            id: decision.id.clone(),
            request_key: decision.request_key.clone(),
            action: decision.action.clone(),
            question: decision.question.clone(),
            options: decision
                .options
                .iter()
                .map(|choice| DecisionChoiceView {
                    id: choice.id.clone(),
                    label: choice.label.clone(),
                    consequence: choice.consequence.clone(),
                    recommended: choice.recommended,
                })
                .collect(),
            required: decision.required,
            status: decision.status.clone(),
            asked_by: decision.asked_by.clone(),
            asked_unix_seconds: decision.asked_unix_seconds,
            answer_option_id: decision
                .answer
                .as_ref()
                .and_then(|answer| answer.option_id.clone()),
            answer_text: decision.answer.as_ref().map(|answer| answer.text.clone()),
            answered_by: decision
                .answer
                .as_ref()
                .map(|answer| answer.answered_by.clone()),
            answered_unix_seconds: decision
                .answer
                .as_ref()
                .map(|answer| answer.answered_unix_seconds),
            answer_source: decision.answer.as_ref().map(|answer| answer.source.clone()),
            cancel_reason: decision.cancel_reason.clone(),
            cancelled_by: decision.cancelled_by.clone(),
            cancelled_unix_seconds: decision.cancelled_unix_seconds,
        }
    }
}

#[derive(ApplyJsonSchema, Debug, Serialize, Deserialize)]
pub struct TaskWorkflowResponse {
    #[property(description = "The current task state")]
    pub task: TaskView,
    #[property(
        description = "Only the selected stage and task-type instructions, with content versions"
    )]
    pub execution_prompts: Vec<ExecutionPromptView>,
    #[property(
        description = "Immutable questions, original choices, actual answers and cancellations on this task"
    )]
    pub decisions: Vec<DecisionView>,
    #[property(description = "The decision created or resolved by this call, when applicable")]
    pub requested_decision_id: Option<String>,
    #[property(
        description = "Bounded project wiki and Graphify evidence relevant to this task, with source references"
    )]
    pub knowledge: Vec<KnowledgeContextHit>,
    #[property(description = "Project knowledge availability and source freshness")]
    pub knowledge_notice: String,
}
fn read_context(
    app: &AppContext,
    id: &str,
    status: Option<&str>,
    selected: Option<String>,
) -> Result<TaskWorkflowResponse, String> {
    let board = app.board.read();
    let resolved = crate::scripts::resolve_task(&board, id)?;
    let knowledge = crate::intelligence::knowledge::search_project(
        app,
        &resolved.project,
        &crate::intelligence::semantic::clip(&resolved.task.text, 4096),
        5,
    )
    .unwrap_or_else(
        |notice| task_manager_shared::ai_settings::KnowledgePreviewResponse {
            hits: vec![],
            notice,
        },
    );
    let prompts = crate::scripts::resolve_execution_prompts(
        &board,
        &resolved.project,
        &resolved.task,
        status,
    )?;
    Ok(TaskWorkflowResponse {
        task: TaskView::from_model(&resolved.task, &resolved.project, &board),
        execution_prompts: prompts.into_iter().map(Into::into).collect(),
        decisions: resolved.task.decisions.iter().map(Into::into).collect(),
        requested_decision_id: selected,
        knowledge: knowledge.hits.into_iter().map(Into::into).collect(),
        knowledge_notice: knowledge.notice,
    })
}

#[derive(ApplyJsonSchema, Debug, Serialize, Deserialize)]
pub struct PrepareTaskInput {
    #[property(description = "The task to prepare, by its human id")]
    pub id: String,
    #[property(
        description = "Optional intended column. Omit for the task's current column; preparation does not change the task"
    )]
    pub status: Option<String>,
}
pub struct PrepareTaskHandler {
    app: Arc<AppContext>,
}
impl PrepareTaskHandler {
    pub fn new(app: Arc<AppContext>) -> Self {
        Self { app }
    }
}
impl ToolDefinition for PrepareTaskHandler {
    const FUNC_NAME: &'static str = "tasks_prepare";
    const DESCRIPTION: &'static str = "Prepare one task before doing work. Returns only the selected column and task-type prompts with content versions, its decision history and analysis-file references. Read the task's comments and attached specification as needed; use the current stage instructions for the work.";
}
#[async_trait::async_trait]
impl McpToolCall<PrepareTaskInput, TaskWorkflowResponse> for PrepareTaskHandler {
    async fn execute_tool_call(
        &self,
        input: PrepareTaskInput,
    ) -> Result<TaskWorkflowResponse, String> {
        read_context(&self.app, &input.id, input.status.as_deref(), None)
    }
}

#[derive(ApplyJsonSchema, Debug, Serialize, Deserialize)]
pub struct DecisionChoiceInput {
    #[property(description = "The task or decision identifier named by this object")]
    pub id: String,
    #[property(description = "The human-visible choice label")]
    pub label: String,
    #[property(description = "What selecting this choice would do")]
    pub consequence: Option<String>,
    #[property(description = "A recommendation only, never a submitted human answer")]
    pub recommended: Option<bool>,
}
#[derive(ApplyJsonSchema, Debug, Serialize, Deserialize)]
pub struct RequestDecisionInput {
    #[property(description = "The task or decision identifier named by this object")]
    pub id: String,
    #[property(
        description = "Stable key for this question on this task; retries must reuse it. A revised question needs a new key"
    )]
    pub request_key: String,
    #[property(description = "The exact proposed action and its scope")]
    pub action: String,
    #[property(description = "The question as presented to the person")]
    pub question: String,
    #[property(
        description = "The original offered choices; an empty list allows a free-text reply"
    )]
    pub options: Option<Vec<DecisionChoiceInput>>,
    #[property(description = "Whether this pending question prevents task completion")]
    pub required: Option<bool>,
    #[property(description = "Who asked the question")]
    pub asked_by: Option<String>,
}
pub struct RequestDecisionHandler {
    app: Arc<AppContext>,
}
impl RequestDecisionHandler {
    pub fn new(app: Arc<AppContext>) -> Self {
        Self { app }
    }
}
impl ToolDefinition for RequestDecisionHandler {
    const FUNC_NAME: &'static str = "tasks_request_decision";
    const DESCRIPTION: &'static str = "Record a proposed action, question and exact choices BEFORE asking a person in chat. Present the recorded question, then wait for a real answer when required. The suggested option is not consent. Returns the stable decision id; repeated identical request keys do not duplicate history.";
}
#[async_trait::async_trait]
impl McpToolCall<RequestDecisionInput, TaskWorkflowResponse> for RequestDecisionHandler {
    async fn execute_tool_call(
        &self,
        input: RequestDecisionInput,
    ) -> Result<TaskWorkflowResponse, String> {
        let key = input.request_key.clone();
        let request = DecisionRequest {
            request_key: key.clone(),
            action: input.action,
            question: input.question,
            options: input
                .options
                .unwrap_or_default()
                .into_iter()
                .map(|choice| DecisionChoice {
                    id: choice.id,
                    label: choice.label,
                    consequence: choice.consequence.unwrap_or_default(),
                    recommended: choice.recommended.unwrap_or(false),
                })
                .collect(),
            required: input.required.unwrap_or(true),
            asked_by: input.asked_by.unwrap_or_else(|| "AI".into()),
        };
        crate::scripts::update_task(
            &self.app,
            &input.id,
            TaskPatch {
                decision: DecisionPatch::Request(request),
                ..Default::default()
            },
        )
        .await?;
        let board = self.app.board.read();
        let task = crate::scripts::resolve_task(&board, &input.id)?.task;
        let id = task
            .decisions
            .iter()
            .find(|decision| decision.request_key == key.trim())
            .map(|decision| decision.id.clone());
        read_context(&self.app, &input.id, None, id)
    }
}

#[derive(ApplyJsonSchema, Debug, Serialize, Deserialize)]
pub struct AnswerDecisionInput {
    #[property(description = "The task or decision identifier named by this object")]
    pub id: String,
    #[property(description = "The stable question id on the named task")]
    pub decision_id: String,
    #[property(description = "The actual selected option id, or omit for a free-text answer")]
    pub option_id: Option<String>,
    #[property(description = "The exact text, preserved as supplied")]
    pub text: Option<String>,
    #[property(
        description = "The person who actually answered; this is a report imported from chat, not an authenticated board-UI answer"
    )]
    pub answered_by: String,
}
pub struct AnswerDecisionHandler {
    app: Arc<AppContext>,
}
impl AnswerDecisionHandler {
    pub fn new(app: Arc<AppContext>) -> Self {
        Self { app }
    }
}
impl ToolDefinition for AnswerDecisionHandler {
    const FUNC_NAME: &'static str = "tasks_answer_decision";
    const DESCRIPTION: &'static str = "Record a person's actual reply from the agent chat against its decision id and task. Call only after receiving that reply. Use the selected option id or preserve a free-text reply; never invent a choice. The original question and choices remain immutable.";
}
#[async_trait::async_trait]
impl McpToolCall<AnswerDecisionInput, TaskWorkflowResponse> for AnswerDecisionHandler {
    async fn execute_tool_call(
        &self,
        input: AnswerDecisionInput,
    ) -> Result<TaskWorkflowResponse, String> {
        crate::scripts::update_task(
            &self.app,
            &input.id,
            TaskPatch {
                decision: DecisionPatch::Answer {
                    id: input.decision_id.clone(),
                    answer: DecisionAnswer {
                        option_id: input.option_id,
                        text: input.text.unwrap_or_default(),
                        answered_by: input.answered_by,
                        answered_unix_seconds: 0,
                        source: "agent_reported".into(),
                    },
                },
                ..Default::default()
            },
        )
        .await?;
        read_context(&self.app, &input.id, None, Some(input.decision_id))
    }
}

#[derive(ApplyJsonSchema, Debug, Serialize, Deserialize)]
pub struct CancelDecisionInput {
    #[property(description = "The task or decision identifier named by this object")]
    pub id: String,
    #[property(description = "The stable question id on the named task")]
    pub decision_id: String,
    #[property(description = "Why this pending question is being cancelled")]
    pub reason: String,
    #[property(description = "Who cancelled the question; defaults to AI")]
    pub who: Option<String>,
}
pub struct CancelDecisionHandler {
    app: Arc<AppContext>,
}
impl CancelDecisionHandler {
    pub fn new(app: Arc<AppContext>) -> Self {
        Self { app }
    }
}
impl ToolDefinition for CancelDecisionHandler {
    const FUNC_NAME: &'static str = "tasks_cancel_decision";
    const DESCRIPTION: &'static str = "Cancel an unanswered question with a reason. Preserves the original action and choices in task history; use a new request key to ask a revised question.";
}
#[async_trait::async_trait]
impl McpToolCall<CancelDecisionInput, TaskWorkflowResponse> for CancelDecisionHandler {
    async fn execute_tool_call(
        &self,
        input: CancelDecisionInput,
    ) -> Result<TaskWorkflowResponse, String> {
        crate::scripts::update_task(
            &self.app,
            &input.id,
            TaskPatch {
                decision: DecisionPatch::Cancel {
                    id: input.decision_id.clone(),
                    reason: input.reason,
                    who: input.who.unwrap_or_else(|| "AI".into()),
                },
                ..Default::default()
            },
        )
        .await?;
        read_context(&self.app, &input.id, None, Some(input.decision_id))
    }
}
