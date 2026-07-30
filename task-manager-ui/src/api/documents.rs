use flurl::HttpVerb;
use task_manager_shared::documents::{
    DocumentsResponse, FindDocumentResponse, GetDocumentInputModel, GetDocumentsInputModel,
    UploadDocumentInputModel, UploadDocumentResponse,
};

use crate::models::RequestError;

use super::{authed, handle_http_response};

/// Index a project's documents — paths, sizes and ids, WITHOUT the texts.
///
/// The tree is built from the paths this returns; folders exist nowhere else. Reads only, like the board:
/// documents are written, moved, deleted and restored through `/mcp`.
pub async fn get_documents(project: &str) -> Result<DocumentsResponse, RequestError> {
    let request = GetDocumentsInputModel {
        project: project.to_string(),
    };

    handle_http_response(authed("/api/documents/v1/list", HttpVerb::Post, request).await).await
}

/// Read one document, text included.
///
/// The other half of the design: a document is not in the board snapshot, so nothing has its text until
/// somebody opens it. A deleted document comes back as a miss carrying `in_trash`, which is a different thing
/// to show than "no such document".
pub async fn get_document(
    project: &str,
    id: &str,
) -> Result<FindDocumentResponse, RequestError> {
    let request = GetDocumentInputModel {
        project: project.to_string(),
        id: id.to_string(),
    };

    handle_http_response(authed("/api/documents/v1/get", HttpVerb::Post, request).await).await
}

/// Upload a file into a project's documents.
///
/// **The only write this client makes about a document.** Everything else — moving, deleting, restoring —
/// arrives through `/mcp`, like every change to the board. This exists because the alternative is not a person
/// using MCP, it is a person unable to upload at all: a PDF on a laptop cannot reach an agent without being
/// base64-ed by hand into a tool call.
///
/// The bytes go up base64-encoded in a JSON body. It costs a third of the size on the way up, once per upload,
/// and it buys the same request path every other call here already uses.
pub async fn upload_document(
    project: &str,
    path: &str,
    bytes: &[u8],
    content_type: Option<String>,
) -> Result<UploadDocumentResponse, RequestError> {
    use rust_extensions::base64::IntoBase64;

    let request = UploadDocumentInputModel {
        project: project.to_string(),
        path: path.to_string(),
        content_base64: bytes.into_base64(),
        content_type,
    };

    handle_http_response(authed("/api/documents/v1/upload", HttpVerb::Post, request).await).await
}
