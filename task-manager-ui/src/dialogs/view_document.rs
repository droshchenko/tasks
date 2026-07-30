use dioxus::prelude::*;
use task_manager_shared::documents::render_size;

/// The block a task or a goal draws for the documents it references.
///
/// **What it has is ids, and nothing else.** The board snapshot carries the reference list and not the
/// documents, so this cannot show a path without a request — and it deliberately does not make one per card:
/// the count is drawn from what is already in hand.
///
/// A click LEAVES the dialog for the Documents screen, rather than opening a second window over it. That is
/// not a compromise: a document can be a PDF, and a PDF wants the whole pane the browser's viewer needs — a
/// modal over a board is the one place it cannot have that. The selection is a url, so the trip is linkable
/// and the back button comes straight back here.
#[component]
pub fn DocumentRefs(ids: Vec<String>) -> Element {
    rsx! {
        div { class: "task-view-attr",
            div { class: "task-view-attr-label", "Documents" }
            div { class: "doc-refs",
                for id in ids.iter() {
                    {
                        let id = id.clone();
                        rsx! {
                            button {
                                class: "doc-ref",
                                key: "{id}",
                                title: "Open this document",
                                onclick: move |_| {
                                    // Closed first: the screen underneath is about to be replaced, and a dialog
                                    // left open would hang over the document that was asked for.
                                    super::close();
                                    navigator().push(crate::AppRoute::Documents { selected: id.clone() });
                                },
                                span { class: "doc-ref-icon", "📄" }
                                span { class: "doc-ref-id", "{id}" }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// `1.2 KB` — re-exported so a dialog need not reach into the shared crate for one function.
pub fn document_size(bytes: i64) -> String {
    render_size(bytes)
}
