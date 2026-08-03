use rust_extensions::date_time::DateTimeAsMicroseconds;
use task_manager_shared::github::{GITHUB_ROOT, github_mirror_path, parse_github_mirror_path};

use crate::app::AppContext;
use crate::board::ProjectModel;
use crate::documents::DocumentIndexEntry;
use crate::postgres::DocumentDto;

use super::resolve_project_by_prefix;

/// What every mirrored file's id starts with.
///
/// **A mirrored file has no real id and this is honest about that.** A document's id is a `SortableId`
/// minted once and never changed, which is what makes its history complete and what a task stores when it
/// references one. A file in somebody else's repository has none of that: it is derived from where the
/// file currently is, so it changes when the file moves and disappears when the file does. The prefix is
/// what stops it being mistaken for the other kind — nothing that starts with this is ever looked for in
/// Postgres.
pub const MIRROR_ID_PREFIX: &str = "github:";

/// Who a mirrored file is attributed to. Not a person and deliberately not an email: nobody on this
/// board wrote it.
pub const MIRROR_AUTHOR: &str = "github";

/// The id a mirrored file is named by.
///
/// It carries the project because an id is looked up on its own — `documents_get` takes one without a
/// project beside it — and a path is only unique within a board.
pub fn mirror_document_id(project_prefix: &str, mirror_path: &str) -> String {
    format!("{MIRROR_ID_PREFIX}{project_prefix}:{mirror_path}")
}

/// Split a mirrored file's id back into the project and the path it names.
pub fn parse_mirror_document_id(id: &str) -> Option<(&str, &str)> {
    let rest = id.strip_prefix(MIRROR_ID_PREFIX)?;
    let (project_prefix, mirror_path) = rest.split_once(':')?;

    if project_prefix.is_empty() || mirror_path.is_empty() {
        return None;
    }

    Some((project_prefix, mirror_path))
}

/// Every mirrored file of one project, as index entries indistinguishable in shape from a document's.
///
/// **Shape, not substance.** They sit in the same list because that is what makes a connected repository
/// readable by the tools that already exist — `documents_list` answers with them, `documents_get` reads
/// them — and they are told apart by their path and their id whenever it matters, which is every write.
///
/// `version` is 0 on all of them, and that number is a statement: this product has no versions of this
/// file. Its history is in the repository it came from.
pub fn mirror_index_entries(app: &AppContext, project: &ProjectModel) -> Vec<DocumentIndexEntry> {
    let mut entries = Vec::new();

    for connection in project.github_connections.iter() {
        let mirror = app.github.get_or_pending(&project.id, &connection.name);

        let updated = mirror.pulled.unwrap_or_else(|| DateTimeAsMicroseconds::new(0));

        for file in mirror.entries.iter() {
            let path = github_mirror_path(&connection.name, &file.path);

            entries.push(DocumentIndexEntry {
                id: mirror_document_id(&project.prefix, &path),
                project_id: project.id.clone(),
                path,
                content_type: file.content_type.clone(),
                is_binary: file.is_binary,
                size: file.size,
                version: 0,
                created: updated,
                updated,
                updated_by: MIRROR_AUTHOR.to_string(),
            });
        }
    }

    entries
}

/// Read one mirrored file off disk, as the row shape every reader of a document already handles.
///
/// The payload is decided the same way the archive upload decides it — the extension says what the file
/// is, and the bytes get to disagree. A `.md` that is not UTF-8 is served as bytes rather than as
/// mojibake.
pub async fn read_mirror_document(
    app: &AppContext,
    project_prefix: &str,
    mirror_path: &str,
) -> Result<DocumentDto, String> {
    let (project_id, project_prefix) = {
        let board = app.board.read();
        let project = resolve_project_by_prefix(&board, project_prefix)?;
        (project.id.clone(), project.prefix.clone())
    };

    let Some((connection, relative)) = parse_github_mirror_path(mirror_path) else {
        return Err(format!(
            "'{mirror_path}' is not a path in a connected repository — those look like `{GITHUB_ROOT}/<connection>/<file>`"
        ));
    };

    let mirror = app.github.get_or_pending(&project_id, connection);

    let Some(entry) = mirror.entry(relative) else {
        return Err(format!(
            "'{mirror_path}' is not in the mirror. It may have been removed upstream, or the connection may not have pulled yet — its state is '{}'",
            mirror.state
        ));
    };

    let Some(on_disk) = mirror.file_on_disk(relative) else {
        return Err(format!("'{mirror_path}' is not in the mirror"));
    };

    let bytes = tokio::fs::read(&on_disk).await.map_err(|err| {
        format!("'{mirror_path}' is in the index but did not read from disk: {err} — the next pull will rebuild it")
    })?;

    // Text only when the extension says text AND the bytes agree. Same rule, and the same reason, as the
    // archive unpack: storing a guess as text is what produces a document nobody can read back.
    let (content, binary_content) = if entry.is_binary {
        (None, Some(bytes))
    } else {
        match String::from_utf8(bytes) {
            Ok(text) if !text.contains('\0') => (Some(text), None),
            Ok(text) => (None, Some(text.into_bytes())),
            Err(err) => (None, Some(err.into_bytes())),
        }
    };

    let updated = mirror.pulled.unwrap_or_else(|| DateTimeAsMicroseconds::new(0));

    Ok(DocumentDto {
        id: mirror_document_id(&project_prefix, mirror_path),
        project_id,
        doc_path: mirror_path.to_string(),
        content_type: Some(entry.content_type.clone()),
        content,
        binary_content,
        content_size: Some(entry.size),
        // Not a version. See `mirror_index_entries`.
        version: 0,
        created: updated,
        updated,
        updated_by: MIRROR_AUTHOR.to_string(),
    })
}

/// Refuse a write that names a path inside the reserved root.
///
/// **One guard, called from every write, rather than a rule each of them remembers.** The paths under
/// `github/` are a window onto a repository this product does not own: there is nothing there to write a
/// version of, and a document created there would be shadowed by the next pull or would shadow a real
/// file. The message says what to do instead, because "refused" without an alternative is the kind of
/// error that gets worked around.
pub fn refuse_reserved_path(path: &str) -> Result<(), String> {
    if !task_manager_shared::github::is_github_path(path) {
        return Ok(());
    }

    Err(format!(
        "'{path}' is inside '{GITHUB_ROOT}/', which is where connected repositories are shown — those files belong to the repository and are read-only here. To bring one into this project, sync it into a folder of your own"
    ))
}

/// Refuse a write that names a mirrored file by its id.
///
/// The other door into the same rule as [`refuse_reserved_path`]. Without it the message would come from
/// the lookup instead — "no document github:TM:…" — which reads as a bug in this product rather than as
/// the deliberate answer it is.
pub fn refuse_reserved_id(id: &str) -> Result<(), String> {
    let Some((_, mirror_path)) = parse_mirror_document_id(id) else {
        return Ok(());
    };

    Err(format!(
        "'{mirror_path}' is a file in a connected repository, which this product only reads. To change it, change it in the repository; to bring it in, sync it into a folder of your own"
    ))
}

/// Refuse a question about a mirrored file's past.
///
/// Softer than a refused write and for a different reason: nothing is being prevented, there simply is
/// no history on this side. Saying where the history IS is the whole content of the message.
pub fn refuse_reserved_history(id: &str) -> Result<(), String> {
    let Some((_, mirror_path)) = parse_mirror_document_id(id) else {
        return Ok(());
    };

    Err(format!(
        "'{mirror_path}' is a file in a connected repository, and this product keeps no versions of it — the mirror only ever holds what the repository holds now. Its history is in the repository"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mirrored_id_says_which_project_and_which_path() {
        let id = mirror_document_id("TM", "github/specs/design/a.md");

        assert_eq!(id, "github:TM:github/specs/design/a.md");
        assert_eq!(
            parse_mirror_document_id(&id),
            Some(("TM", "github/specs/design/a.md"))
        );
    }

    /// A real document's id must never be mistaken for a mirrored one — the two are looked up in
    /// completely different places.
    #[test]
    fn a_real_id_is_not_a_mirrored_one() {
        assert_eq!(parse_mirror_document_id("01HXYZ0123456789ABCDEFG"), None);
        // The prefix alone names nothing.
        assert_eq!(parse_mirror_document_id("github:"), None);
        assert_eq!(parse_mirror_document_id("github:TM:"), None);
    }

    #[test]
    fn a_write_into_the_reserved_root_is_refused_with_somewhere_to_go() {
        let err = refuse_reserved_path("github/specs/a.md").unwrap_err();
        assert!(err.contains("sync"), "{err}");

        assert!(refuse_reserved_path("github").is_err());

        // A folder of the project's own that merely starts the same way is untouched.
        assert!(refuse_reserved_path("github-notes/a.md").is_ok());
        assert!(refuse_reserved_path("docs/a.md").is_ok());
    }
}
