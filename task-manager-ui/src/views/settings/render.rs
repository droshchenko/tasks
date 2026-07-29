use dioxus::prelude::*;

use crate::AppRoute;

/// Where a bare `/settings` lands.
const DEFAULT_SECTION: &str = SECTION_COLUMN_TEMPLATES;

const SECTION_COLUMN_TEMPLATES: &str = "column-templates";
const SECTION_DIAGNOSTICS: &str = "diagnostics";

/// Settings — a menu of areas on the left, the chosen one on the right.
///
/// The section comes off the route rather than out of a signal, so every area is linkable and the browser's
/// back button works between them: /settings/column-templates, /settings/diagnostics.
#[component]
pub fn RenderSettings() -> Element {
    let section = match use_route::<AppRoute>() {
        AppRoute::SettingsSection { section } => section,
        _ => DEFAULT_SECTION.to_string(),
    };

    rsx! {
        div { class: "page-header",
            h1 { class: "page-title", "Settings" }
        }

        div { class: "settings-layout",
            nav { class: "settings-menu",
                RenderMenuLink {
                    section: SECTION_COLUMN_TEMPLATES.to_string(),
                    title: "Column templates".to_string(),
                    active: section == SECTION_COLUMN_TEMPLATES,
                }
                RenderMenuLink {
                    section: SECTION_DIAGNOSTICS.to_string(),
                    title: "Diagnostics".to_string(),
                    active: section == SECTION_DIAGNOSTICS,
                }
            }

            div { class: "settings-content",
                if section == SECTION_COLUMN_TEMPLATES {
                    super::ColumnTemplatesPanel {}
                } else if section == SECTION_DIAGNOSTICS {
                    super::DiagnosticsPanel {}
                } else {
                    // A stale bookmark to a section that no longer exists lands here.
                    div { class: "empty-note",
                        "That settings section does not exist. Pick one on the left."
                    }
                }
            }
        }
    }
}

#[component]
fn RenderMenuLink(section: String, title: String, active: bool) -> Element {
    rsx! {
        Link {
            class: if active { "settings-menu-link active" } else { "settings-menu-link" },
            to: AppRoute::SettingsSection { section },
            "{title}"
        }
    }
}
