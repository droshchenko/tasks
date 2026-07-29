use dioxus::prelude::*;

use crate::AppRoute;

/// Where Google drops the browser back.
///
/// Exchanges the code for a session token, stores it and goes to Home. The URL is replaced on the way so
/// a reload does not re-submit a spent code — Google refuses it the second time, and the person would be
/// looking at a failure that is not one.
///
/// `code`, `state` and `error` arrive as route props rather than being read off `window.location`. The
/// first version read the location, which the router had already stripped, so every sign-in reported
/// "Google did not return a sign-in" — see the route declaration in `main.rs`.
#[component]
pub fn RenderAuthCallback(code: String, state: String, error: String) -> Element {
    let mut failure = use_signal(String::new);

    use_future(move || {
        // Decoded here, because nothing before this point does it: the router splits the raw query on
        // `&` and `=` and hands the slice over untouched, and `location.search()` is the encoded query by
        // definition. What comes in is `4%2F0A…` and `fYpZ…%2F%2BsXa…`.
        //
        // Passing those on would not fail here — it would fail one hop later and blame Google. The
        // server hands the code to FlUrl's `UrlEncodedBody`, which escapes `%` as `%25`, so a code left
        // encoded reaches Google as `4%252F0A…`; Google decodes one layer, sees `4%2F0A…`, and answers
        // `invalid_grant`. The state would fail its own check just as quietly, being base64 with `/` and
        // `+` in it.
        let code = decode_callback_value(&code);
        let state = decode_callback_value(&state);
        let error = decode_callback_value(&error);

        async move {
            // Google's own refusal, and the ordinary case of it is somebody pressing Cancel on the
            // consent screen. Reported as itself rather than as a fault: there is nothing to retry
            // differently, and "something went wrong" would be a lie about a deliberate choice.
            if !error.is_empty() {
                failure.set(if error == "access_denied" {
                    "Sign-in was cancelled.".to_string()
                } else {
                    format!("Google refused the sign-in: {error}")
                });
                return;
            }

            if code.is_empty() || state.is_empty() {
                failure.set("Google did not return a sign-in to complete.".to_string());
                return;
            }

            match crate::api::finish_google_login(&code, &state).await {
                Ok(response) => {
                    crate::web::storage::save_session_token(&response.token);
                    replace_url_with_root();
                    navigator().push(AppRoute::Home {});
                }
                Err(err) => failure.set(err.message),
            }
        }
    });

    let failure_text = failure.read().clone();

    rsx! {
        div { class: "full-screen",
            div { class: "full-screen-form",
                if failure_text.is_empty() {
                    div { class: "loading-note", "Signing in…" }
                } else {
                    h1 { class: "login-title", "Could not sign in" }
                    div { class: "error-banner", "{failure_text}" }
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

/// Undo the encoding an OAuth callback parameter arrives in.
///
/// `+` becomes a space before percent-decoding, which is not a generic-URI rule but is the right one
/// here: RFC 6749 §4.1.2 says the callback's query is `application/x-www-form-urlencoded`, and in that
/// format `+` IS a space. The order matters — a `+` recovered from `%2B` must not then be read as a
/// space, and our own `state` is base64 that regularly contains one.
fn decode_callback_value(src: &str) -> String {
    let with_spaces = src.replace('+', " ");

    percent_encoding::percent_decode_str(&with_spaces)
        .decode_utf8_lossy()
        .trim()
        .to_string()
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The values off the real failed callback. Both carry `/`, the state also a `+` — the characters
    /// that made the first version fail.
    #[test]
    fn a_real_callback_decodes() {
        assert_eq!(
            decode_callback_value(
                "4%2F0AXEQxICCHfSoVEzGIlLkSCcJCyFfMob0TPBZpldYie1En6hAs8eAl-FFdMgcPU6ekDH1Cg"
            ),
            "4/0AXEQxICCHfSoVEzGIlLkSCcJCyFfMob0TPBZpldYie1En6hAs8eAl-FFdMgcPU6ekDH1Cg"
        );

        assert_eq!(
            decode_callback_value(
                "fYpZzi9qlF%2FNRoFG0329QdzBp2WJfTE9QuHE7f6Dpeucmj9H21%2F%2BsXa4SuOf8FvJ"
            ),
            "fYpZzi9qlF/NRoFG0329QdzBp2WJfTE9QuHE7f6Dpeucmj9H21/+sXa4SuOf8FvJ"
        );
    }

    /// `%2B` has to survive as `+`, and a bare `+` has to become a space. Swapping the two steps breaks
    /// the first case, and the base64 state would then fail to decrypt — which surfaces as "sign in
    /// again", not as a bug.
    #[test]
    fn an_encoded_plus_stays_a_plus_and_a_bare_one_is_a_space() {
        assert_eq!(decode_callback_value("a%2Bb"), "a+b");
        assert_eq!(decode_callback_value("a+b"), "a b");
    }

    /// An absent parameter is an empty prop, not a missing one — the caller distinguishes "no code" from
    /// "a code", so whitespace must not read as present.
    #[test]
    fn nothing_decodes_to_nothing() {
        assert!(decode_callback_value("").is_empty());
        assert!(decode_callback_value("+").is_empty());
        assert!(decode_callback_value("%20").is_empty());
    }
}
