use std::sync::Arc;

use service_sdk::macros::use_my_http_server;
use task_manager_shared::auth::{GoogleCallbackInputModel, GoogleCallbackResponse};

use crate::app::AppContext;
use crate::http_server::errors::unauthorized;

use_my_http_server!();

#[http_route(
    method: "GET",
    route: "/api/auth/v1/google-callback",
    controller: "Auth",
    summary: "Finish a Google sign-in",
    description: "Public. Exchanges the one-time code for the person's identity, checks they are allowed in, and returns a session token. Allowed in means: they are on the roster, or the settings admin list names them — in which case their row is created on the spot, which is what lets the first person into an empty database.",
    input_data: "GoogleCallbackInputModel",
    result: [
        {status_code: 200, description: "Signed in", model: "GoogleCallbackResponse"},
        {status_code: 401, description: "Google refused, or this person is not allowed in"},
    ]
)]
pub struct GoogleCallbackAction {
    app: Arc<AppContext>,
}

impl GoogleCallbackAction {
    pub fn new(app: Arc<AppContext>) -> Self {
        Self { app }
    }
}

async fn handle_request(
    action: &GoogleCallbackAction,
    input_data: GoogleCallbackInputModel,
    _ctx: &HttpContext,
) -> Result<HttpOkResult, HttpFailResult> {
    // The state has to be one we issued and have not seen come back before. Without this, a third party
    // could hand a victim's browser a code of its own choosing.
    if !crate::auth::LoginState::is_valid(&input_data.state, &action.app.session_key) {
        return Err(unauthorized("Sign-in expired — start again"));
    }

    let settings = action.app.settings_reader.get_settings().await;

    let identity = crate::auth::exchange_code(
        &settings.google_client_id,
        &settings.google_client_secret,
        &settings.google_redirect_uri,
        &input_data.code,
    )
    .await
    .map_err(|_| unauthorized("Google did not confirm the sign-in"))?;

    let admin_in_settings = action.app.is_admin_in_settings(&identity.email).await;
    let existing = action.app.board.read().get_user(&identity.email);

    match &existing {
        Some(user) if user.disabled => {
            return Err(unauthorized("This account has been disabled"));
        }
        Some(_) => {}
        // Not on the roster. Allowed only for an address the settings admin list names — that is the
        // bootstrap door, and it is the only way into a database with no users in it.
        None if admin_in_settings => {
            crate::scripts::provision_settings_admin(&action.app, &identity.email, &identity.name)
                .await;
        }
        None => {
            return Err(unauthorized("This account has not been given access"));
        }
    }

    let token = crate::auth::SessionToken::issue(identity.email).to_token(&action.app.session_key);

    HttpOutput::as_json(GoogleCallbackResponse { token }).into_ok_result(true)
}
