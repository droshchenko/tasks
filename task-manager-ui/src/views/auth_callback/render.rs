use dioxus::prelude::*;

use crate::AppRoute;

/// Where Google drops the browser back.
///
/// Reads `?code=` and `?state=` off the URL, exchanges them for a session token, stores it and goes to
/// Home. The URL is replaced on the way so a reload does not re-submit a spent code — Google refuses it
/// the second time, and the person would be looking at a failure that is not one.
#[component]
pub fn RenderAuthCallback() -> Element {
    let mut error = use_signal(String::new);

    use_future(move || async move {
        let query = read_query();

        let code = read_param(&query, "code").unwrap_or_default();
        let state = read_param(&query, "state").unwrap_or_default();

        if code.is_empty() || state.is_empty() {
            error.set("Google did not return a sign-in to complete.".to_string());
            return;
        }

        match crate::api::finish_google_login(&code, &state).await {
            Ok(response) => {
                crate::web::storage::save_session_token(&response.token);
                replace_url_with_root();
                navigator().push(AppRoute::Home {});
            }
            Err(err) => error.set(err.message),
        }
    });

    let error_text = error.read().clone();

    rsx! {
        div { class: "full-screen",
            div { class: "full-screen-form",
                if error_text.is_empty() {
                    div { class: "loading-note", "Signing in…" }
                } else {
                    h1 { class: "login-title", "Could not sign in" }
                    div { class: "error-banner", "{error_text}" }
                    button {
                        class: "btn btn-primary btn-google",
                        onclick: move |_| {
                            replace_url_with_root();
                            navigator().push(AppRoute::Home {});
                        },
                        "Try again"
                    }
                }
            }
        }
    }
}

fn read_query() -> String {
    web_sys::window()
        .and_then(|window| window.location().search().ok())
        .unwrap_or_default()
}

/// Read one parameter out of a `?a=1&b=2` string.
///
/// Hand-rolled rather than pulled from a crate: the two values read here are a Google authorization code
/// and a base64 state, neither of which is percent-encoded in a way that matters, and the alternative is
/// a dependency for six lines.
fn read_param(query: &str, name: &str) -> Option<String> {
    let query = query.trim_start_matches('?');
    let prefix = format!("{name}=");

    query
        .split('&')
        .find_map(|pair| pair.strip_prefix(prefix.as_str()))
        .map(|value| value.replace('+', " "))
        .filter(|value| !value.is_empty())
}

/// Drop the query string without reloading, so the spent code is not left sitting in the address bar.
fn replace_url_with_root() {
    if let Some(window) = web_sys::window()
        && let Ok(history) = window.history()
    {
        let _ = history.replace_state_with_url(&wasm_state(), "", Some("/"));
    }
}

fn wasm_state() -> js_sys::JsString {
    js_sys::JsString::from("")
}
