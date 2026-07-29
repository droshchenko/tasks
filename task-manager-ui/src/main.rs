use dioxus::prelude::*;
use futures::{SinkExt, StreamExt};
use reqwasm::websocket::{Message, futures::WebSocket};

mod api;
mod models;
mod states;
mod templates;
mod views;
mod web;

use models::ServerWsMessage;
use states::{AppState, SignedIn};

#[derive(Routable, PartialEq, Clone)]
pub enum AppRoute {
    #[route("/")]
    Home {},
    // Where Google sends the browser back. The path is NOT free to choose: it has to match the
    // `google-redirect-uri` secret, which in turn has to match the Google console byte for byte.
    //
    // A route rather than a server redirect so the token exchange and storing the token happen in the same
    // place the rest of the client lives.
    //
    // The query arguments MUST be declared here, and they arrive as PROPS — reading
    // `window.location().search()` in a hook is too late. A route that does not name them gets the query
    // stripped: the address bar ends up at a bare `/authorized` and the search string comes back empty,
    // which reads exactly like "Google sent nothing". That is what the first version did.
    //
    // Google also appends `iss`, `scope`, `authuser` and `prompt`, which we neither send nor read —
    // undeclared query arguments are ignored, so they are harmless.
    #[route("/authorized?:code&:state&:error")]
    AuthCallback {
        code: String,
        state: String,
        error: String,
    },
    #[route("/projects-setup")]
    ProjectsSetup {},
    #[route("/users")]
    Users {},
    #[route("/settings")]
    Settings {},
    #[route("/logout")]
    Logout {},
}

fn main() {
    dioxus::LaunchBuilder::new().launch(|| {
        rsx! {
            document::Link { rel: "icon", href: asset!("/public/favicon.ico") }
            document::Meta {
                name: "viewport",
                content: "width=device-width, initial-scale=1.0",
            }
            Router::<AppRoute> {}
        }
    });
}

/// Everything below the router shares one `AppState`, and every screen goes through [`Shell`].
///
/// `Shell` owns the one question every screen needs answered first — is anybody signed in — so no
/// individual view has to handle "not yet asked", and none of them can forget to.
#[component]
fn Shell(active: &'static str, children: Element) -> Element {
    let mut app_state = use_context_provider(|| Signal::new(AppState::default()));

    let signed_in = app_state.read().signed_in.clone();

    // Asked once per page load. `/me` is the only way to find out: the token in localStorage may be
    // expired, encrypted with a rotated key, or belong to somebody since disabled — all of which look
    // identical from here and are all answered by asking.
    use_future(move || async move {
        if app_state.read().signed_in != SignedIn::Unknown {
            return;
        }

        match crate::api::get_me().await {
            Ok(Some(me)) => app_state.write().signed_in = SignedIn::Yes(me),
            Ok(None) => {
                crate::web::storage::clear_session_token();
                app_state.write().signed_in = SignedIn::No;
            }
            // A transport failure is not "signed out" — showing the login screen would hide the real
            // problem behind a button that cannot help.
            Err(err) => {
                crate::web::console_log(format!("/me failed: {err}").as_str());
                app_state.write().signed_in = SignedIn::No;
            }
        }
    });

    match signed_in {
        SignedIn::Unknown => rsx! {
            div { class: "full-screen",
                div { class: "loading-note", "Loading…" }
            }
        },
        SignedIn::No => rsx! {
            crate::views::login::RenderLogin {}
        },
        SignedIn::Yes(_) => {
            if !app_state.read().ws_started {
                app_state.write().ws_started = true;
                kick_off_ws(app_state);
            }

            rsx! {
                crate::templates::ContentPanel { active, {children} }
            }
        }
    }
}

#[component]
fn Home() -> Element {
    rsx! {
        Shell { active: "home",
            crate::views::home::RenderHome {}
        }
    }
}

#[component]
fn ProjectsSetup() -> Element {
    rsx! {
        Shell { active: "projects",
            crate::views::projects_setup::RenderProjectsSetup {}
        }
    }
}

#[component]
fn Users() -> Element {
    rsx! {
        Shell { active: "users",
            crate::views::users::RenderUsers {}
        }
    }
}

#[component]
fn Settings() -> Element {
    rsx! {
        Shell { active: "settings",
            crate::views::settings::RenderSettings {}
        }
    }
}

/// Not wrapped in `Shell`: it runs before there is a session, and asking `/me` first would only fail.
#[component]
fn AuthCallback(code: String, state: String, error: String) -> Element {
    rsx! {
        crate::views::auth_callback::RenderAuthCallback { code, state, error }
    }
}

#[component]
fn Logout() -> Element {
    rsx! {
        crate::views::logout::RenderLogout {}
    }
}

/// Hold the WebSocket open and bump `board_revision` when the server says a board changed.
///
/// One socket for the whole session, opened once the first signed-in screen renders. It carries the
/// session token as a query parameter because the browser WebSocket API cannot send headers.
///
/// No reconnect loop yet: a dropped socket leaves the board static until the page is reloaded, and the
/// dot in the header goes grey so that is visible rather than silent.
fn kick_off_ws(mut app_state: Signal<AppState>) {
    spawn(async move {
        let token = crate::web::storage::get_session_token().unwrap_or_default();

        // Bound before use: `get_origin` borrows from the settings, so calling it on a temporary would
        // not outlive the statement.
        let settings = dioxus_utils::js::GlobalAppSettings::new();
        let origin = settings.get_origin();

        let ws_origin = if origin.starts_with("https") {
            origin.replacen("https", "wss", 1)
        } else {
            origin.replacen("http", "ws", 1)
        };

        let ws_url = if ws_origin.ends_with('/') {
            format!("{ws_origin}ws?token={token}")
        } else {
            format!("{ws_origin}/ws?token={token}")
        };

        // The channel is installed before the socket opens, so a `watch` sent by Home while the
        // handshake is still in flight waits in it rather than being dropped.
        let mut outgoing = crate::web::install_ws_sender();

        match WebSocket::open(&ws_url) {
            Ok(ws) => {
                app_state.write().ws_live = true;

                // Split so sending and receiving are independent: the read half blocks on the server, and
                // a `watch` must not have to wait behind it.
                let (mut write, mut read) = ws.split();

                spawn(async move {
                    while let Some(payload) = outgoing.next().await {
                        if write.send(Message::Text(payload)).await.is_err() {
                            break;
                        }
                    }
                });

                while let Some(message) = read.next().await {
                    match message {
                        Ok(Message::Text(text)) => match ServerWsMessage::parse(&text) {
                            // Bumping a counter rather than applying anything: the view watches it and
                            // re-reads the board it is showing, so the two cannot disagree.
                            ServerWsMessage::ProjectChanged => {
                                app_state.write().board_revision += 1;
                            }
                            ServerWsMessage::Error(err) => {
                                crate::web::console_log(format!("ws: {err}").as_str());
                            }
                            ServerWsMessage::Unknown(raw) => {
                                crate::web::console_log(format!("ws: unknown {raw}").as_str());
                            }
                        },
                        Ok(Message::Bytes(_)) => {}
                        Err(err) => {
                            crate::web::console_log(format!("ws error: {err:?}").as_str());
                            break;
                        }
                    }
                }

                app_state.write().ws_live = false;
            }
            Err(err) => {
                crate::web::console_log(format!("cannot open ws: {err:?}").as_str());
            }
        }
    });
}
