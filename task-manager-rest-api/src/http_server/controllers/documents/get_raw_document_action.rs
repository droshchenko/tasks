use std::sync::Arc;

use service_sdk::macros::use_my_http_server;
use task_manager_shared::documents::GetRawDocumentInputModel;

use crate::app::AppContext;
use crate::http_server::errors::{forbidden, not_found};
use crate::scripts::{body_of, content_type_of};

use_my_http_server!();

#[http_route(
    method: "GET",
    route: "/api/documents/v1/raw",
    controller: "Documents",
    summary: "A document's bytes",
    description: "The document itself, as bytes, with its own Content-Type. THE ONE ENDPOINT THE BROWSER CALLS WITHOUT OUR CODE IN THE LOOP: an <img> or an <iframe> is pointed at this url and fetches it, which is what makes a PDF open in the browser's own viewer and an image draw in place. It is also why the session token is a QUERY PARAMETER here — neither tag can send an Authorization header, the same trade the WebSocket makes in this product for the same reason. A GET rather than a POST for the same reason: it is a url, not a call. Works for a text document too, which is what makes 'download' work on one.",
    input_data: "GetRawDocumentInputModel",
    result: [
        {status_code: 200, description: "The bytes"},
        {status_code: 401, description: "Not authenticated"},
        {status_code: 403, description: "No access to this project"},
        {status_code: 404, description: "No such document"},
    ]
)]
pub struct GetRawDocumentAction {
    app: Arc<AppContext>,
}

impl GetRawDocumentAction {
    pub fn new(app: Arc<AppContext>) -> Self {
        Self { app }
    }
}

async fn handle_request(
    action: &GetRawDocumentAction,
    input_data: GetRawDocumentInputModel,
    _ctx: &HttpContext,
) -> Result<HttpOkResult, HttpFailResult> {
    // From the query rather than the header, and checked exactly as a header token is — see
    // `resolve_auth_user_from_token`. The envelope differs; the token does not.
    crate::auth::require_project_access_with_token(
        &action.app,
        &input_data.token,
        &input_data.project_id,
    )
    .await?;

    let telemetry = service_sdk::my_telemetry::MyTelemetryContext::create_empty();

    let Some(row) = action
        .app
        .documents_repo
        .get_by_id(input_data.id.trim(), &telemetry)
        .await
    else {
        return Err(not_found("No such document"));
    };

    if row.project_id != input_data.project_id {
        return Err(forbidden("That document belongs to another project"));
    }

    let content_type = content_type_of(row.content_type.as_deref());

    // Text and bytes both come out as bytes here: a text document served raw is what makes "download" work
    // on one, and its Content-Type says what it is.
    let content = match body_of(&row) {
        crate::scripts::DocumentBody::Text(text) => text.into_bytes(),
        crate::scripts::DocumentBody::Binary(bytes) => bytes,
    };

    // `Raw` because the type is whatever was stored — `application/pdf`, `image/webp` — and the enum's named
    // variants cover only the handful the static file server needs. No `Content-Disposition`: the default is
    // to draw it, which is what an <iframe> pointed here needs, and `attachment` would download instead. A
    // download is this same url opened in a new tab, which the viewer offers on its own.
    HttpOutput::as_content(content, Some(WebContentType::Raw(content_type))).into_ok_result(false)
}
