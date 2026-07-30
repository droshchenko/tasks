use dioxus::prelude::*;
use dioxus_utils::{DataState, RenderState};
use task_manager_shared::documents::FindDocumentResponse;

/// One document, read on demand.
///
/// **The only dialog on this side that fetches its own data**, and the reason is the one decision documents
/// are built around: a document is not in the board snapshot, so nothing on screen has its text until
/// somebody asks for it. Every other dialog is handed a model it already holds.
///
/// It is still not a write path — nothing here saves. A document is uploaded, moved, deleted and restored
/// through `/mcp`, exactly like every change to the board.
///
/// A miss is shown as the server's own prose. The case that matters is a document somebody deleted: the
/// reference on the task is deliberately not cleaned up, so this is what a reader sees, and "it is in the
/// trash" is a different thing to be told than "it is not there".
#[component]
pub fn ViewDocumentDialog(project_id: String, id: String) -> Element {
    let cs = use_signal(ComponentState::default);
    let cs_ra = cs.read();

    let found = match get_document(cs, &cs_ra, &project_id, &id) {
        Ok(found) => found,
        Err(element) => {
            return super::dialog_template_read_only("Document", element, Some("modal-task"));
        }
    };

    let Some(document) = found.document.as_ref() else {
        let reason = if found.not_found.is_empty() {
            "Nothing found.".to_string()
        } else {
            found.not_found.clone()
        };

        return super::dialog_template_read_only(
            if found.in_trash { "In the trash" } else { "Not found" },
            rsx! {
                div { class: "empty-note", "{reason}" }
            },
            Some("modal-task"),
        );
    };

    // The path is the title: it is the document's name AND where it lives, and a reader who followed a
    // reference from a task has no other way to know which of a folder's files they are looking at.
    let title = document.path.clone();

    // Rendered rather than shown as source, through the one renderer every text on this side goes through —
    // see `md_to_html`. It matters more here than anywhere: a specification is where an agent actually writes
    // a table, which CommonMark does not have and would arrive as a wall of pipes.
    let content_html = super::md_to_html(&document.content);
    let empty = document.content.trim().is_empty();

    let content = rsx! {
        div { class: "doc-view",
            div { class: "doc-view-meta",
                span { class: "tag", "v{document.version}" }
                span { class: "doc-view-by", "{document.updated_by}" }
            }

            if empty {
                // Said rather than left blank: an empty pane reads as something that failed to load, and an
                // empty document is a real thing somebody uploaded.
                div { class: "field-hint", "This document is empty." }
            } else {
                div { class: "doc-view-body md", dangerous_inner_html: "{content_html}" }
            }
        }
    };

    super::dialog_template_read_only(&title, content, Some("modal-task"))
}

#[derive(Default)]
struct ComponentState {
    document: DataState<FindDocumentResponse>,
}

fn get_document<'s>(
    mut cs: Signal<ComponentState>,
    cs_ra: &'s ComponentState,
    project_id: &str,
    id: &str,
) -> Result<&'s FindDocumentResponse, Element> {
    match cs_ra.document.as_ref() {
        RenderState::None => {
            let project_id = project_id.to_string();
            let id = id.to_string();

            spawn(async move {
                cs.write().document.set_loading();

                match crate::api::get_document(&project_id, &id).await {
                    Ok(found) => cs.write().document.set_loaded(found),
                    Err(err) => cs.write().document.set_error(err.message),
                }
            });

            Err(render_loading())
        }
        RenderState::Loading => Err(render_loading()),
        RenderState::Loaded(found) => Ok(found),
        RenderState::Error(err) => Err(rsx! {
            div { class: "error-note", "{err}" }
        }),
    }
}

fn render_loading() -> Element {
    rsx! {
        div { class: "loading-note", "Loading…" }
    }
}

/// The block a task or a goal draws for the documents it references.
///
/// **What it has is ids, and nothing else.** The board snapshot carries the reference list and not the
/// documents, so this cannot show a path without a request — and it deliberately does not make one per card:
/// the count is drawn from what is already in hand, and the text arrives when a row is clicked.
///
/// Which is why a row is labelled by its id rather than by a name. It looks unfriendly and is the honest
/// shape of what is known here; the path appears the moment the document is opened, as the dialog's title.
#[component]
pub fn DocumentRefs(project_id: String, ids: Vec<String>) -> Element {
    rsx! {
        div { class: "task-view-attr",
            div { class: "task-view-attr-label", "Documents" }
            div { class: "doc-refs",
                for id in ids.iter() {
                    {
                        let project_id = project_id.clone();
                        let id = id.clone();
                        rsx! {
                            button {
                                class: "doc-ref",
                                key: "{id}",
                                title: "Open this document",
                                onclick: move |_| {
                                    crate::dialogs::open(crate::dialogs::DialogState::ViewDocument {
                                        project_id: project_id.clone(),
                                        id: id.clone(),
                                    });
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
