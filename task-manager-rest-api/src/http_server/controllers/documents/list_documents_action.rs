use std::sync::Arc;

use service_sdk::macros::use_my_http_server;
use task_manager_shared::documents::{DocumentsResponse, GetDocumentsInputModel};

use crate::app::AppContext;
use crate::http_server::errors::not_found;
use crate::mappers::document_to_index_entry;

use_my_http_server!();

#[http_route(
    method: "POST",
    route: "/api/documents/v1/list",
    controller: "Documents",
    summary: "Index a project's documents",
    description: "Every live document of one project, by path, WITHOUT the texts. The tree the Documents screen draws is built from these paths: folders are not stored anywhere, they are read off the paths of the documents in them, so an empty folder cannot exist. Reads only — a document is written, moved, deleted and restored through /mcp, like everything else. The trash is deliberately not returned by anything here: it is not a place to browse, and MCP is the only way to see it. A document is the one thing in this service that is NOT held in memory, so this reads Postgres on every call.",
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
    crate::auth::require_project_access(&action.app, ctx, &input_data.project_id).await?;

    // The project is checked to exist before the query rather than after: an unknown id would otherwise
    // come back as an empty list, which reads as "this project has no documents".
    if action
        .app
        .board
        .read()
        .get_project(&input_data.project_id)
        .is_none()
    {
        return Err(not_found("No such project"));
    }

    let rows =
        crate::scripts::list_documents_of_project(&action.app, &input_data.project_id).await;

    let documents = rows.iter().map(document_to_index_entry).collect();

    HttpOutput::as_json(DocumentsResponse { documents }).into_ok_result(true)
}
