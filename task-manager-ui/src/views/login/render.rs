use dioxus::prelude::*;

/// The only pre-auth screen: one button.
///
/// There is nothing to type — no password lives here, and the address is whatever Google says it is.
#[component]
pub fn RenderLogin() -> Element {
    let mut error = use_signal(String::new);
    let mut going = use_signal(|| false);

    let start_login = move |_| {
        going.set(true);
        error.set(String::new());

        spawn(async move {
            match crate::api::get_google_auth_url().await {
                Ok(response) => crate::web::navigate_to(response.url.as_str()),
                Err(err) => {
                    going.set(false);
                    error.set(err.message);
                }
            }
        });
    };

    let error_text = error.read().clone();
    let is_going = *going.read();

    rsx! {
        div { class: "full-screen",
            div { class: "full-screen-form",
                h1 { class: "login-title", "Task Manager" }
                p { class: "login-note",
                    "The board is worked by agents and configured here. Sign in with the Google account an admin put on the roster."
                }

                if !error_text.is_empty() {
                    div { class: "error-banner", "{error_text}" }
                }

                button {
                    class: "btn btn-primary btn-google",
                    disabled: is_going,
                    onclick: start_login,
                    if is_going {
                        "Redirecting…"
                    } else {
                        "Sign in with Google"
                    }
                }
            }
        }
    }
}
