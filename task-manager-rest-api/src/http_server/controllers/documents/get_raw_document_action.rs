use std::sync::Arc;

use service_sdk::macros::use_my_http_server;
use task_manager_shared::documents::GetRawDocumentInputModel;

use crate::app::AppContext;
use crate::http_server::errors::{forbidden, not_found};
use crate::scripts::{body_of, content_type_of};
use task_manager_shared::documents::is_html_content_type;

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

    let content_type = content_type_of(row.content_type.as_deref(), &row.doc_path);

    // Text and bytes both come out as bytes here: a text document served raw is what makes "download" work
    // on one, and its Content-Type says what it is.
    let content = match body_of(&row) {
        crate::scripts::DocumentBody::Text(text) => text.into_bytes(),
        crate::scripts::DocumentBody::Binary(bytes) => bytes,
    };

    // HTML is the one type served here that can execute in a browser, and it would execute in OUR origin —
    // where the session token lives, in local storage. A document is uploaded by an agent through `/mcp`, so
    // an unsandboxed page would let whatever wrote it read every viewer's token.
    //
    // `sandbox allow-scripts` puts the response in an opaque origin: scripts and styles still run, so the page
    // renders as the page it is, but it cannot reach our storage or call our API as the signed-in user.
    // `allow-scripts` WITHOUT `allow-same-origin` is the combination that matters — together, the two let a
    // framed document remove the sandbox itself.
    //
    // On the RESPONSE and not only on the frame, because "open full screen" loads this url directly in a tab,
    // where a frame's `sandbox` attribute does not exist. Both halves are needed and they key off the same
    // question.
    let sandbox = if is_html_content_type(&content_type) {
        Some("sandbox allow-scripts")
    } else {
        None
    };

    // `Raw` because the type is whatever was stored — `application/pdf`, `image/webp` — and the enum's named
    // variants cover only the handful a static file server needs. No `Content-Disposition`: the default is to
    // draw it, which is what an <iframe> pointed here needs, and `attachment` would download instead.
    HttpOutput::from_builder()
        .set_content(content)
        .set_content_type(WebContentType::Raw(content_type))
        // Without it a browser sniffs a type for anything whose declared one looks wrong — which is how a
        // document stored as text/plain gets executed as html, straight past the check above.
        .add_header_if_some("Content-Security-Policy", sandbox.map(|itm| itm.to_string()))
        .add_header("X-Content-Type-Options", "nosniff".to_string())
        // The url is stable but the document behind it is not: an agent can rewrite it while somebody is
        // reading. A cached response would show a version that no longer exists with no way to tell.
        .add_header(
            "Cache-Control",
            "no-store, no-cache, must-revalidate, max-age=0".to_string(),
        )
        .into_ok_result(false)
}
