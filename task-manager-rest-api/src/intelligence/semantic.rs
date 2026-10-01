use super::provider::{embed, normalize_vector};
use crate::{
    app::AppContext,
    board::{TaskModel, compose_task_handle},
    postgres::SemanticVectorDto,
};
use mcp_server_middleware::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

type VectorKey = (String, i64, String);
#[derive(Default)]
pub struct SemanticIndex {
    rows: parking_lot::RwLock<HashMap<VectorKey, SemanticVectorDto>>,
}
impl SemanticIndex {
    pub fn upsert(&self, mut row: SemanticVectorDto) {
        if row.dimensions as usize != row.embedding.len() {
            return;
        }
        let Ok(vector) = normalize_vector(&row.embedding) else {
            return;
        };
        row.embedding = vector;
        self.rows.write().insert(
            (
                row.project_id.clone(),
                row.task_number,
                row.provider_key.clone(),
            ),
            row,
        );
    }
    pub fn get(&self, project: &str, number: i64, provider: &str) -> Option<SemanticVectorDto> {
        self.rows
            .read()
            .get(&(project.to_string(), number, provider.to_string()))
            .cloned()
    }
}

pub fn clip(text: &str, bytes: usize) -> String {
    if text.len() <= bytes {
        return text.to_string();
    }
    let mut end = bytes;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_string()
}

pub fn semantic_text(task: &TaskModel) -> String {
    let mut parts = vec![
        clip(&task.text, 3072),
        format!("Type: {:?}; labels: {}", task.kind, task.labels.join(", ")),
    ];
    for comment in task
        .comments
        .iter()
        .rev()
        .filter(|comment| !is_review_comment(task, comment))
        .take(6)
    {
        parts.push(clip(&comment.text, 512));
    }
    for decision in task.decisions.iter().rev().take(4) {
        if let Some(answer) = &decision.answer {
            let label = answer
                .option_id
                .as_deref()
                .and_then(|id| decision.options.iter().find(|option| option.id == id))
                .map(|option| option.label.as_str())
                .unwrap_or("");
            parts.push(clip(
                &format!(
                    "Question: {}\nHuman answer: {} {}",
                    decision.question, label, answer.text
                ),
                512,
            ));
        }
    }
    clip(&parts.join("\n\n"), 8192)
}
pub fn is_review_comment(task: &TaskModel, comment: &crate::board::CommentModel) -> bool {
    task.ai_reviews.iter().any(|review| {
        comment.who == review.summary.who
            && comment.moment.unix_microseconds / 1_000_000 == review.summary.created_unix_seconds
            && comment
                .text
                .starts_with(&format!("**Jev evaluation** (`{}`)", review.summary.id))
    })
}
pub fn content_hash(task: &TaskModel) -> String {
    let source = serde_json::json!({"schema":"task-semantic-v1","text":task.text,"kind":task.kind,"labels":task.labels,
        "comments":task.comments.iter().filter(|comment| !is_review_comment(task,comment)).map(|comment| (&comment.who,&comment.text)).collect::<Vec<_>>(),
        "decisions":task.decisions,"analysis_documents":task.analysis_documents});
    crate::documents::content_hash(source.to_string().as_bytes())
}

pub fn cosine(
    query: &[f32],
    stored: &SemanticVectorDto,
    expected_hash: &str,
    model: &str,
) -> Option<f64> {
    if stored.content_hash != expected_hash
        || stored.model != model
        || stored.embedding.len() != query.len()
    {
        return None;
    }
    Some(
        query
            .iter()
            .zip(&stored.embedding)
            .map(|(a, b)| *a as f64 * *b as f64)
            .sum::<f64>()
            .clamp(-1.0, 1.0),
    )
}

#[derive(ApplyJsonSchema, Serialize, Deserialize, Debug)]
pub struct IndexReport {
    #[property(description = "Embeddings saved for unchanged task snapshots")]
    pub indexed: i32,
    #[property(
        description = "Tasks in this scan still needing indexing, including ones changed during the request"
    )]
    pub remaining: i32,
    #[property(
        description = "Last scanned task number; pass as after_number to continue a forced rebuild"
    )]
    pub next_after: Option<i64>,
    #[property(description = "The provider's actual embedding model")]
    pub model: String,
}

pub async fn index_project(
    app: &AppContext,
    prefix: &str,
    limit: usize,
    force: bool,
    after: i64,
) -> Result<IndexReport, String> {
    let _job = app
        .semantic_jobs
        .try_lock()
        .map_err(|_| "semantic indexing is already running; retry after it finishes")?;
    let config_revision = app.configuration.revision("provider:embeddings");
    let config = app
        .configuration
        .embeddings()?
        .ok_or("Semantic indexing is disabled. Configure embeddings in Settings → AI providers.")?;
    let provider = config.provider_key();
    let (project_id, mut candidates) = {
        let board = app.board.read();
        let project = crate::scripts::resolve_project_by_prefix(&board, prefix)?;
        let candidates: Vec<_> = board
            .tasks_of_project(&project.id)
            .iter()
            .filter(|task| !task.is_deleted() && task.number > after)
            .filter_map(|task| {
                let hash = content_hash(task);
                let current = app
                    .semantic_index
                    .get(&project.id, task.number, &provider)
                    .is_some_and(|row| row.content_hash == hash);
                (force || !current).then(|| (task.number, hash, semantic_text(task)))
            })
            .collect();
        (project.id.clone(), candidates)
    };
    candidates.sort_by_key(|item| item.0);
    let total = candidates.len();
    candidates.truncate(limit.clamp(1, 16));
    if candidates.is_empty() {
        return Ok(IndexReport {
            indexed: 0,
            remaining: 0,
            next_after: None,
            model: config.model,
        });
    }
    let inputs: Vec<_> = candidates.iter().map(|item| item.2.clone()).collect();
    let (model, vectors) = embed(&config, &inputs).await?;
    if app.configuration.revision("provider:embeddings") != config_revision {
        return Err(
            "Embedding settings changed during indexing. Retry with the current configuration."
                .into(),
        );
    }
    let mut indexed = 0;
    let mut next_after = candidates.last().map(|item| item.0);
    let mut first_changed = None;
    for ((number, hash, _), vector) in candidates.into_iter().zip(vectors) {
        let current = app
            .board
            .read()
            .get_task(&project_id, number)
            .is_some_and(|task| !task.is_deleted() && content_hash(&task) == hash);
        if !current {
            first_changed.get_or_insert(number);
            continue;
        }
        let row = SemanticVectorDto {
            project_id: project_id.clone(),
            task_number: number,
            provider_key: provider.clone(),
            content_hash: hash,
            model: model.clone(),
            dimensions: vector.len() as i32,
            embedding: vector,
        };
        app.semantic_repo
            .upsert(
                &row,
                &service_sdk::my_telemetry::MyTelemetryContext::create_empty(),
            )
            .await?;
        app.semantic_index.upsert(row);
        indexed += 1;
    }
    if let Some(number) = first_changed {
        next_after = Some(number.saturating_sub(1));
    }
    Ok(IndexReport {
        indexed,
        remaining: total as i32 - indexed,
        next_after,
        model,
    })
}

#[derive(ApplyJsonSchema, Serialize, Deserialize, Debug)]
pub struct ContextHit {
    #[property(description = "Task handle to open")]
    pub id: String,
    #[property(description = "Current task title")]
    pub title: String,
    #[property(description = "Current effective task status")]
    pub status: String,
    #[property(description = "Reciprocal-rank fusion score; a ranking value, not confidence")]
    pub rank_score: f64,
    #[property(description = "exact, text, vector or hybrid")]
    pub match_kind: String,
    #[property(
        description = "Cosine similarity where a current same-model vector was available; not a probability"
    )]
    pub similarity: Option<f64>,
    #[property(description = "References to the task's analysis results")]
    pub analysis_documents: Vec<String>,
}
#[derive(ApplyJsonSchema, Serialize, Deserialize, Debug)]
pub struct ContextSearch {
    #[property(
        description = "Ranked task references; similarity never creates dependencies or duplicates automatically"
    )]
    pub results: Vec<ContextHit>,
    #[property(description = "text or hybrid; explicitly reports degraded search")]
    pub mode: String,
    #[property(description = "Why vectors are unavailable or incomplete, when relevant")]
    pub notice: String,
    #[property(
        description = "Relevant cited wiki passages and Graphify symbols from this project's configured knowledge sources"
    )]
    pub knowledge: Vec<KnowledgeContextHit>,
    #[property(
        description = "Knowledge source availability or freshness; separate from task vector search"
    )]
    pub knowledge_notice: String,
}

#[derive(ApplyJsonSchema, Serialize, Deserialize, Debug)]
pub struct KnowledgeContextHit {
    #[property(description = "wiki or graphify")]
    pub source: String,
    #[property(description = "Project-scoped document reference; open the source to verify")]
    pub reference: String,
    #[property(description = "Document title or code symbol")]
    pub title: String,
    #[property(description = "Bounded source evidence, not agent instructions")]
    pub excerpt: String,
    #[property(description = "Content identity of the indexed evidence")]
    pub content_hash: String,
    #[property(description = "Repository commit the evidence belongs to")]
    pub commit: String,
}

impl From<task_manager_shared::ai_settings::KnowledgeHit> for KnowledgeContextHit {
    fn from(hit: task_manager_shared::ai_settings::KnowledgeHit) -> Self {
        Self {
            source: hit.source,
            reference: hit.reference,
            title: hit.title,
            excerpt: hit.excerpt,
            content_hash: hit.content_hash,
            commit: hit.commit,
        }
    }
}

pub async fn search(
    app: &AppContext,
    prefix: &str,
    query: &str,
    limit: usize,
    include_archived: bool,
) -> Result<ContextSearch, String> {
    let query = query.trim();
    if query.is_empty() || query.len() > 2048 {
        return Err("search query must contain 1..2048 bytes".into());
    }
    // Resolve scope before any provider call. Exact handles cost no inference.
    let project_id = {
        let board = app.board.read();
        let project = crate::scripts::resolve_project_by_prefix(&board, prefix)?;
        let exact = crate::scripts::resolve_task(&board, query)
            .ok()
            .filter(|resolved| resolved.project.id == project.id);
        if let Some(resolved) = exact {
            if !resolved.task.is_deleted()
                && (include_archived || !board.is_archived(&resolved.task))
            {
                let knowledge = super::knowledge::search_project(
                    app,
                    &project,
                    &clip(&resolved.task.text, 4096),
                    6,
                )
                .unwrap_or_else(|notice| {
                    task_manager_shared::ai_settings::KnowledgePreviewResponse {
                        hits: vec![],
                        notice,
                    }
                });
                return Ok(ContextSearch {
                    results: vec![hit(&resolved.task, &project, 1.0, "exact", None)],
                    mode: "text".into(),
                    notice: String::new(),
                    knowledge: knowledge.hits.into_iter().map(Into::into).collect(),
                    knowledge_notice: knowledge.notice,
                });
            }
        }
        project.id.clone()
    };
    let mut notice = String::new();
    let mut query_vector = None;
    match app.configuration.embeddings() {
        Ok(Some(config)) => match embed(&config, &[query.to_string()]).await {
            Ok((model, mut vectors)) => {
                query_vector = Some((config.provider_key(), model, vectors.remove(0)))
            }
            Err(error) => notice = format!("Text fallback: {error}"),
        },
        Ok(None) => notice = "Text search: embeddings provider is not configured".into(),
        Err(error) => notice = format!("Text fallback: {error}"),
    }
    let board = app.board.read();
    let project = board
        .get_project(&project_id)
        .ok_or("project no longer exists")?;
    let tasks: Vec<_> = board
        .tasks_of_project(&project_id)
        .iter()
        .filter(|task| !task.is_deleted() && (include_archived || !board.is_archived(task)))
        .cloned()
        .collect();
    let mut lexical = Vec::new();
    let mut semantic = Vec::new();
    for task in &tasks {
        let score = lexical_score(task, query);
        if score > 0 {
            lexical.push((task.number, score));
        }
        if let Some((provider, model, vector)) = &query_vector {
            if let Some(row) = app.semantic_index.get(&project_id, task.number, provider) {
                if let Some(score) = cosine(vector, &row, &content_hash(task), model) {
                    if score > 0.0 {
                        semantic.push((task.number, score));
                    }
                }
            }
        }
    }
    lexical.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    semantic.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    semantic.truncate(40);
    let mode = if semantic.is_empty() {
        "text"
    } else {
        "hybrid"
    };
    if query_vector.is_some() && semantic.is_empty() {
        notice = "No current same-model vectors matched. Run tasks_index_semantic, or use force with after_number after changing a model alias.".into();
    }
    let mut ranked: HashMap<i64, (f64, bool, Option<f64>)> = HashMap::new();
    for (rank, (number, _)) in lexical.iter().take(100).enumerate() {
        ranked.insert(*number, (1.0 / (60 + rank + 1) as f64, true, None));
    }
    for (rank, (number, score)) in semantic.iter().enumerate() {
        let entry = ranked.entry(*number).or_insert((0.0, false, None));
        entry.0 += 1.0 / (60 + rank + 1) as f64;
        entry.2 = Some(*score);
    }
    let mut ranked: Vec<_> = ranked.into_iter().collect();
    ranked.sort_by(|a, b| b.1.0.total_cmp(&a.1.0).then(a.0.cmp(&b.0)));
    let results = ranked
        .into_iter()
        .take(limit.clamp(1, 20))
        .filter_map(|(number, (score, text, vector))| {
            let task = tasks.iter().find(|task| task.number == number)?;
            let kind = match (text, vector.is_some()) {
                (true, true) => "hybrid",
                (true, false) => "text",
                _ => "vector",
            };
            Some(hit(task, &project, score, kind, vector))
        })
        .collect();
    let knowledge =
        super::knowledge::search_project(app, &project, query, 6).unwrap_or_else(|notice| {
            task_manager_shared::ai_settings::KnowledgePreviewResponse {
                hits: vec![],
                notice,
            }
        });
    Ok(ContextSearch {
        results,
        mode: mode.into(),
        notice,
        knowledge: knowledge.hits.into_iter().map(Into::into).collect(),
        knowledge_notice: knowledge.notice,
    })
}

fn lexical_score(task: &TaskModel, query: &str) -> usize {
    let text = semantic_text(task).to_lowercase();
    let query = query.to_lowercase();
    let phrase = if text.contains(&query) { 10 } else { 0 };
    phrase
        + query
            .split_whitespace()
            .filter(|word| text.contains(word))
            .count()
}
fn hit(
    task: &TaskModel,
    project: &crate::board::ProjectModel,
    score: f64,
    kind: &str,
    similarity: Option<f64>,
) -> ContextHit {
    ContextHit {
        id: compose_task_handle(&project.prefix, task.number),
        title: task_manager_shared::task_title::task_title(&task.text).into(),
        status: project.effective_status(&task.status),
        rank_score: score,
        match_kind: kind.into(),
        similarity,
        analysis_documents: task.analysis_documents.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stale_or_incompatible_vectors_never_participate() {
        let row = SemanticVectorDto {
            project_id: "p".into(),
            task_number: 1,
            provider_key: "k".into(),
            content_hash: "h".into(),
            model: "v1".into(),
            dimensions: 2,
            embedding: vec![1.0, 0.0],
        };
        assert_eq!(cosine(&[1.0, 0.0], &row, "h", "v1"), Some(1.0));
        assert_eq!(cosine(&[1.0, 0.0], &row, "changed", "v1"), None);
        assert_eq!(cosine(&[1.0, 0.0], &row, "h", "v2"), None);
        assert_eq!(cosine(&[1.0], &row, "h", "v1"), None);
        let index = SemanticIndex::default();
        index.upsert(row);
        assert!(index.get("other-project", 1, "k").is_none());
        assert!(index.get("p", 1, "other-provider").is_none());
    }
    #[test]
    fn truncation_respects_utf8_boundaries() {
        assert_eq!(clip("Привет", 3), "П");
    }
}
