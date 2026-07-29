use std::sync::Arc;

use service_sdk::macros::use_my_http_server;

use crate::app::AppContext;

use_my_http_server!();

#[http_route(
    method: "POST",
    route: "/api/auth/v1/logout",
    controller: "Auth",
    summary: "Sign out",
    description: "Always succeeds, and does nothing on the server. A session is carried inside its token rather than stored, so signing out means the client discards it. Revoking a live token is not possible by design — the TTL is short, and disabling a person locks them out on their very next request, which is what revocation would actually be for.",
    result: [
        {status_code: 204, description: "Signed out"},
    ]
)]
pub struct LogoutAction {
    _app: Arc<AppContext>,
}

impl LogoutAction {
    pub fn new(app: Arc<AppContext>) -> Self {
        Self { _app: app }
    }
}

async fn handle_request(
    _action: &LogoutAction,
    _ctx: &HttpContext,
) -> Result<HttpOkResult, HttpFailResult> {
    // Nothing to do server-side: the session lives inside the token, so signing out is the client
    // dropping it. The endpoint exists so the UI has one thing to call and so this is written down
    // somewhere other than a comment in the client.
    HttpOutput::Empty.into_ok_result(true)
}
