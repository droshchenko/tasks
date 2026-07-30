use dioxus::prelude::*;
use task_manager_shared::documents::{DocumentIndexEntryResponse, document_file_name, render_size};

use super::{DocumentsState, file_icon, folder_icon};

/// How far one level is pushed in, in pixels.
const INDENT: usize = 14;

/// One node of the tree: a folder with children, or a document.
///
/// **Built from the index on every render, and stored nowhere.** That is not a shortcut, it is the model: a
/// folder does not exist on the server either — it is read off the paths of the documents in it. So an empty
/// folder cannot appear here, and the last document leaving one makes it vanish with nothing to clean up.
#[derive(Clone, PartialEq)]
pub enum DocumentNode {
    Folder {
        /// The folder's own name — the last segment, which is what a row shows.
        name: String,
        /// The full path from the root, which is what the open/closed set is keyed by: two folders can be
        /// called `design`, and folding one must not fold the other.
        path: String,
        children: Vec<DocumentNode>,
    },
    Document {
        name: String,
        entry: DocumentIndexEntryResponse,
    },
}

impl DocumentNode {
    /// Folders before documents, each half alphabetical.
    ///
    /// Folders first because that is what a file tree does everywhere and a reader scans for structure before
    /// contents; alphabetical within each half because any other order is one nobody can predict.
    fn sort_key(&self) -> (u8, String) {
        match self {
            Self::Folder { name, .. } => (0, name.to_lowercase()),
            Self::Document { name, .. } => (1, name.to_lowercase()),
        }
    }
}

/// Turn a flat index into the tree.
///
/// The entries arrive sorted by path — the server sorts them — but nothing here relies on it: each path is
/// walked segment by segment into the structure and every level is sorted afterwards, so an index in any order
/// draws the same tree.
pub fn build_tree(entries: &[DocumentIndexEntryResponse]) -> Vec<DocumentNode> {
    let mut roots: Vec<DocumentNode> = Vec::new();

    for entry in entries {
        let mut segments: Vec<&str> = entry
            .path
            .split('/')
            .filter(|itm| !itm.is_empty())
            .collect();

        // The file name comes off the end; what is left is the folders it sits in. A path with nothing in it is
        // skipped rather than drawn as a nameless row — the server refuses to store one, so this only guards
        // against a shape nobody predicted.
        if segments.pop().is_none() {
            continue;
        }

        insert(&mut roots, &segments, "", entry);
    }

    sort_level(&mut roots);
    roots
}

/// Walk one document into the tree, one folder per call.
///
/// Recursive rather than a loop over a `&mut` cursor, which is what the borrow checker has to say about
/// descending into a vector you are also pushing to — and the recursion is bounded by the depth of a path.
fn insert(
    level: &mut Vec<DocumentNode>,
    folders: &[&str],
    walked: &str,
    entry: &DocumentIndexEntryResponse,
) {
    let Some((head, rest)) = folders.split_first() else {
        level.push(DocumentNode::Document {
            name: document_file_name(&entry.path).to_string(),
            entry: entry.clone(),
        });

        return;
    };

    let path = if walked.is_empty() {
        head.to_string()
    } else {
        format!("{walked}/{head}")
    };

    // Found or created, in that order: two documents under `docs/` must land in ONE `docs` node, or the tree
    // would draw the folder twice with half the files in each.
    let position = level.iter().position(|node| match node {
        DocumentNode::Folder { path: existing, .. } => existing == &path,
        DocumentNode::Document { .. } => false,
    });

    let position = match position {
        Some(position) => position,
        None => {
            level.push(DocumentNode::Folder {
                name: head.to_string(),
                path: path.clone(),
                children: Vec::new(),
            });

            level.len() - 1
        }
    };

    // `if let` rather than an unwrap: the entry was found or created as a folder one statement ago, and a
    // panic here would blank the whole screen over one path.
    if let DocumentNode::Folder { children, .. } = &mut level[position] {
        insert(children, rest, &path, entry);
    }
}

fn sort_level(level: &mut Vec<DocumentNode>) {
    level.sort_by_key(|node| node.sort_key());

    for node in level.iter_mut() {
        if let DocumentNode::Folder { children, .. } = node {
            sort_level(children);
        }
    }
}

/// The document the url is pointing at, or `""` when it points at none.
fn selected_id() -> String {
    match use_route::<crate::AppRoute>() {
        crate::AppRoute::Documents { selected } => selected,
        _ => String::new(),
    }
}

/// One level of the tree.
///
/// Split from [`DocumentTreeRow`] so the recursion goes through two components rather than one calling itself
/// — the same shape the file browser this screen is modelled on uses.
#[component]
pub fn DocumentNodes(
    nodes: Vec<DocumentNode>,
    depth: usize,
    cs: Signal<DocumentsState>,
) -> Element {
    rsx! {
        for node in nodes.iter() {
            DocumentTreeRow {
                key: "{node_key(node)}",
                node: node.clone(),
                depth,
                cs,
            }
        }
    }
}

/// A key that survives a rebuild: a folder by its path, a document by its id.
fn node_key(node: &DocumentNode) -> String {
    match node {
        DocumentNode::Folder { path, .. } => format!("folder:{path}"),
        DocumentNode::Document { entry, .. } => format!("doc:{}", entry.id),
    }
}

/// One row — a folder that opens and closes, or a document that can be selected.
#[component]
pub fn DocumentTreeRow(
    node: DocumentNode,
    depth: usize,
    cs: Signal<DocumentsState>,
) -> Element {
    let mut cs = cs;

    // Read off the url rather than out of the state: the url is what says which document is open, and reading
    // it here also means a row re-draws when the selection moves — including when it moves by the back button.
    //
    // Read into a variable first and not behind a short-circuit: it is a hook, and skipping it on a folder row
    // would make the hook order depend on what the row happens to be.
    let selected = selected_id();

    let indent = format!("padding-left: {}px", depth * INDENT + 6);

    match node {
        DocumentNode::Folder {
            name,
            path,
            children,
        } => {
            let expanded = cs.read().is_expanded(&path);
            let icon = folder_icon(expanded);
            let for_toggle = path.clone();

            rsx! {
                div {
                    class: "tree-row",
                    style: "{indent}",
                    onclick: move |_| cs.write().toggle(&for_toggle),

                    img { class: "tree-icon", src: "{icon}" }
                    span { class: "tree-name truncate", "{name}" }
                    span { class: "tree-size dim", "{children.len()}" }
                }

                // Only mounted while open, so collapsing a folder stops its children rendering at all.
                if expanded {
                    DocumentNodes {
                        nodes: children.clone(),
                        depth: depth + 1,
                        cs,
                    }
                }
            }
        }
        DocumentNode::Document { name, entry } => {
            let icon = file_icon(&name);
            let is_selected = selected == entry.id;

            let row_class = if is_selected {
                "tree-row selected"
            } else {
                "tree-row"
            };

            let id = entry.id.clone();
            let size = render_size(entry.size);

            rsx! {
                div {
                    class: "{row_class}",
                    style: "{indent}",
                    title: "{entry.path}",
                    // Selecting is navigating: the id goes into the url and the viewer follows from there.
                    // Nothing about the selection is written to the state — which is what makes a document
                    // linkable and the back button work.
                    onclick: move |_| {
                        navigator().push(crate::AppRoute::Documents { selected: id.clone() });
                    },

                    img { class: "tree-icon", src: "{icon}" }
                    span { class: "tree-name truncate", "{name}" }
                    span { class: "tree-size dim", "{size}" }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str) -> DocumentIndexEntryResponse {
        DocumentIndexEntryResponse {
            id: format!("id-{path}"),
            project_id: "p".to_string(),
            path: path.to_string(),
            content_type: "text/markdown".to_string(),
            is_binary: false,
            version: 1,
            size: 0,
            updated_unix_seconds: 0,
            updated_by: "AI".to_string(),
        }
    }

    fn names(nodes: &[DocumentNode]) -> Vec<String> {
        nodes
            .iter()
            .map(|node| match node {
                DocumentNode::Folder { name, children, .. } => {
                    format!("{name}/({})", names(children).join(","))
                }
                DocumentNode::Document { name, .. } => name.clone(),
            })
            .collect()
    }

    #[test]
    fn a_flat_list_stays_flat() {
        let tree = build_tree(&[entry("b.md"), entry("a.md")]);
        assert_eq!(names(&tree), vec!["a.md", "b.md"]);
    }

    /// The whole point: the folders in the tree came out of the paths, and nothing else said they exist.
    #[test]
    fn folders_come_out_of_the_paths() {
        let tree = build_tree(&[
            entry("docs/design/system.md"),
            entry("docs/notes.md"),
            entry("readme.md"),
        ]);

        assert_eq!(
            names(&tree),
            vec!["docs/(design/(system.md),notes.md)", "readme.md"]
        );
    }

    /// Two documents in one folder must land in ONE node, or the folder is drawn twice with half its files in
    /// each — which is what a naive append does.
    #[test]
    fn documents_of_one_folder_share_its_node() {
        let tree = build_tree(&[entry("docs/a.md"), entry("docs/b.md")]);

        assert_eq!(tree.len(), 1, "one folder, not two");
        assert_eq!(names(&tree), vec!["docs/(a.md,b.md)"]);
    }

    /// Order in, same tree out. The server sorts, but a tree that depended on that would break silently the
    /// day anything else fed it.
    #[test]
    fn the_order_it_arrives_in_does_not_matter() {
        let sorted = build_tree(&[entry("docs/a.md"), entry("docs/z/b.md"), entry("top.md")]);
        let shuffled = build_tree(&[entry("top.md"), entry("docs/z/b.md"), entry("docs/a.md")]);

        assert_eq!(names(&sorted), names(&shuffled));
    }

    /// Folders before documents at every level: a reader scans for structure first.
    #[test]
    fn folders_sort_before_documents() {
        let tree = build_tree(&[entry("zz.md"), entry("aa/one.md")]);
        assert_eq!(names(&tree), vec!["aa/(one.md)", "zz.md"]);
    }

    /// A folder is keyed by its path and a document by its id, so a repaint does not swap rows around.
    #[test]
    fn every_node_has_a_key_of_its_own() {
        let tree = build_tree(&[entry("docs/a.md")]);

        assert_eq!(node_key(&tree[0]), "folder:docs");

        if let DocumentNode::Folder { children, .. } = &tree[0] {
            assert_eq!(node_key(&children[0]), "doc:id-docs/a.md");
        } else {
            panic!("expected a folder");
        }
    }
}
