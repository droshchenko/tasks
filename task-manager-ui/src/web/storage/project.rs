use task_manager_shared::auth::PROJECT_COOKIE;
use wasm_bindgen::JsCast;

/// Which board was open last, by PREFIX.
///
/// **A cookie rather than local storage, and the prefix rather than the id.** The cookie is readable by the
/// server, which is what lets it know which board a browser is on without the client saying so on every
/// request; the prefix is what a person calls a board everywhere else in this product, and the internal id has
/// no business crossing into a url or a cookie.
///
/// It is a PREFERENCE and never an authority: every request that acts on a project checks membership of the
/// project it names, so hand-editing this cookie opens nothing its owner could not already open. That is what
/// makes it safe for the client to write.
///
/// Not `HttpOnly`, for the same reason — the client is what sets it.
pub fn save_last_project(prefix: &str) {
    // A year: it is a preference, and one that should survive a laptop being shut. `SameSite=Lax` so it is
    // not sent from another site, `path=/` so every screen sees the same value.
    set_cookie(&format!(
        "{PROJECT_COOKIE}={prefix}; path=/; max-age={}; SameSite=Lax",
        365 * 24 * 60 * 60
    ));
}

pub fn get_last_project() -> Option<String> {
    let cookies = read_cookies()?;

    cookies
        .split(';')
        .filter_map(|pair| pair.split_once('='))
        .find(|(name, _)| name.trim() == PROJECT_COOKIE)
        .map(|(_, value)| value.trim().to_string())
        .filter(|itm| !itm.is_empty())
}

fn read_cookies() -> Option<String> {
    web_sys::window()?
        .document()?
        .dyn_into::<web_sys::HtmlDocument>()
        .ok()?
        .cookie()
        .ok()
}

fn set_cookie(value: &str) {
    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return;
    };

    if let Ok(document) = document.dyn_into::<web_sys::HtmlDocument>() {
        let _ = document.set_cookie(value);
    }
}
