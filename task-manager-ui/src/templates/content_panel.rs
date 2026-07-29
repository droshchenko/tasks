use dioxus::prelude::*;

/// The shell every signed-in screen sits in: menu on the left, work area on the right.
#[component]
pub fn ContentPanel(active: &'static str, children: Element) -> Element {
    rsx! {
        super::MenuPanel { active }
        div { class: "main-content", {children} }
    }
}
