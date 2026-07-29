use dioxus::prelude::*;

use crate::AppRoute;
use crate::states::AppState;

/// The left menu of product areas.
///
/// Projects setup, Users and Settings are hidden from a non-admin — cosmetic on its own, since the
/// server refuses them anyway, but showing a person three screens that all answer 403 is worse than
/// showing them the one that works.
#[component]
pub fn MenuPanel(active: &'static str) -> Element {
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
            "sidebar-link active"
        } else {
            "sidebar-link"
        }
    };

    rsx! {
        div { class: "sidebar",
            div { class: "sidebar-logo", "Task Manager" }
            nav { class: "sidebar-nav",
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
            div { class: "sidebar-bottom",
                div { class: "sidebar-whoami", "{whoami}" }
                Link { class: "sidebar-link", to: AppRoute::Logout {}, "Sign out" }
            }
        }
    }
}
