use flurl::HttpVerb;
use task_manager_shared::documents::{
    DocumentsResponse, FindDocumentResponse, GetDocumentInputModel, GetDocumentsInputModel,
};

use crate::models::RequestError;

use super::{authed, handle_http_response};

/// Index a project's documents — paths, sizes and ids, WITHOUT the texts.
///
/// The tree is built from the paths this returns; folders exist nowhere else. Reads only, like the board:
/// documents are written, moved, deleted and restored through `/mcp`.
pub async fn get_documents(project_prefix: &str) -> Result<DocumentsResponse, RequestError> {
    let request = GetDocumentsInputModel {
        project: project_prefix.to_string(),
    };

    handle_http_response(authed("/api/documents/v1/list", HttpVerb::Post, request).await).await
}

/// Read one document, text included.
///
/// The other half of the design: a document is not in the board snapshot, so nothing has its text until
/// somebody opens it. A deleted document comes back as a miss carrying `in_trash`, which is a different thing
/// to show than "no such document".
pub async fn get_document(
    project_prefix: &str,
    id: &str,
) -> Result<FindDocumentResponse, RequestError> {
    let request = GetDocumentInputModel {
        project: project_prefix.to_string(),
        id: id.to_string(),
    };

    handle_http_response(authed("/api/documents/v1/get", HttpVerb::Post, request).await).await
}
