use super::{authed, handle_http_response, handle_http_response_opt};
use crate::models::RequestError;
use flurl::{EmptyRequestModel, HttpVerb};
use task_manager_shared::ai_settings::*;

pub async fn get_ai_settings() -> Result<Option<AiSettingsResponse>, RequestError> {
    handle_http_response_opt(
        authed("/api/settings/ai/v1/get", HttpVerb::Post, EmptyRequestModel).await,
    )
    .await
}
pub async fn save_ai_provider(
    input: SaveProviderInput,
) -> Result<AiSettingsResponse, RequestError> {
    handle_http_response(authed("/api/settings/ai/v1/save", HttpVerb::Post, input).await).await
}
pub async fn test_ai_provider(provider: String) -> Result<ProviderTestResponse, RequestError> {
    handle_http_response(
        authed(
            "/api/settings/ai/v1/test",
            HttpVerb::Post,
            TestProviderInput { provider },
        )
        .await,
    )
    .await
}
pub async fn get_index_status(project: String) -> Result<IndexStatusResponse, RequestError> {
    handle_http_response(
        authed(
            "/api/settings/ai/v1/index/status",
            HttpVerb::Post,
            ProjectAiInput { project },
        )
        .await,
    )
    .await
}
pub async fn start_task_index(
    project: String,
    force: bool,
) -> Result<IndexStatusResponse, RequestError> {
    handle_http_response(
        authed(
            "/api/settings/ai/v1/index/start",
            HttpVerb::Post,
            StartIndexInput { project, force },
        )
        .await,
    )
    .await
}
pub async fn cancel_task_index(project: String) -> Result<IndexStatusResponse, RequestError> {
    handle_http_response(
        authed(
            "/api/settings/ai/v1/index/cancel",
            HttpVerb::Post,
            ProjectAiInput { project },
        )
        .await,
    )
    .await
}
pub async fn get_knowledge_settings(
    project: String,
) -> Result<KnowledgeSettingsResponse, RequestError> {
    handle_http_response(
        authed(
            "/api/settings/knowledge/v1/get",
            HttpVerb::Post,
            ProjectAiInput { project },
        )
        .await,
    )
    .await
}
pub async fn save_knowledge_settings(
    project: String,
    revision: i64,
    config: KnowledgeConfig,
) -> Result<KnowledgeSettingsResponse, RequestError> {
    let input = SaveKnowledgeInput {
        project,
        revision,
        enabled: config.enabled,
        connection: config.connection,
        wiki_paths: config.wiki_paths,
        graph_source: config.graph_source,
        graph_path: config.graph_path,
        auto_refresh: config.auto_refresh,
    };
    handle_http_response(authed("/api/settings/knowledge/v1/save", HttpVerb::Post, input).await)
        .await
}
pub async fn refresh_knowledge(project: String) -> Result<KnowledgeSettingsResponse, RequestError> {
    handle_http_response(
        authed(
            "/api/settings/knowledge/v1/refresh",
            HttpVerb::Post,
            ProjectAiInput { project },
        )
        .await,
    )
    .await
}
pub async fn upload_knowledge_graph(
    project: String,
    revision: i64,
    content: Vec<u8>,
) -> Result<KnowledgeSettingsResponse, RequestError> {
    handle_http_response(
        authed(
            "/api/settings/knowledge/v1/graph",
            HttpVerb::Post,
            UploadGraphInput {
                project,
                revision,
                content,
            },
        )
        .await,
    )
    .await
}
pub async fn preview_knowledge(
    project: String,
    query: String,
) -> Result<KnowledgePreviewResponse, RequestError> {
    handle_http_response(
        authed(
            "/api/settings/knowledge/v1/preview",
            HttpVerb::Post,
            KnowledgePreviewInput { project, query },
        )
        .await,
    )
    .await
}
