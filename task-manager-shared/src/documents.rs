use my_http_utils::macros::{MyHttpInput, MyHttpObjectStructure};
use serde::{Deserialize, Serialize};

// Never put `///` doc comments on fields of a struct deriving MyHttpInput or
// MyHttpObjectStructure: the macro's attribute parser panics with `Somehow we got Punct here: =`.

// One document's index entry: everything about it EXCEPT its content.
//
// The split is the whole reason this type exists. A document is the one thing in the product that is not
// held in memory — it is read out of Postgres on request — so the list a screen draws must not carry the
// texts: a project with two hundred documents would otherwise ship every one of them to draw a tree of
// names. Content arrives one document at a time, when somebody opens it.
//
// `path` is the document's name AND its position: folders are derived from it and are not stored anywhere,
// so `docs/design/system.md` puts this entry two levels down without any folder having to exist.
//
// `id` is a `SortableId` and never changes — not when the text is rewritten, not when the path moves. That
// is what makes the history complete: every version of this document is stitched together by this string.
// It is also what a task or a goal stores when it references a document, which is why moving a document
// never breaks a reference.
#[derive(Serialize, Deserialize, MyHttpObjectStructure, Clone, Debug, PartialEq)]
pub struct DocumentIndexEntryResponse {
    pub id: String,
    pub project_id: String,
    pub path: String,
    // Which version is current. Starts at 1 and moves on every write — a rewrite, a move, a delete, a
    // restore. Shown because it is the cheapest possible answer to "has anybody touched this".
    pub version: i64,
    // How many characters the content is. A size rather than the text: it says whether a document is a
    // note or a specification, which is what a reader wants from a list.
    pub size: i64,
    pub updated_unix_seconds: i64,
    // Who wrote the current version — an email or the literal `AI`, unvalidated for the same reason a
    // comment's author is: MCP has no session to derive one from.
    pub updated_by: String,
}

// One document in full, content included. What a reader gets when they open one.
#[derive(Serialize, Deserialize, MyHttpObjectStructure, Clone, Debug, PartialEq)]
pub struct DocumentResponse {
    pub id: String,
    pub project_id: String,
    pub path: String,
    // Markdown. Rendered by whoever draws it, never trusted as HTML — an agent wrote it.
    pub content: String,
    pub version: i64,
    pub created_unix_seconds: i64,
    pub updated_unix_seconds: i64,
    pub updated_by: String,
}

#[derive(Serialize, Deserialize, MyHttpObjectStructure, Clone, Debug, PartialEq)]
pub struct DocumentsResponse {
    pub documents: Vec<DocumentIndexEntryResponse>,
}

// One document, or the reason there is none.
//
// A miss is prose rather than an empty result, the same rule the whole surface follows: a reference to a
// document that has been deleted has to read as "it is in the trash", never as a blank pane.
#[derive(Serialize, Deserialize, MyHttpObjectStructure, Clone, Debug, PartialEq)]
pub struct FindDocumentResponse {
    pub document: Option<DocumentResponse>,
    // True when the id names a document that is in the trash. Told apart from "no such id" because they
    // are different facts and only one of them is recoverable — restoring is an MCP call away.
    pub in_trash: bool,
    pub not_found: String,
}

#[derive(MyHttpInput)]
pub struct GetDocumentsInputModel {
    #[http_body(name: "projectId", description: "Which project's documents to index")]
    pub project_id: String,
}

#[derive(MyHttpInput)]
pub struct GetDocumentInputModel {
    #[http_body(name: "projectId", description: "Which project the document belongs to")]
    pub project_id: String,
    #[http_body(name: "id", description: "Which document, by the id its index entry reports")]
    pub id: String,
}

/// The longest a document path may be. Generous enough for a deep folder tree, short enough that a path is
/// still something a person can read in a list.
pub const MAX_PATH_LEN: usize = 400;

/// The separator, and the only one. A document path is not a filesystem path: it is normalised to forward
/// slashes so the same document is named the same way whichever machine an agent is running on.
pub const PATH_SEPARATOR: char = '/';

/// Normalise a document path, or say why it is not one.
///
/// Here rather than on the server because both sides need the same answer: the server validates every write
/// with it, and the browser splits the result into folders — a client that disagreed about what a path is
/// would draw a tree the server does not have.
///
/// What it accepts, and why each rule is a rule:
///
/// * **A leading slash is dropped.** `/docs/a.md` and `docs/a.md` are one document. An agent handed a path
///   by a person will write either, and treating them as two documents at the same place is the single
///   easiest way to end up with a silent duplicate.
/// * **Backslashes become slashes**, for the same reason: an agent on Windows types the separator its shell
///   uses, and the document it means is the same document.
/// * **Empty segments collapse.** `docs//a.md` is `docs/a.md` — a doubled separator is a typo, not a folder
///   with no name.
/// * **`.` and `..` are refused.** This is not a walk over a filesystem; there is no current directory to be
///   relative to, and accepting `..` would mean two spellings of one path that only a resolver could tell
///   apart.
/// * **A trailing slash is refused.** A path names a document, and `docs/` names a folder — which does not
///   exist as a thing at all, since folders are derived from paths.
///
/// **Case is preserved and significant**: `README.md` and `readme.md` are two documents. A path is the name
/// a person gave the document, and folding case would silently rewrite it.
pub fn normalise_document_path(src: &str) -> Result<String, String> {
    let src = src.trim().replace('\\', "/");

    if src.is_empty() {
        return Err("a document needs a path, e.g. `notes.md` or `docs/design/system.md`".to_string());
    }

    if src.len() > MAX_PATH_LEN {
        return Err(format!(
            "that path is {} characters — the limit is {MAX_PATH_LEN}",
            src.len()
        ));
    }

    if src.ends_with(PATH_SEPARATOR) {
        return Err(format!(
            "'{src}' ends in a slash, so it names a folder rather than a document — folders are not stored, they come from the paths of the documents in them"
        ));
    }

    let mut segments: Vec<&str> = Vec::new();

    for segment in src.split(PATH_SEPARATOR) {
        let segment = segment.trim();

        // A doubled separator, or space around one. Dropped rather than refused: it is a typo with exactly
        // one sensible reading.
        if segment.is_empty() {
            continue;
        }

        if segment == "." || segment == ".." {
            return Err(format!(
                "'{src}' contains '{segment}' — a document path is not relative to anything, so write it out in full"
            ));
        }

        segments.push(segment);
    }

    if segments.is_empty() {
        return Err(format!("'{src}' has no name in it"));
    }

    Ok(segments.join("/"))
}

/// The folders a path sits in, outermost first. Empty for a document at the top level.
///
/// Derived, never stored — which is the whole model: a folder exists exactly as long as some document's path
/// mentions it, and an empty folder is not a thing that can exist.
pub fn document_folders(path: &str) -> Vec<&str> {
    let mut segments: Vec<&str> = path.split(PATH_SEPARATOR).collect();
    segments.pop();
    segments
}

/// The last segment of a path — what a reader sees as the document's name in its folder.
pub fn document_file_name(path: &str) -> &str {
    path.rsplit(PATH_SEPARATOR).next().unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_path_survives_untouched() {
        assert_eq!(
            normalise_document_path("docs/design/system.md").unwrap(),
            "docs/design/system.md"
        );
        assert_eq!(normalise_document_path("notes.md").unwrap(), "notes.md");
    }

    /// The case this exists for: a person says `/path/document.md`, an agent passes it verbatim, and it has
    /// to be the same document as `path/document.md` rather than a second one at the same place.
    #[test]
    fn the_spellings_of_one_path_all_normalise_to_it() {
        for src in [
            "/docs/a.md",
            "docs/a.md",
            "  docs/a.md  ",
            "docs//a.md",
            "docs / a.md",
            "\\docs\\a.md",
        ] {
            assert_eq!(
                normalise_document_path(src).unwrap(),
                "docs/a.md",
                "{src:?} should normalise to docs/a.md"
            );
        }
    }

    /// Case is part of the name. Folding it would quietly rewrite what somebody called their document.
    #[test]
    fn case_is_significant() {
        assert_eq!(normalise_document_path("README.md").unwrap(), "README.md");
        assert_ne!(
            normalise_document_path("README.md").unwrap(),
            normalise_document_path("readme.md").unwrap()
        );
    }

    #[test]
    fn what_is_not_a_path_is_refused() {
        for src in [
            "",
            "   ",
            "/",
            "docs/",
            "docs/../a.md",
            "./a.md",
            "..",
        ] {
            assert!(
                normalise_document_path(src).is_err(),
                "{src:?} should not be accepted as a path"
            );
        }
    }

    #[test]
    fn an_over_long_path_is_refused() {
        let long = format!("{}.md", "a".repeat(MAX_PATH_LEN));
        assert!(normalise_document_path(&long).is_err());
    }

    #[test]
    fn folders_come_from_the_path_and_nothing_else() {
        assert_eq!(document_folders("docs/design/system.md"), vec!["docs", "design"]);
        assert_eq!(document_folders("notes.md"), Vec::<&str>::new());
    }

    #[test]
    fn the_name_is_the_last_segment() {
        assert_eq!(document_file_name("docs/design/system.md"), "system.md");
        assert_eq!(document_file_name("notes.md"), "notes.md");
    }
}
