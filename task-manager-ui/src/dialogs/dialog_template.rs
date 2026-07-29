use dioxus::prelude::*;

use super::DialogState;

/// The wrapper every dialog sits in.
///
/// Cancel and the close cross are built in — never add them in a dialog's own body, or a dialog ends up
/// with two ways to close that can drift apart.
pub fn dialog_template(title: &str, content: Element, btn_success: Element) -> Element {
    dialog_template_ex(title, content, btn_success, None)
}

/// [`dialog_template`] with a width class — `modal-lg` or `modal-xl`.
pub fn dialog_template_ex(
    title: &str,
    content: Element,
    btn_success: Element,
    extra_class: Option<&str>,
) -> Element {
    let modal_class = match extra_class {
        Some(extra) => format!("modal {extra}"),
        None => "modal".to_string(),
    };

    let title = title.to_string();

    rsx! {
        div {
            class: "modal-backdrop",
            // Clicking the backdrop closes. The body below stops propagation, so a click inside the
            // dialog — including a drag that ends outside an input — does not count as "outside".
            onclick: move |_| close(),
            div {
                class: "{modal_class}",
                onclick: move |event| event.stop_propagation(),
                div { class: "modal-header",
                    div { class: "modal-title", "{title}" }
                    button { class: "modal-close", title: "Close", onclick: move |_| close(), "×" }
                }
                div { class: "modal-body", {content} }
                div { class: "modal-footer",
                    button { class: "btn", onclick: move |_| close(), "Cancel" }
                    {btn_success}
                }
            }
        }
    }
}

/// Closing is setting the state back to `None`, from anywhere — no dialog owns its own visibility.
pub fn close() {
    consume_context::<Signal<DialogState>>().set(DialogState::None);
}
