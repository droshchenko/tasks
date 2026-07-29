use dioxus::prelude::*;

use crate::AppRoute;
use crate::states::AppState;

/// The bar of product areas, across the top.
///
/// Across the top rather than down the left side because of Home: the board is a row of columns and the
/// only thing it wants is horizontal room. A left menu costs that room on every screen in order to be
/// visible on the one screen — Projects setup — where the vertical space would have been free anyway.
///
/// Projects setup, Users and Settings are hidden from a non-admin — cosmetic on its own, since the server
/// refuses them anyway, but showing a person three screens that all answer 403 is worse than showing them
/// the one that works.
#[component]
pub fn TopBar(active: &'static str) -> Element {
    let app_state = consume_context::<Signal<AppState>>();
    let app_state_ra = app_state.read();

    let is_admin = app_state_ra.is_admin();
    let whoami = app_state_ra
        .me()
        .map(|me| {
            if me.name.trim().is_empty() {
                me.email.clone()
            } else {
                format!("{} · {}", me.name, me.email)
            }
        })
        .unwrap_or_default();

    let class_of = |name: &str| {
        if name == active {
            "topbar-tab active"
        } else {
            "topbar-tab"
        }
    };

    rsx! {
        header { class: "topbar",
            div { class: "topbar-brand", "Task Manager" }
            nav { class: "topbar-tabs",
                Link { class: class_of("home"), to: AppRoute::Home {}, "Home" }
                if is_admin {
                    Link {
                        class: class_of("projects"),
                        to: AppRoute::ProjectsSetup {},
                        "Projects setup"
                    }
                    Link { class: class_of("users"), to: AppRoute::Users {}, "Users" }
                    Link { class: class_of("settings"), to: AppRoute::Settings {}, "Settings" }
                }
            }
            div { class: "topbar-right",
                // Title as well as text: the bar is one row, so a long name · email is truncated, and
                // the full address is the thing that answers "which of my Google accounts is this?".
                div { class: "topbar-whoami", title: "{whoami}", "{whoami}" }
                Link { class: "topbar-signout", to: AppRoute::Logout {}, "Sign out" }
            }
        }
    }
}
