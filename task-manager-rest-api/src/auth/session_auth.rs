use std::sync::Arc;

use crate::app::AppContext;
use crate::http_server::errors::{forbidden, unauthorized};

service_sdk::macros::use_my_http_server!();

/// The resolved caller.
pub struct AuthUser {
    pub email: String,
    pub name: String,
    /// The answer, not the stored flag: the `admin` column on the user row **or** membership of the
    /// settings admin list. Callers gate on this and should not have to know which one it came from.
    pub is_admin: bool,
}

/// Pull the session token out of the `Authorization` header.
pub fn extract_bearer(ctx: &HttpContext) -> Option<String> {
    let raw = ctx
        .request
        .get_headers()
        .try_get_case_insensitive_as_str("Authorization")
        .ok()
        .flatten()?;

    let raw = raw.trim();
    let token = raw
        .strip_prefix("Bearer ")
        .or_else(|| raw.strip_prefix("bearer "))
        .unwrap_or(raw)
        .trim();

    if token.is_empty() {
        None
    } else {
        Some(token.to_string())
    }
}

/// Resolve the caller from the session token.
///
/// The user row is re-read on every request rather than trusted from the session, so disabling someone
/// or taking their admin flag away takes effect immediately instead of when their token happens to
/// expire. Reads are from memory, so this costs nothing.
pub async fn resolve_auth_user(
    app: &Arc<AppContext>,
    ctx: &HttpContext,
) -> Result<AuthUser, HttpFailResult> {
    let Some(token) = extract_bearer(ctx) else {
        return Err(unauthorized("Not authenticated"));
    };

    let Some(session) = crate::auth::SessionToken::parse(&token, &app.session_key) else {
        return Err(unauthorized("Not authenticated"));
    };

    let admin_in_settings = app.is_admin_in_settings(&session.email).await;

    match app.board.read().get_user(&session.email) {
        Some(user) => {
            if user.disabled {
                return Err(unauthorized("Account is disabled"));
            }

            Ok(AuthUser {
                email: user.email.clone(),
                name: user.name.clone(),
                is_admin: user.admin || admin_in_settings,
            })
        }
        // No row, but the settings list names them: that is the bootstrap case, and refusing it would
        // leave an empty database with no way in. Everyone else needs a row.
        None if admin_in_settings => Ok(AuthUser {
            email: session.email.clone(),
            name: String::new(),
            is_admin: true,
        }),
        None => Err(unauthorized("Not authenticated")),
    }
}

/// For the endpoints that configure the product. Hiding a menu entry in the UI is cosmetic — this is
/// what actually enforces it.
pub async fn require_admin(
    app: &Arc<AppContext>,
    ctx: &HttpContext,
) -> Result<AuthUser, HttpFailResult> {
    let user = resolve_auth_user(app, ctx).await?;

    if !user.is_admin {
        return Err(forbidden("Admin only"));
    }

    Ok(user)
}

/// Resolve the caller and check they may look at this board, in one call.
///
/// One function rather than two because every board read needs both, and because a `Result` whose error
/// is an `HttpFailResult` and whose success is `()` is a shape clippy rightly objects to — the error
/// dwarfs the value it is wrapped with.
pub async fn require_project_access(
    app: &Arc<AppContext>,
    ctx: &HttpContext,
    project_id: &str,
) -> Result<AuthUser, HttpFailResult> {
    let user = resolve_auth_user(app, ctx).await?;

    if user.is_admin {
        return Ok(user);
    }

    let is_member = app
        .board
        .read()
        .get_project(project_id)
        .map(|project| project.is_member(&user.email))
        .unwrap_or(false);

    if is_member {
        Ok(user)
    } else {
        // The same message a missing project gets, so probing ids cannot map out boards the caller has
        // no access to.
        Err(forbidden("No access to this project"))
    }
}
