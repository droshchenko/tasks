use my_http_utils::macros::{MyHttpInput, MyHttpObjectStructure};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, MyHttpObjectStructure, Clone, Debug, Default, PartialEq)]
pub struct ProviderSettingsResponse {
    pub provider: String,
    pub enabled: bool,
    pub endpoint: String,
    pub model: String,
    pub has_key: bool,
    pub source: String,
    pub revision: i64,
    pub configured: bool,
    pub notice: String,
}

#[derive(Serialize, Deserialize, MyHttpObjectStructure, Clone, Debug, PartialEq)]
pub struct AiSettingsResponse {
    pub embeddings: ProviderSettingsResponse,
    pub jev: ProviderSettingsResponse,
}

#[derive(MyHttpInput)]
pub struct SaveProviderInput {
    #[http_body(name: "provider", description: "embeddings or jev")]
    pub provider: String,
    #[http_body(name: "enabled", description: "Enable this provider")]
    pub enabled: bool,
    #[http_body(name: "endpoint", description: "Full embeddings URL; Jev uses the fixed TypeSafe endpoint")]
    pub endpoint: String,
    #[http_body(name: "model", description: "Provider model identifier")]
    pub model: String,
    #[http_body(name: "apiKey", description: "A replacement credential; never returned by reads")]
    pub api_key: Option<String>,
    #[http_body(name: "clearKey", description: "Explicitly remove the current credential")]
    pub clear_key: bool,
    #[http_body(name: "revision", description: "Revision last read by this editor")]
    pub revision: i64,
}

#[derive(MyHttpInput)]
pub struct TestProviderInput {
    #[http_body(name: "provider", description: "Test the saved embeddings or jev connection using synthetic input")]
    pub provider: String,
}

#[derive(Serialize, Deserialize, MyHttpObjectStructure, Clone, Debug, PartialEq)]
pub struct ProviderTestResponse {
    pub success: bool,
    pub model: String,
    pub message: String,
    pub elapsed_ms: i64,
}

#[derive(MyHttpInput)]
pub struct ProjectAiInput {
    #[http_body(name: "project", description: "Project prefix")]
    pub project: String,
}

#[derive(MyHttpInput)]
pub struct StartIndexInput {
    #[http_body(name: "project", description: "Project prefix")]
    pub project: String,
    #[http_body(name: "force", description: "Rebuild existing task vectors as well")]
    pub force: bool,
}

#[derive(Serialize, Deserialize, MyHttpObjectStructure, Clone, Debug, Default, PartialEq)]
pub struct IndexJobResponse {
    pub state: String,
    pub processed: i32,
    pub total: i32,
    pub remaining: i32,
    pub started_unix_seconds: Option<i64>,
    pub finished_unix_seconds: Option<i64>,
    pub message: String,
}

#[derive(Serialize, Deserialize, MyHttpObjectStructure, Clone, Debug, PartialEq)]
pub struct IndexStatusResponse {
    pub project: String,
    pub configured: bool,
    pub tasks_total: i32,
    pub vectors_current: i32,
    pub vectors_missing: i32,
    pub model: String,
    pub notice: String,
    pub job: IndexJobResponse,
}

#[derive(Serialize, Deserialize, MyHttpObjectStructure, Clone, Debug, Default, PartialEq)]
pub struct KnowledgeConfig {
    pub enabled: bool,
    pub connection: String,
    pub wiki_paths: Vec<String>,
    pub graph_source: String,
    pub graph_path: String,
    pub auto_refresh: bool,
}

#[derive(Serialize, Deserialize, MyHttpObjectStructure, Clone, Debug, PartialEq)]
pub struct KnowledgeConnection {
    pub name: String,
    pub repository: String,
    pub branch: String,
    pub root_path: String,
    pub state: String,
    pub commit: String,
}

#[derive(Serialize, Deserialize, MyHttpObjectStructure, Clone, Debug, Default, PartialEq)]
pub struct KnowledgeStatus {
    pub state: String,
    pub wiki_documents: i32,
    pub graph_nodes: i32,
    pub graph_edges: i32,
    pub source_commit: String,
    pub graph_commit: String,
    pub updated_unix_seconds: Option<i64>,
    pub notice: String,
}

#[derive(Serialize, Deserialize, MyHttpObjectStructure, Clone, Debug, PartialEq)]
pub struct KnowledgeSettingsResponse {
    pub project: String,
    pub revision: i64,
    pub config: KnowledgeConfig,
    pub connections: Vec<KnowledgeConnection>,
    pub status: KnowledgeStatus,
}

#[derive(MyHttpInput)]
pub struct SaveKnowledgeInput {
    #[http_body(name: "project", description: "Project prefix")]
    pub project: String,
    #[http_body(name: "revision", description: "Last observed settings revision")]
    pub revision: i64,
    #[http_body(name: "enabled", description: "Use project knowledge in context retrieval")]
    pub enabled: bool,
    #[http_body(name: "connection", description: "Existing repository connection name")]
    pub connection: String,
    #[http_body(name: "wikiPaths", description: "Wiki folders or Markdown files relative to the connected root")]
    pub wiki_paths: Vec<String>,
    #[http_body(name: "graphSource", description: "none, repository or upload")]
    pub graph_source: String,
    #[http_body(name: "graphPath", description: "Graphify JSON export in the connected repository")]
    pub graph_path: String,
    #[http_body(name: "autoRefresh", description: "Refresh wiki sources when the connected repository changes")]
    pub auto_refresh: bool,
}

#[derive(MyHttpInput)]
pub struct UploadGraphInput {
    #[http_query(name: "project", description: "Project prefix")]
    pub project: String,
    #[http_query(name: "revision", description: "Last observed knowledge settings revision")]
    pub revision: i64,
    #[http_body_raw(description: "Graphify JSON export, up to 64 MiB")]
    pub content: Vec<u8>,
}

#[derive(MyHttpInput)]
pub struct KnowledgePreviewInput {
    #[http_body(name: "project", description: "Project prefix")]
    pub project: String,
    #[http_body(name: "query", description: "Text or symbol to retrieve from saved project sources")]
    pub query: String,
}

#[derive(Serialize, Deserialize, MyHttpObjectStructure, Clone, Debug, PartialEq)]
pub struct KnowledgeHit {
    pub source: String,
    pub reference: String,
    pub title: String,
    pub excerpt: String,
    pub content_hash: String,
    pub commit: String,
}

#[derive(Serialize, Deserialize, MyHttpObjectStructure, Clone, Debug, Default, PartialEq)]
pub struct KnowledgePreviewResponse {
    pub hits: Vec<KnowledgeHit>,
    pub notice: String,
}
