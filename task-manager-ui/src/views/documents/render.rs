use dioxus::prelude::*;
use dioxus_utils::RenderState;
use task_manager_shared::documents::{
    DocumentResponse, FindDocumentResponse, is_framed_content_type, is_html_content_type,
    is_image_content_type, raw_document_url, render_size,
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

    // Any text document has two ways of being read, so it gets the switch — INCLUDING html, which is drawn as
    // the page it is and whose markup is then one click away.
    let is_text = document.as_ref().map(|itm| !itm.is_binary).unwrap_or(false);

    // The url the frame is pointed at, and the one "open in a new tab" uses — the same address, so the tab
    // shows exactly what the pane is showing rather than a second rendering of it.
    let raw_url = document
        .as_ref()
        .map(|itm| raw_document_url(&itm.project_id, &itm.id, &token()))
        .unwrap_or_default();

    // By content type, not by `is_binary`. That was the bug: html is stored as TEXT — it is diffable and
    // searchable, so it belongs in the text column — and framing only binary payloads showed a web page as a
    // wall of markup.
    let is_framed = document
        .as_ref()
        .map(|itm| is_framed_content_type(&itm.content_type) && !show_source)
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

/// Whether a text document should be RENDERED as Markdown rather than shown as it was written.
///
/// Only Markdown is: a csv, a config or a page's markup put through a Markdown renderer comes out mangled, and
/// what a reader wants from those is the file.
fn is_markdown_content_type(content_type: &str) -> bool {
    content_type == "text/markdown" || content_type.starts_with("text/markdown;")
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

/// How one document is drawn.
///
/// **Decided by the content type, and only then by whether there is text to fall back to.** The order matters
/// and getting it wrong is what this function was rewritten for: html is text, so a viewer that asked
/// `is_binary` first drew a web page as markup.
fn render_document(document: &DocumentResponse, show_source: bool) -> Element {
    let raw_url = raw_document_url(&document.project_id, &document.id, &token());

    if is_image_content_type(&document.content_type) {
        return rsx! {
            div { class: "viewer-image",
                img { src: "{raw_url}", alt: "{document.path}" }
            }
        };
    }

    // A frame, unless the reader asked for the source — which only a text document has, so `show_source` on a
    // PDF is not a state that exists.
    if is_framed_content_type(&document.content_type) && !show_source {
        // HTML is sandboxed and a PDF is not, and the difference is not an oversight.
        //
        // HTML served from our own origin executes in it, and the session token is in this origin's local
        // storage — so a document an agent uploaded could read every viewer's token. `allow-scripts` WITHOUT
        // `allow-same-origin` is the combination that fixes it: the page still runs its own scripts and
        // stylesheets, so it renders as the page it is, but it sits in an opaque origin with no way back to
        // our storage or our API. Granting both would let the document drop the sandbox itself.
        //
        // A PDF gets none of that. Its scripts run inside the browser's PDF viewer rather than in our page,
        // where they cannot reach anything of ours — and a sandbox on the response breaks that viewer. The
        // matching `Content-Security-Policy` on the raw endpoint keys off the same distinction, which is what
        // covers this url being opened directly in a tab, where the attribute below does not exist.
        let sandbox = is_html_content_type(&document.content_type);

        return rsx! {
            if sandbox {
                iframe {
                    class: "viewer-frame",
                    src: "{raw_url}",
                    // Quoted because `dioxus_elements::iframe` does not declare this attribute — the string
                    // form is how a custom one is written, and it lands on the tag verbatim.
                    "sandbox": "allow-scripts",
                }
            } else {
                iframe { class: "viewer-frame", src: "{raw_url}" }
            }
        };
    }

    if document.is_binary {
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

    // The source, either because the reader asked for it or because this is a text document that is not
    // Markdown — html markup, a csv, a config. Running those through a Markdown renderer would mangle them.
    if show_source || !is_markdown_content_type(&document.content_type) {
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
