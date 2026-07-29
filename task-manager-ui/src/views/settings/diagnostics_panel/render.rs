use dioxus::prelude::*;
use task_manager_shared::system::DiagnosticsResponse;

/// The Diagnostics section of Settings — read-only on purpose.
///
/// There is nothing to configure here: the Google credentials, the encryption key and the admin list all
/// live in the service settings rather than the database, because credentials entered through a screen
/// that itself requires authentication would leave a fresh deployment with no way in.
///
/// What it is for is the first question when a sign-in fails — which client id was picked up and which
/// redirect URI is expected. Reading that beats guessing it from logs.
#[component]
pub fn DiagnosticsPanel() -> Element {
    let mut diagnostics = use_signal(|| None::<DiagnosticsResponse>);
    let mut allowed = use_signal(|| true);
    let mut loading = use_signal(|| true);
    let mut error = use_signal(String::new);

    use_future(move || async move {
        match crate::api::get_diagnostics().await {
            Ok(Some(response)) => {
                diagnostics.set(Some(response));
                loading.set(false);
            }
            Ok(None) => {
                allowed.set(false);
                loading.set(false);
            }
            Err(err) => {
                error.set(err.message);
                loading.set(false);
            }
        }
    });

    if *loading.read() {
        return rsx! {
            div { class: "loading-note", "Loading…" }
        };
    }

    if !*allowed.read() {
        return rsx! {
            div { class: "page-header",
                h1 { class: "page-title", "Settings" }
            }
            div { class: "empty-note", "Admins only." }
        };
    }

    let error_text = error.read().clone();
    let current = diagnostics.read().clone();

    rsx! {
        div { class: "page-header",
            h1 { class: "page-title", "Settings" }
        }
        p { class: "page-note",
            "Read-only. These values come from the service settings, not from the database — that is what lets the service authenticate before anybody has signed in. To change one, change the settings template and redeploy."
        }

        if !error_text.is_empty() {
            div { class: "error-banner", "{error_text}" }
        }

        if let Some(current) = current {
            div { class: "card",
                div { class: "card-title", "Google sign-in" }
                div { class: "table-responsive",
                    table { class: "table",
                        tbody {
                            tr {
                                th { "Configured" }
                                td {
                                    if current.google_configured {
                                        "Yes"
                                    } else {
                                        "No — client id or secret is missing, and nobody can sign in"
                                    }
                                }
                            }
                            tr {
                                th { "Client id" }
                                td { class: "mono", "{current.google_client_id}" }
                            }
                            tr {
                                th { "Redirect URI" }
                                td { class: "mono", "{current.google_redirect_uri}" }
                            }
                        }
                    }
                }
                div { class: "field-hint",
                    "The redirect URI must match the one registered in the Google console byte for byte. A mismatch is the single most common cause of a sign-in that ends on a Google error page."
                }
            }

            div { class: "card",
                div { class: "card-title", "This deployment" }
                div { class: "table-responsive",
                    table { class: "table",
                        tbody {
                            tr {
                                th { "Admins in settings" }
                                td { "{current.admins_in_settings}" }
                            }
                            tr {
                                th { "Projects" }
                                td { "{current.projects_amount}" }
                            }
                            tr {
                                th { "People on the roster" }
                                td { "{current.users_amount}" }
                            }
                            tr {
                                th { "Home tabs connected" }
                                td { "{current.connected_homes}" }
                            }
                        }
                    }
                }
                div { class: "field-hint",
                    "Admins in settings is the emergency door: on an empty database nobody is an admin in Postgres, and those addresses are what let the first person in. If it is zero and the roster is empty, nobody can get in at all."
                }
            }
        }
    }
}
