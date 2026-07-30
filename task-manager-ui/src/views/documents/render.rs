use dioxus::prelude::*;
use dioxus_utils::{DataState, RenderState};
use task_manager_shared::documents::DocumentIndexEntryResponse;
use task_manager_shared::projects::ProjectResponse;

use super::tree::{DocumentNode, all_folder_paths, build_tree};

/// The Documents screen: a project's documents as folders and files.
///
/// **Read-only, and read on demand.** Documents are the one part of the product the server does not hold in
/// memory, so this is the one screen that is not served out of the board snapshot: it asks for the index when
/// it opens, and a document's text is fetched only when a row is clicked. There is deliberately no live
/// refresh — the socket carries the board, and putting documents in it would ship every text to every open
/// screen on every write. The Refresh button is the answer to "an agent just uploaded something".
///
/// Nothing here writes. A document is uploaded, moved, deleted and restored through `/mcp`, exactly as every
/// change to the board is — and the trash is not shown at all, on purpose: it is a list you ask an agent for
/// when something needs restoring, not a place to browse.
#[component]
pub fn RenderDocuments() -> Element {
    let cs = use_signal(ComponentState::default);

    // The selection is remembered where Home and Goals remember it, so moving between tabs keeps you on the
    // board you were looking at.
    use_effect(move || {
        let project_id = cs.read().selected.clone();

        if !project_id.is_empty() {
            crate::web::storage::save_last_project(&project_id);
        }
    });

    let cs_ra = cs.read();

    let projects = match get_projects(cs, &cs_ra) {
        Ok(projects) => projects,
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

    let header = rsx! {
        RenderHeader { projects: projects.to_vec(), cs }
    };

    let documents: Vec<DocumentIndexEntryResponse> = match get_documents(cs, &cs_ra) {
        Ok(documents) => documents.to_vec(),
        Err(element) => {
            return rsx! {
                div { class: "docs-page",
                    {header}
                    {element}
                }
            };
        }
    };

    let project_id = cs_ra.selected.clone();
    let expanded = cs_ra.expanded.clone();
    drop(cs_ra);

    // Built here rather than kept in state: the tree IS the index, folded a certain way, and a second copy
    // would be a thing to keep in step with the first for no gain.
    let tree = build_tree(&documents);

    rsx! {
        div { class: "docs-page",
            {header}

            if documents.is_empty() {
                div { class: "empty-note",
                    "No documents on this project yet. They are uploaded through MCP — ask an agent to put one here."
                }
            } else {
                div { class: "docs-tree",
                    RenderNodes {
                        nodes_depth: 0,
                        nodes: std::rc::Rc::new(tree),
                        project_id,
                        expanded,
                        cs,
                    }
                }
            }
        }
    }
}

#[derive(Default)]
struct ComponentState {
    projects: DataState<Vec<ProjectResponse>>,
    selected: String,
    documents: DataState<Vec<DocumentIndexEntryResponse>>,
    /// Which folders are open, by their full path. Open by default — see `all_folder_paths`: a tree that
    /// arrived folded would hide the documents somebody came for behind one row.
    expanded: Vec<String>,
}

impl ComponentState {
    fn select(&mut self, project_id: String) {
        if self.selected == project_id {
            return;
        }

        self.selected = project_id;
        // Reset rather than clear: the next render sees `None` and loads, the same path a first visit takes.
        self.documents.reset();
        self.expanded.clear();
    }

    fn toggle(&mut self, path: &str) {
        if let Some(at) = self.expanded.iter().position(|itm| itm == path) {
            self.expanded.remove(at);
        } else {
            self.expanded.push(path.to_string());
        }
    }
}

fn get_projects(
    mut cs: Signal<ComponentState>,
    cs_ra: &ComponentState,
) -> Result<&[ProjectResponse], Element> {
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

                        let mut write = cs.write();
                        write.selected = initial;
                        write.projects.set_loaded(response.projects);
                    }
                    Err(err) => cs.write().projects.set_error(err.message),
                }
            });

            Err(render_loading())
        }
        RenderState::Loading => Err(render_loading()),
        RenderState::Loaded(projects) => Ok(projects.as_slice()),
        RenderState::Error(err) => Err(render_error(err)),
    }
}

fn get_documents(
    mut cs: Signal<ComponentState>,
    cs_ra: &ComponentState,
) -> Result<&[DocumentIndexEntryResponse], Element> {
    if cs_ra.selected.is_empty() {
        return Ok(&[]);
    }

    match cs_ra.documents.as_ref() {
        RenderState::None => {
            let project_id = cs_ra.selected.clone();

            spawn(async move {
                cs.write().documents.set_loading();

                match crate::api::get_documents(&project_id).await {
                    Ok(response) => {
                        // Every folder open, worked out from the tree the index makes — see `expanded`. Done
                        // here rather than in the render body, which must never write a signal.
                        let expanded = all_folder_paths(&build_tree(&response.documents));

                        let mut write = cs.write();
                        write.expanded = expanded;
                        write.documents.set_loaded(response.documents);
                    }
                    Err(err) => cs.write().documents.set_error(err.message),
                }
            });

            Err(render_loading())
        }
        RenderState::Loading => Err(render_loading()),
        RenderState::Loaded(documents) => Ok(documents.as_slice()),
        RenderState::Error(err) => Err(render_error(err)),
    }
}

fn render_loading() -> Element {
    rsx! {
        div { class: "loading-note", "Loading…" }
    }
}

fn render_error(message: &str) -> Element {
    rsx! {
        div { class: "error-note", "{message}" }
    }
}

#[component]
fn RenderHeader(projects: Vec<ProjectResponse>, cs: Signal<ComponentState>) -> Element {
    let selected_id = cs.read().selected.clone();
    let mut cs = cs;

    rsx! {
        div { class: "page-header",
            h1 { class: "page-title", "Documents" }
            div { class: "project-picker",
                select {
                    onchange: move |event| cs.write().select(event.value()),
                    for project in projects.iter() {
                        option {
                            value: "{project.id}",
                            selected: project.id == selected_id,
                            "{project.prefix} · {project.name}"
                        }
                    }
                }
            }
            // The one control on the screen, and it exists because nothing here is live: documents are not in
            // the board snapshot, so a document an agent uploads while this is open does not appear on its
            // own. A reset sends the next render down the loading path.
            button {
                class: "btn",
                title: "Read the index again",
                onclick: move |_| cs.write().documents.reset(),
                "Refresh"
            }
        }
    }
}

/// One level of the tree.
///
/// Recursive, and takes its children behind an `Rc` so that recursing does not copy the whole subtree into a
/// prop at every level — the nodes themselves are compared by value, so a level that did not change does not
/// repaint.
#[component]
fn RenderNodes(
    nodes: std::rc::Rc<Vec<DocumentNode>>,
    nodes_depth: usize,
    project_id: String,
    expanded: Vec<String>,
    cs: Signal<ComponentState>,
) -> Element {
    let mut cs = cs;

    // Indentation as a style rather than as nested containers: the rows then all have the same height and
    // the same hit area, whatever level they are on.
    let indent = nodes_depth * 16;

    rsx! {
        for node in nodes.iter() {
            match node {
                DocumentNode::Folder { name, path, children } => {
                    let open = expanded.iter().any(|itm| itm == path);
                    let for_toggle = path.clone();

                    rsx! {
                        div { class: "docs-node", key: "folder:{path}",
                            button {
                                class: "docs-folder",
                                style: "padding-left: {indent}px",
                                onclick: move |_| cs.write().toggle(&for_toggle),
                                span { class: "docs-caret", if open { "▾" } else { "▸" } }
                                span { class: "docs-folder-name", "{name}" }
                                span { class: "board-column-count", "{children.len()}" }
                            }

                            if open {
                                RenderNodes {
                                    nodes: std::rc::Rc::new(children.clone()),
                                    nodes_depth: nodes_depth + 1,
                                    project_id: project_id.clone(),
                                    expanded: expanded.clone(),
                                    cs,
                                }
                            }
                        }
                    }
                }
                DocumentNode::Document { name, entry } => {
                    let project_id = project_id.clone();
                    let id = entry.id.clone();

                    rsx! {
                        div { class: "docs-node", key: "doc:{entry.id}",
                            button {
                                class: "docs-file",
                                style: "padding-left: {indent}px",
                                title: "Open {entry.path}",
                                onclick: move |_| {
                                    crate::dialogs::open(crate::dialogs::DialogState::ViewDocument {
                                        project_id: project_id.clone(),
                                        id: id.clone(),
                                    });
                                },
                                span { class: "docs-file-icon", "📄" }
                                span { class: "docs-file-name", "{name}" }
                                // The size and the version, which is what a list of documents is asked: is
                                // this a note or a specification, and has anybody touched it.
                                span { class: "docs-file-meta", "{entry.size} chars · v{entry.version}" }
                                span { class: "docs-file-by", "{entry.updated_by}" }
                            }
                        }
                    }
                }
            }
        }
    }
}
