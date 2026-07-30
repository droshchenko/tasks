use std::sync::Arc;

use service_sdk::my_http_server::{
    HttpContext, HttpFailResult, HttpOkResult, HttpOutput, HttpServerMiddleware, WebContentType,
};
use task_manager_shared::documents::{is_html_content_type, parse_raw_document_url};

use crate::app::AppContext;
use crate::http_server::errors::not_found;
use crate::scripts::{DocumentBody, body_of, content_type_of};

/// Serves a document's bytes at `/raw/{prefix}/{path}`.
///
/// **A middleware rather than an action, and the path form rather than a query, for one reason: a framed html
/// page asks for its own assets with relative urls.** The browser resolves those against the address the page
/// came from — served as `/raw/TM/docs/page.html`, a `style.css` beside it resolves to
/// `/raw/TM/docs/style.css` and arrives. From `?project=TM&id=…` it would resolve back onto the api route and
/// arrive as nothing. A path of arbitrary depth is not something the routing macro can express, which is what
/// makes this a middleware.
///
/// **The session is a cookie, so the url carries no token.** That is the other half of the change: this url is
/// what "open full screen" hands somebody, and a token in it would travel into browser history, `Referer`
/// headers and proxy logs. As a cookie it opens for the reader only if they are signed in and on that board.
///
/// Registered before the controllers; anything that is not this route returns `None` and falls through.
pub struct RawDocumentsMiddleware {
    app: Arc<AppContext>,
}

impl RawDocumentsMiddleware {
    pub fn new(app: Arc<AppContext>) -> Self {
        Self { app }
    }
}

#[async_trait::async_trait]
impl HttpServerMiddleware for RawDocumentsMiddleware {
    async fn handle_request(
        &self,
        ctx: &mut HttpContext,
    ) -> Option<Result<HttpOkResult, HttpFailResult>> {
        let (prefix, path) = parse_raw_document_url(ctx.request.http_path.as_str())?;

        Some(serve(&self.app, ctx, &prefix, &path).await)
    }
}

async fn serve(
    app: &Arc<AppContext>,
    ctx: &HttpContext,
    prefix: &str,
    path: &str,
) -> Result<HttpOkResult, HttpFailResult> {
    // The project comes from the URL and membership is checked against it, which is what makes the url safe to
    // hand around: naming a board does not open it.
    let project_id = {
        let board = app.board.read();

        match crate::scripts::resolve_project_by_prefix(&board, prefix) {
            Ok(project) => project.id.clone(),
            // The same message a forbidden project gets: probing prefixes must not map out boards the caller
            // cannot see.
            Err(_) => return Err(not_found("No such document")),
        }
    };

    crate::auth::require_project_access(app, ctx, &project_id).await?;

    let telemetry = service_sdk::my_telemetry::MyTelemetryContext::create_empty();

    // By PATH rather than by id, because the path is what the url carries — and it is what a relative link
    // inside a framed page resolves to, which is the whole point of this route.
    let Some(row) = app
        .documents_repo
        .get_by_path(&project_id, path, &telemetry)
        .await
    else {
        return Err(not_found("No such document"));
    };

    let content_type = content_type_of(row.content_type.as_deref(), &row.doc_path);

    // Text and bytes both come out as bytes here: a text document served raw is what makes "download" work on
    // one, and its Content-Type says what it is.
    let content = match body_of(&row) {
        DocumentBody::Text(text) => text.into_bytes(),
        DocumentBody::Binary(bytes) => bytes,
    };

    // HTML is the one type served here that a browser executes, and it would execute in OUR origin. A document
    // is uploaded by an agent through `/mcp`, so an unsandboxed page would be running somebody else's script
    // beside the signed-in session.
    //
    // `sandbox allow-scripts` puts the response in an opaque origin: scripts and styles still run, so the page
    // renders as the page it is, but it cannot reach our API as the signed-in user. `allow-scripts` WITHOUT
    // `allow-same-origin` is the combination that matters — together the two let a framed document remove the
    // sandbox itself.
    //
    // On the RESPONSE and not only on the frame, because "open full screen" loads this url directly in a tab,
    // where a frame's `sandbox` attribute does not exist.
    let sandbox = if is_html_content_type(&content_type) {
        Some("sandbox allow-scripts".to_string())
    } else {
        None
    };

    HttpOutput::from_builder()
        .set_content(content)
        .set_content_type(WebContentType::Raw(content_type))
        .add_header_if_some("Content-Security-Policy", sandbox)
        // Without it a browser sniffs a type for anything whose declared one looks wrong — which is how a
        // document stored as text/plain gets executed as html, straight past the check above.
        .add_header("X-Content-Type-Options", "nosniff".to_string())
        // The url is stable but the document behind it is not: an agent can rewrite it while somebody reads.
        .add_header(
            "Cache-Control",
            "no-store, no-cache, must-revalidate, max-age=0".to_string(),
        )
        .into_ok_result(false)
}
