use crate::{
    app::AppContext,
    intelligence::{jev, semantic},
};
use mcp_server_middleware::*;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(ApplyJsonSchema, Serialize, Deserialize, Debug)]
pub struct SemanticIndexInput {
    #[property(
        description = "Project prefix to index; only its non-deleted tasks are sent to the configured embeddings provider"
    )]
    pub project: String,
    #[property(description = "Maximum task snapshots to embed, 1..16, default 16")]
    pub limit: Option<i64>,
    #[property(
        description = "Rebuild cached vectors too, for example after changing a model alias"
    )]
    pub force: Option<bool>,
    #[property(
        description = "Continue a forced rebuild after the previous next_after task number; omit to scan from the start"
    )]
    pub after_number: Option<i64>,
}
pub struct SemanticIndexHandler {
    app: Arc<AppContext>,
}
impl SemanticIndexHandler {
    pub fn new(app: Arc<AppContext>) -> Self {
        Self { app }
    }
}
impl ToolDefinition for SemanticIndexHandler {
    const FUNC_NAME: &'static str = "tasks_index_semantic";
    const DESCRIPTION: &'static str = "Build a bounded batch of current task embeddings for one project. Sends task text, selected recent comments and recorded human decisions to the backend-configured provider. Keys never travel through MCP. Derived vectors do not change task state. Repeat until remaining is zero; for force use next_after as after_number. Ordinary text search works without this feature.";
}
#[async_trait::async_trait]
impl McpToolCall<SemanticIndexInput, semantic::IndexReport> for SemanticIndexHandler {
    async fn execute_tool_call(
        &self,
        input: SemanticIndexInput,
    ) -> Result<semantic::IndexReport, String> {
        semantic::index_project(
            &self.app,
            &input.project,
            input.limit.unwrap_or(16).clamp(1, 16) as usize,
            input.force.unwrap_or(false),
            input.after_number.unwrap_or(0).max(0),
        )
        .await
    }
}

#[derive(ApplyJsonSchema, Serialize, Deserialize, Debug)]
pub struct ContextSearchInput {
    #[property(description = "Project prefix; results never cross this boundary")]
    pub project: String,
    #[property(
        description = "Task ID or natural-language search. Sent to the configured embeddings provider except for an exact task ID"
    )]
    pub query: String,
    #[property(description = "Maximum results, 1..20, default 10")]
    pub limit: Option<i64>,
    #[property(description = "Include archived tasks when looking for past work; true by default")]
    pub include_archived: Option<bool>,
}
pub struct ContextSearchHandler {
    app: Arc<AppContext>,
}
impl ContextSearchHandler {
    pub fn new(app: Arc<AppContext>) -> Self {
        Self { app }
    }
}
impl ToolDefinition for ContextSearchHandler {
    const FUNC_NAME: &'static str = "tasks_search_context";
    const DESCRIPTION: &'static str = "Find similar work and past human decisions using exact IDs, text and current same-model task vectors. Reports whether search was hybrid or fell back to text. Similarity is not a probability or evidence of a dependency/duplicate. Open candidate tasks and their analysis-file links before acting. tasks_search remains the exhaustive local text/regex search.";
}
#[async_trait::async_trait]
impl McpToolCall<ContextSearchInput, semantic::ContextSearch> for ContextSearchHandler {
    async fn execute_tool_call(
        &self,
        input: ContextSearchInput,
    ) -> Result<semantic::ContextSearch, String> {
        semantic::search(
            &self.app,
            &input.project,
            &input.query,
            input.limit.unwrap_or(10).clamp(1, 20) as usize,
            input.include_archived.unwrap_or(true),
        )
        .await
    }
}

#[derive(ApplyJsonSchema, Serialize, Deserialize, Debug)]
pub struct ReviewInput {
    #[property(description = "Task handle to evaluate")]
    pub id: String,
    #[property(description = "Author requesting this evaluation; use your established identity")]
    pub who: String,
}
#[derive(ApplyJsonSchema, Serialize, Deserialize, Debug)]
pub struct ReviewView {
    #[property(description = "Immutable evaluation ID")]
    pub id: String,
    #[property(
        description = "SHA-256 of the exact model request, including selected rules and context"
    )]
    pub input_hash: String,
    #[property(description = "Actual provider-reported model")]
    pub model: String,
    #[property(
        description = "Version of the deterministic review policy and question definitions"
    )]
    pub policy_version: String,
    #[property(description = "Author who requested the evaluation")]
    pub who: String,
    #[property(description = "Server time of the evaluation")]
    pub created_unix_seconds: i64,
    #[property(description = "jev or imported provenance; never a human answer")]
    pub source: String,
    #[property(description = "Proposed configured task-type ID; absent means keep current")]
    pub suggested_kind: Option<String>,
    #[property(
        description = "Probability of the selected kind option, separate from distribution confidence"
    )]
    pub kind_probability: f64,
    #[property(description = "Choice distribution confidence, not human approval")]
    pub kind_confidence: f64,
    #[property(description = "Priority proposed from the ordered urgency rubric")]
    pub suggested_priority: String,
    #[property(description = "Continuous expected urgency score on levels 0..4")]
    pub urgency_score: f64,
    #[property(description = "Urgency distribution confidence")]
    pub urgency_confidence: f64,
    #[property(
        description = "Noul probability that material input or an owner decision is missing; no extra confidence is invented"
    )]
    pub needs_human_probability: f64,
    #[property(
        description = "Candidate duplicate from the supplied same-project context; no relation was created"
    )]
    pub suggested_duplicate: Option<String>,
    #[property(description = "Probability of the selected duplicate option, which may be none")]
    pub duplicate_probability: f64,
    #[property(description = "Duplicate-choice distribution confidence")]
    pub duplicate_confidence: f64,
    #[property(
        description = "Always true in the initial policy: a recommendation cannot authorize mutations or answer a person for them"
    )]
    pub review_required: bool,
    #[property(
        description = "Exact request snapshot, only when explicitly requested from history"
    )]
    pub request_json: Option<String>,
    #[property(
        description = "Validated provider response and full distributions, only when explicitly requested from history"
    )]
    pub response_json: Option<String>,
}
impl ReviewView {
    fn from_record(review: &task_manager_shared::ai_reviews::AiReview, snapshots: bool) -> Self {
        let item = &review.summary;
        Self {
            id: item.id.clone(),
            input_hash: item.input_hash.clone(),
            model: item.model.clone(),
            policy_version: item.policy_version.clone(),
            who: item.who.clone(),
            created_unix_seconds: item.created_unix_seconds,
            source: item.source.clone(),
            suggested_kind: item.suggested_kind.clone(),
            kind_probability: item.kind_probability,
            kind_confidence: item.kind_confidence,
            suggested_priority: item.suggested_priority.clone(),
            urgency_score: item.urgency_score,
            urgency_confidence: item.urgency_confidence,
            needs_human_probability: item.needs_human_probability,
            suggested_duplicate: item.suggested_duplicate.clone(),
            duplicate_probability: item.duplicate_probability,
            duplicate_confidence: item.duplicate_confidence,
            review_required: item.review_required,
            request_json: snapshots.then(|| review.request_json.clone()),
            response_json: snapshots.then(|| review.response_json.clone()),
        }
    }
}
pub struct ReviewHandler {
    app: Arc<AppContext>,
}
impl ReviewHandler {
    pub fn new(app: Arc<AppContext>) -> Self {
        Self { app }
    }
}
impl ToolDefinition for ReviewHandler {
    const FUNC_NAME: &'static str = "tasks_review";
    const DESCRIPTION: &'static str = "Evaluate one task directly with TypeSafe Jev on the tasks backend: configured task type (Choice), urgency (Score), need for human clarification (Noul), and possible duplicate among cached related tasks (Choice). Sends a bounded task/rule/project-knowledge snapshot to the configured provider. Records the validated evaluation and an authored task comment; identical current snapshots reuse history. Does not change type, priority, dependencies, status or human answers. Stale results are rejected after inference. An administrator configures Jev in Settings → AI providers.";
}
#[async_trait::async_trait]
impl McpToolCall<ReviewInput, ReviewView> for ReviewHandler {
    async fn execute_tool_call(&self, input: ReviewInput) -> Result<ReviewView, String> {
        let review = jev::evaluate(&self.app, &input.id, &input.who).await?;
        Ok(ReviewView::from_record(&review, false))
    }
}

#[derive(ApplyJsonSchema, Serialize, Deserialize, Debug)]
pub struct ReviewHistoryInput {
    #[property(description = "Task handle")]
    pub id: String,
    #[property(description = "Maximum latest records, 1..10, default 3")]
    pub limit: Option<i64>,
    #[property(
        description = "Include exact request and response JSON for auditing; false by default to keep context small"
    )]
    pub include_snapshots: Option<bool>,
}
#[derive(ApplyJsonSchema, Serialize, Deserialize, Debug)]
pub struct ReviewHistory {
    #[property(
        description = "Recorded evaluations, newest first; historical results are not a new model call"
    )]
    pub reviews: Vec<ReviewView>,
}
pub struct ReviewHistoryHandler {
    app: Arc<AppContext>,
}
impl ReviewHistoryHandler {
    pub fn new(app: Arc<AppContext>) -> Self {
        Self { app }
    }
}
impl ToolDefinition for ReviewHistoryHandler {
    const FUNC_NAME: &'static str = "tasks_review_history";
    const DESCRIPTION: &'static str = "Read recorded Jev evaluations with their input hashes, model, rule version, author and probabilities. Optionally inspect exact snapshots. This is read-only and works without an AI provider key.";
}
#[async_trait::async_trait]
impl McpToolCall<ReviewHistoryInput, ReviewHistory> for ReviewHistoryHandler {
    async fn execute_tool_call(&self, input: ReviewHistoryInput) -> Result<ReviewHistory, String> {
        let board = self.app.board.read();
        let resolved = crate::scripts::resolve_task(&board, &input.id)?;
        Ok(ReviewHistory {
            reviews: resolved
                .task
                .ai_reviews
                .iter()
                .rev()
                .take(input.limit.unwrap_or(3).clamp(1, 10) as usize)
                .map(|review| {
                    ReviewView::from_record(review, input.include_snapshots.unwrap_or(false))
                })
                .collect(),
        })
    }
}
