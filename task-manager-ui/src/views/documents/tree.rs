use task_manager_shared::documents::{DocumentIndexEntryResponse, document_file_name};

/// One node of the tree the screen draws: a folder with children, or a document.
///
/// `Clone` and `PartialEq` because a level of it is handed to a child component as a prop, which Dioxus
/// requires both of. Compared by value rather than by pointer: an index that came back identical draws an
/// identical tree, and a level that has not changed then does not repaint.
///
/// **Built on every render out of the index, and stored nowhere.** That is not a shortcut — it is the model:
/// a folder does not exist on the server either, it is read off the paths of the documents in it. So an empty
/// folder cannot appear here, and the last document leaving one makes it vanish, with nothing to clean up.
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
/// The entries are expected sorted by path — the server sorts them — but nothing here relies on it: each path
/// is walked segment by segment into the structure, and the result is sorted at every level afterwards. So an
/// index that arrived in any order draws the same tree.
pub fn build_tree(entries: &[DocumentIndexEntryResponse]) -> Vec<DocumentNode> {
    let mut roots: Vec<DocumentNode> = Vec::new();

    for entry in entries {
        let mut segments: Vec<&str> = entry
            .path
            .split('/')
            .filter(|itm| !itm.is_empty())
            .collect();

        // The file name comes off the end; what is left is the folders it sits in. A path with nothing in it
        // is skipped rather than turned into a nameless row — the server refuses to store one, so this only
        // guards against a shape nobody predicted.
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
            // From the whole path rather than from the last segment, so it agrees with what the rest of the
            // product calls the document's name.
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

/// Every folder path in a tree, so a screen can open all of them at once.
///
/// Used for the initial state: a tree that arrives entirely folded shows a reader one row and hides the
/// documents they came for. Folding is then something they do, not something they have to undo.
pub fn all_folder_paths(nodes: &[DocumentNode]) -> Vec<String> {
    let mut paths = Vec::new();
    collect_folders(nodes, &mut paths);
    paths
}

fn collect_folders(nodes: &[DocumentNode], into: &mut Vec<String>) {
    for node in nodes {
        if let DocumentNode::Folder { path, children, .. } = node {
            into.push(path.clone());
            collect_folders(children, into);
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

    #[test]
    fn every_folder_can_be_opened_at_once() {
        let tree = build_tree(&[entry("docs/design/system.md"), entry("top.md")]);

        let mut folders = all_folder_paths(&tree);
        folders.sort();

        assert_eq!(folders, vec!["docs", "docs/design"]);
    }
}
