use std::sync::Arc;

use service_sdk::macros::use_my_http_server;
use task_manager_shared::documents::{FindDocumentResponse, GetDocumentInputModel};

use crate::app::AppContext;
use crate::http_server::errors::forbidden;
use crate::mappers::document_to_response;

use_my_http_server!();

#[http_route(
    method: "POST",
    route: "/api/documents/v1/get",
    controller: "Documents",
    summary: "Read one document",
    description: "One document with its text. This is the fetch-on-demand half of the design: a board read carries only the IDS of the documents a task or a goal references, and the text is asked for here when somebody opens one. A miss comes back as prose rather than as an empty body — most usefully for a document that has been deleted, which reads as 'it is in the trash' and not as a blank pane, because restoring it is one MCP call away. The trash itself is not readable from here: what is in it, and putting anything back, are MCP-only.",
    input_data: "GetDocumentInputModel",
    result: [
        {status_code: 200, description: "The document, or why there is none", model: "FindDocumentResponse"},
        {status_code: 401, description: "Not authenticated"},
        {status_code: 403, description: "No access to this project"},
    ]
)]
pub struct GetDocumentAction {
    app: Arc<AppContext>,
}

impl GetDocumentAction {
    pub fn new(app: Arc<AppContext>) -> Self {
        Self { app }
    }
}

async fn handle_request(
    action: &GetDocumentAction,
    input_data: GetDocumentInputModel,
    ctx: &HttpContext,
) -> Result<HttpOkResult, HttpFailResult> {
    crate::auth::require_project_access(&action.app, ctx, &input_data.project_id).await?;

    let telemetry = service_sdk::my_telemetry::MyTelemetryContext::create_empty();
    let id = input_data.id.trim();

    if let Some(row) = action.app.documents_repo.get_by_id(id, &telemetry).await {
        // The caller passed a project AND an id, and the id is what actually finds the row — so the two have
        // to be checked to agree. Without this, membership of one project would read any document of any
        // other: the access check above is about the project that was NAMED, not the one the document is on.
        if row.project_id != input_data.project_id {
            return Err(forbidden("That document belongs to another project"));
        }

        return HttpOutput::as_json(FindDocumentResponse {
            document: Some(document_to_response(&row)),
            in_trash: false,
            not_found: String::new(),
        })
        .into_ok_result(true);
    }

    // Not live. The trash is asked ONLY to say which of the two misses this is — the row itself is not
    // returned, because the browser does not show trashed documents at all.
    let in_trash = action
        .app
        .documents_repo
        .get_trashed(id, &telemetry)
        .await
        .map(|itm| itm.project_id == input_data.project_id)
        .unwrap_or(false);

    let not_found = if in_trash {
        "This document has been deleted — it is in the trash. Restoring it is an MCP call.".to_string()
    } else {
        format!("No document {id}")
    };

    HttpOutput::as_json(FindDocumentResponse {
        document: None,
        in_trash,
        not_found,
    })
    .into_ok_result(true)
}
