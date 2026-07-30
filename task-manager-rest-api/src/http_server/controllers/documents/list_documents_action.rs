use std::sync::Arc;

use service_sdk::macros::use_my_http_server;
use task_manager_shared::documents::{DocumentsResponse, GetDocumentsInputModel};

use crate::app::AppContext;
use crate::mappers::document_entry_to_index_entry;

use_my_http_server!();

#[http_route(
    method: "POST",
    route: "/api/documents/v1/list",
    controller: "Documents",
    summary: "Index a project's documents",
    description: "Every live document of one project, by path, WITHOUT the payloads. The tree the Documents screen draws is built from these paths: folders are not stored anywhere, they are read off the paths of the documents in them, so an empty folder cannot exist. Served from memory — the index is the one part of a document that is cached, precisely so that the payloads are not: a project can hold PDFs, and drawing a tree must not pull them. A text document's content comes from /api/documents/v1/get; a binary one's bytes from /api/documents/v1/raw, which the browser fetches itself. There is deliberately no WebSocket for any of this: the socket carries the board, and documents would ride along on every push. Reads only — documents are written, moved, deleted and restored through /mcp, and the trash is not exposed here at all.",
    input_data: "GetDocumentsInputModel",
    result: [
        {status_code: 200, description: "The index", model: "DocumentsResponse"},
        {status_code: 401, description: "Not authenticated"},
        {status_code: 403, description: "No access to this project"},
        {status_code: 404, description: "No such project"},
    ]
)]
pub struct ListDocumentsAction {
    app: Arc<AppContext>,
}

impl ListDocumentsAction {
    pub fn new(app: Arc<AppContext>) -> Self {
        Self { app }
    }
}

async fn handle_request(
    action: &ListDocumentsAction,
    input_data: GetDocumentsInputModel,
    ctx: &HttpContext,
) -> Result<HttpOkResult, HttpFailResult> {
    // The prefix is what a person calls a board, and it is the only name a project has on this boundary.
    // Resolved once, here, together with the access check; everything below speaks in ids.
    let project =
        crate::auth::require_project_by_prefix(&action.app, ctx, &input_data.project).await?;

    let documents = action
        .app
        .documents_index
        .of_project(&project.id)
        .iter()
        .map(|entry| document_entry_to_index_entry(entry, &project.prefix))
        .collect();

    HttpOutput::as_json(DocumentsResponse { documents }).into_ok_result(true)
}
