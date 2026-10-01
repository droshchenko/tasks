use dioxus::prelude::*;

/// The shell every signed-in screen sits in: the bar of product areas across the top, the work area
/// under it at the full width of the window.
#[component]
pub fn ContentPanel(active: &'static str, children: Element) -> Element {
    let dialog = consume_context::<Signal<crate::dialogs::DialogState>>();
    let modal_open = !matches!(&*dialog.read(), crate::dialogs::DialogState::None);
    rsx! {
        div { id: "main-panel", inert: modal_open.then_some(""),
            super::TopBar { active }
            div { class: "main-content", {children} }
        }
    }
}
