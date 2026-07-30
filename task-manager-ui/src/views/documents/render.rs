use dioxus::prelude::*;
use dioxus_utils::RenderState;
use task_manager_shared::documents::{
    DocumentResponse, FindDocumentResponse, is_previewable_content_type, raw_document_url,
    render_size,
};
use task_manager_shared::projects::ProjectResponse;

use super::{
    DocumentNodes, DocumentsState, build_tree, get_content, get_index, render_viewer_note,
};

/// The Documents screen: the tree on the left, whatever is selected on the right.
///
/// Modelled on the file browser in `remote-development-mcp`, deliberately and down to the class names — it is
/// the same problem, and a second design for it would only be a second thing to maintain. `selected` arrives
/// from the url rather than from the state, so a document can be linked to and a reload lands back on it.
///
/// **Read-only.** Documents are uploaded, moved, deleted and restored through `/mcp`, exactly as every change
/// to the board is, and the trash is not shown at all — it is a list you ask an agent for, not a place to
/// browse. There is no WebSocket either: the socket carries the board, and payloads must not ride along, so
/// Refresh is the answer to "an agent just uploaded something".
#[component]
pub fn RenderDocuments(selected: String) -> Element {
    let mut cs = use_signal(DocumentsState::default);

    // Every folder down to the selection, opened. In an effect rather than the initialiser because the
    // selection can arrive AFTER the first render — following a reference from a task does exactly that — and
    // `use_reactive!` is what makes a prop wake it, since a prop is not a signal.
    use_effect(use_reactive!(|selected| {
        if !selected.is_empty() {
            cs.write().reveal(&selected);
        }
    }));

    let cs_ra = cs.read();

    // Owned before the read guard is dropped: the picker below is drawn after it, and a borrow of the state
    // cannot outlive it — the same reason the Goals screen takes a copy.
    let projects: Vec<ProjectResponse> = match get_projects(cs, &cs_ra) {
        Ok(projects) => projects.to_vec(),
        Err(element) => return element,
    };

    if projects.is_empty() {
        return rsx! {
            div { class: "page-header",
                h1 { class: "page-title", "Documents" }
            }
            div { class: "empty-note",
                "You are not on any project yet. Ask an admin to add you to one."
            }
        };
    }

    let project_id = cs_ra.selected_project.clone();
    let show_source = cs_ra.show_source;

    // A bare `/documents` parses to an empty string, which is "nothing selected" rather than a document whose
    // id is empty.
    let selected = if selected.is_empty() {
        None
    } else {
        Some(selected)
    };

    let index = get_index(cs, &cs_ra);

    let found = selected
        .as_ref()
        .map(|id| get_content(cs, &cs_ra, &project_id, id));

    // Built here rather than kept in state: the tree IS the index, folded a certain way, and a second copy
    // would be a thing to keep in step with the first for no gain.
    let nodes = match index.as_ref() {
        Ok(entries) => build_tree(entries),
        Err(_) => Vec::new(),
    };

    let tree_note = index.err();

    let document = match found.as_ref() {
        Some(Ok(found)) => found.document.clone(),
        _ => None,
    };

    // Only a text document has two ways of being read, so only it gets the switch.
    let is_text = document.as_ref().map(|itm| !itm.is_binary).unwrap_or(false);

    // The url the frame is pointed at, and the one "open in a new tab" uses — the same address, so the tab
    // shows exactly what the pane is showing rather than a second rendering of it.
    let raw_url = document
        .as_ref()
        .map(|itm| raw_document_url(&itm.project_id, &itm.id, &token()))
        .unwrap_or_default();

    let is_framed = document
        .as_ref()
        .map(|itm| itm.is_binary && is_previewable_content_type(&itm.content_type))
        .unwrap_or(false);

    let viewer = match found {
        Some(Ok(found)) => render_found(found, show_source),
        Some(Err(note)) => note,
        None => rsx! {
            div { class: "viewer-note", "Select a document." }
        },
    };

    let head_path = document.as_ref().map(|itm| itm.path.clone());

    drop(cs_ra);

    rsx! {
        div { class: "docs-page",
            div { class: "page-header",
                h1 { class: "page-title", "Documents" }
                div { class: "project-picker",
                    select {
                        onchange: move |event| {
                            let picked = event.value();
                            crate::web::storage::save_last_project(&picked);
                            cs.write().select_project(picked);
                            // The selection is dropped in the same move: an id from the old project names
                            // nothing in the new one.
                            navigator().push(crate::AppRoute::Documents { selected: String::new() });
                        },
                        for project in projects.iter() {
                            option {
                                value: "{project.id}",
                                selected: project.id == project_id,
                                "{project.prefix} · {project.name}"
                            }
                        }
                    }
                }
                button {
                    class: "btn",
                    title: "Read the index again",
                    onclick: move |_| cs.write().refresh(),
                    "Refresh"
                }
            }

            div { class: "files-layout",
                div { class: "files-tree",
                    if let Some(note) = tree_note {
                        {note}
                    } else if nodes.is_empty() {
                        div { class: "tree-note",
                            "No documents yet — they are uploaded through MCP."
                        }
                    } else {
                        // Keyed by project: switching mounts a fresh tree rather than re-using the rows of the
                        // previous one.
                        DocumentNodes { key: "{project_id}", nodes, depth: 0, cs }
                    }
                }

                div { class: "files-viewer",
                    if let Some(path) = head_path {
                        div { class: "viewer-head",
                            span { class: "truncate", "{path}" }
                            div { class: "spacer" }
                            if is_text {
                                button {
                                    class: "viewer-toggle",
                                    onclick: move |_| cs.write().toggle_source(),
                                    if show_source { "rendered" } else { "source" }
                                }
                            }
                            // Both for a framed preview — a pdf in one column of a split view was not laid out
                            // for that — and for anything else, where it IS the download.
                            a {
                                class: "viewer-toggle",
                                href: "{raw_url}",
                                target: "_blank",
                                if is_framed { "open full screen" } else { "download" }
                            }
                        }
                    }
                    div { class: "viewer-body", {viewer} }
                }
            }
        }
    }
}

/// The session token, for the two tags that fetch a document's bytes themselves.
///
/// They cannot send an `Authorization` header, so it goes in the url — the same trade the WebSocket makes here.
fn token() -> String {
    crate::web::storage::get_session_token().unwrap_or_default()
}

fn render_found(found: &FindDocumentResponse, show_source: bool) -> Element {
    let Some(document) = found.document.as_ref() else {
        let reason = if found.not_found.is_empty() {
            "Nothing found.".to_string()
        } else {
            found.not_found.clone()
        };

        return render_viewer_note(&reason, !found.in_trash);
    };

    render_document(document, show_source)
}

fn render_document(document: &DocumentResponse, show_source: bool) -> Element {
    let raw_url = raw_document_url(&document.project_id, &document.id, &token());

    if document.is_binary {
        if document.content_type.starts_with("image/") {
            return rsx! {
                div { class: "viewer-image",
                    img { src: "{raw_url}", alt: "{document.path}" }
                }
            };
        }

        if is_previewable_content_type(&document.content_type) {
            return rsx! {
                // The browser's own viewer, which is the whole reason the raw endpoint exists: a PDF drawn by
                // Chrome beats anything this client could do with the bytes, and the bytes never enter the wasm.
                iframe { class: "viewer-frame", src: "{raw_url}" }
            };
        }

        return rsx! {
            div { class: "viewer-note",
                div { "{document.content_type} — {render_size(document.size)}" }
                div { class: "field-hint", "Not something the browser can draw." }
                a { class: "viewer-toggle", href: "{raw_url}", target: "_blank", "download" }
            }
        };
    }

    let text = document.content.clone().unwrap_or_default();

    if text.trim().is_empty() {
        // Said rather than left blank: an empty pane reads as something that failed to load, and an empty
        // document is a real thing somebody created.
        return rsx! {
            div { class: "viewer-note", "This document is empty." }
        };
    }

    if show_source {
        return rsx! {
            pre { class: "viewer-text", "{text}" }
        };
    }

    // Through the one renderer every text on this side goes through — GFM, with raw HTML escaped rather than
    // trusted. It matters more here than anywhere: a specification is where an agent actually writes a table.
    let html = crate::dialogs::md_to_html(&text);

    rsx! {
        div { class: "viewer-markdown md", dangerous_inner_html: "{html}" }
    }
}

fn get_projects<'s>(
    mut cs: Signal<DocumentsState>,
    cs_ra: &'s DocumentsState,
) -> Result<&'s [ProjectResponse], Element> {
    match cs_ra.projects.as_ref() {
        RenderState::None => {
            spawn(async move {
                cs.write().projects.set_loading();

                match crate::api::get_projects().await {
                    Ok(response) => {
                        let remembered = crate::web::storage::get_last_project();

                        let initial = remembered
                            .filter(|id| response.projects.iter().any(|itm| &itm.id == id))
                            .or_else(|| response.projects.first().map(|itm| itm.id.clone()))
                            .unwrap_or_default();

                        // The project and the folders it was left open at, decided in one write before any row
                        // is drawn — a render must not write, and by the first row everything is settled.
                        let mut write = cs.write();

                        if !initial.is_empty() {
                            write.adopt_project(initial, "");
                        }

                        write.projects.set_loaded(response.projects);
                    }
                    Err(err) => cs.write().projects.set_error(err.message),
                }
            });

            Err(render_loading())
        }
        RenderState::Loading => Err(render_loading()),
        RenderState::Loaded(projects) => Ok(projects.as_slice()),
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
