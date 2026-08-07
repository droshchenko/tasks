use std::path::PathBuf;

use rust_extensions::date_time::DateTimeAsMicroseconds;
use task_manager_shared::github::{GITHUB_ROOT, github_mirror_path, parse_github_mirror_path};
// Minted here on every listing and read in the browser to open the file one names, so the spelling lives
// in `shared` beside the reference vocabulary that carries it. Re-exported because this module is where
// the rest of the service has always reached for it.
pub use task_manager_shared::github::{mirror_document_id, parse_mirror_document_id};

use crate::app::AppContext;
use crate::board::{GithubConnectionModel, ProjectModel};
use crate::documents::DocumentIndexEntry;
use crate::github::workdir;
use crate::postgres::DocumentDto;

use super::resolve_project_by_prefix;

/// Who a file of a connected repository is attributed to in a listing.
///
/// Not a person, and deliberately not an email: what a listing can honestly say is "this came from the
/// repository". Who wrote a particular line is a question for `git log`, which knows, and this side does
/// not — nothing here records an author, because nothing here keeps versions.
pub const MIRROR_AUTHOR: &str = "github";

/// Every file of every connection of one project, as index entries indistinguishable in shape from a
/// document's.
///
/// **Shape, not substance.** They sit in the same list because that is what makes a connected repository
/// readable by the tools that already exist — `documents_list` answers with them, `documents_get` reads
/// them, `documents_edit` writes them — and they are told apart by their path and their id wherever it
/// matters.
///
/// `version` is 0 on all of them, and that number is a statement: this product keeps no versions of these
/// files. Their history is the repository's, and `git log` is how it is read.
pub fn mirror_index_entries(app: &AppContext, project: &ProjectModel) -> Vec<DocumentIndexEntry> {
    let mut entries = Vec::new();

    for connection in project.github_connections.iter() {
        let mirror = app.github.get_or_pending(&project.id, &connection.name);

        let updated = mirror.listed.unwrap_or_else(|| DateTimeAsMicroseconds::new(0));

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

/// One file of a connected repository, resolved all the way down to where it is on disk.
pub struct MirrorTarget {
    pub project_id: String,
    pub project_prefix: String,
    pub connection: GithubConnectionModel,
    /// The clone's own folder — what a git command runs in.
    pub clone_dir: PathBuf,
    /// The file, relative to the connection's root.
    pub relative: String,
    /// The file as this product names it: `github/<connection>/<file>`.
    pub mirror_path: String,
    /// Where the file is on disk.
    pub full_path: PathBuf,
}

/// Turn a `github/<connection>/<file>` path into everything needed to touch that file.
///
/// The one place the four questions are answered together — which project, which connection, where its
/// clone is, and which file inside it — so that a read, a write and a delete cannot disagree about any of
/// them.
pub fn resolve_mirror_target(
    app: &AppContext,
    project_prefix: &str,
    mirror_path: &str,
) -> Result<MirrorTarget, String> {
    let Some((connection_name, relative)) = parse_github_mirror_path(mirror_path) else {
        return Err(format!(
            "'{mirror_path}' is not a path in a connected repository — those look like `{GITHUB_ROOT}/<connection>/<file>`"
        ));
    };

    let (project_id, project_prefix, connection) = {
        let board = app.board.read();
        let project = resolve_project_by_prefix(&board, project_prefix)?;

        let connection = project
            .github_connection(connection_name)
            .ok_or_else(|| {
                format!("this project has no connected repository called '{connection_name}'")
            })?
            .clone();

        (project.id.clone(), project.prefix.clone(), connection)
    };

    let clone_dir = workdir::connection_dir(&app.git_repos_path, &project_id, connection_name);

    if !workdir::is_cloned(&clone_dir) {
        let mirror = app.github.get_or_pending(&project_id, connection_name);

        return Err(format!(
            "'{connection_name}' has not been cloned yet — its state is '{}'{}",
            mirror.state,
            match mirror.error.is_empty() {
                true => String::new(),
                false => format!(": {}", mirror.error),
            }
        ));
    }

    let full_path = workdir::file_path(&clone_dir, &connection.repo_path, relative)?;

    Ok(MirrorTarget {
        project_id,
        project_prefix,
        connection,
        clone_dir,
        relative: relative.to_string(),
        mirror_path: mirror_path.to_string(),
        full_path,
    })
}

/// Read one file of a connected repository, off the disk it is on.
///
/// **No network, no rate limit and no key.** This used to be a request to GitHub per read, which is what
/// made a public repository browsable and a sync of two hundred files not: anonymous reads share sixty an
/// hour. A clone spends that budget once, on the clone, and every read afterwards is a file read.
///
/// The payload is decided the same way an uploaded archive decides it — the extension says what the file
/// is, and the bytes get to disagree. A `.md` that is not UTF-8 is served as bytes rather than as
/// mojibake.
pub async fn read_mirror_document(
    app: &AppContext,
    project_prefix: &str,
    mirror_path: &str,
) -> Result<DocumentDto, String> {
    let target = resolve_mirror_target(app, project_prefix, mirror_path)?;

    let bytes = workdir::read_file(&target.full_path)
        .map_err(|err| format!("'{mirror_path}' did not read: {err}"))?;

    Ok(document_of(&target, bytes))
}

/// Write one file of a connected repository.
///
/// **What this produces is a change in the working tree, and nothing else.** Nothing is staged, nothing is
/// committed and nothing is pushed — `git status` will now show the file as modified, and what happens to
/// it next is a git command somebody runs deliberately. That separation is the point: an edit that
/// committed itself would put a commit on a branch for every keystroke of an agent's, and an edit that
/// pushed itself would do it to a repository somebody else is working in.
///
/// `who` is taken for the same shape of call as every other write and is deliberately not recorded here:
/// there is no version row to record it on. The author of the work is recorded where it belongs, on the
/// commit — `git commit --author "Name <email>"`.
pub async fn write_mirror_document(
    app: &AppContext,
    project_prefix: &str,
    mirror_path: &str,
    bytes: &[u8],
) -> Result<DocumentDto, String> {
    let target = resolve_mirror_target(app, project_prefix, mirror_path)?;

    workdir::write_file(&target.full_path, bytes)
        .map_err(|err| format!("'{mirror_path}' did not write: {err}"))?;

    // Immediately rather than at the next tick of the timer: a file created here that did not appear in
    // `documents_list` for ten minutes would read as a write that silently did nothing.
    crate::github::relist_connection(app, &target.project_id, &target.connection).await;

    Ok(document_of(&target, bytes.to_vec()))
}

/// Delete one file of a connected repository from the working tree.
///
/// **There is no trash here, and it needs none.** A document of the project's own goes to a trash table
/// because Postgres is the only place it exists. A tracked file that is deleted is still in every commit
/// that ever held it: `git checkout -- <path>` brings it straight back, and `git log --diff-filter=D`
/// finds it if nobody remembers the name. Deleting a file that was never committed is the one case that
/// really loses it, and that is true of any working copy.
///
/// Returns the path it had, which is what a caller wants echoed back.
pub async fn delete_mirror_document(
    app: &AppContext,
    project_prefix: &str,
    mirror_path: &str,
) -> Result<String, String> {
    let target = resolve_mirror_target(app, project_prefix, mirror_path)?;

    let root = workdir::connection_root(&target.clone_dir, &target.connection.repo_path);

    workdir::delete_file(&target.full_path, &root)
        .map_err(|err| format!("'{mirror_path}' did not delete: {err}"))?;

    crate::github::relist_connection(app, &target.project_id, &target.connection).await;

    Ok(target.mirror_path)
}

/// Move one file of a connected repository to another path in the SAME connection.
///
/// **Across the boundary is not a move, and is refused as one.** Moving a file out of a connection into
/// the project's own documents is a sync — it produces a document with an id, a version and a history,
/// and the file stays in the repository. Moving one in is an upload. Neither is what "move" means, and
/// doing either silently under this name would be a rename that quietly changed what a thing IS.
pub async fn move_mirror_document(
    app: &AppContext,
    project_prefix: &str,
    from_path: &str,
    to_path: &str,
) -> Result<DocumentDto, String> {
    let from = resolve_mirror_target(app, project_prefix, from_path)?;

    if !task_manager_shared::github::is_github_path(to_path) {
        return Err(format!(
            "'{from_path}' is in a connected repository and '{to_path}' is not — moving between the two is not a move. To bring a copy into this project's own documents, sync it; the file stays in the repository either way"
        ));
    }

    let to = resolve_mirror_target(app, project_prefix, to_path)?;

    if to.connection.name != from.connection.name {
        return Err(format!(
            "'{from_path}' and '{to_path}' are in two different connected repositories — a move happens inside one working copy. Read it from the one and write it into the other"
        ));
    }

    if to.full_path == from.full_path {
        return Err(format!(
            "'{to_path}' is already where that file is — nothing to move"
        ));
    }

    if to.full_path.exists() {
        return Err(format!(
            "'{to_path}' is taken — pick another path, or write over that one if replacing it is what you meant"
        ));
    }

    let bytes = workdir::read_file(&from.full_path)
        .map_err(|err| format!("'{from_path}' did not read: {err}"))?;

    // Written before the old one is removed. The other order loses the file if the write fails, and this
    // order at worst leaves both — which `git status` shows and one delete undoes.
    workdir::write_file(&to.full_path, &bytes)
        .map_err(|err| format!("'{to_path}' did not write: {err}"))?;

    let root = workdir::connection_root(&from.clone_dir, &from.connection.repo_path);

    workdir::delete_file(&from.full_path, &root)
        .map_err(|err| format!("'{from_path}' did not delete: {err}"))?;

    crate::github::relist_connection(app, &to.project_id, &to.connection).await;

    Ok(document_of(&to, bytes))
}

/// Splice pieces of one file of a connected repository, in place.
///
/// **The same splice `documents_edit` performs on a document, written somewhere else.** `apply_edits` is
/// shared verbatim, so everything that makes the tool worth reaching for holds here too: only what
/// changes is sent, an `old_string` that appears more than once fails the whole batch rather than
/// changing seven places, and nothing is written unless every edit applies.
///
/// **What it does NOT have is `expected_version`, because there are no versions here.** The optimistic
/// lock exists to catch somebody having uploaded in between; the equivalent question in a repository is
/// answered by git, and answered better — `git diff` says exactly what is uncommitted and `git log` says
/// what arrived. A non-zero `expected_version` is refused rather than ignored, because silently accepting
/// a lock that checks nothing is worse than not offering one.
pub async fn edit_mirror_document(
    app: &AppContext,
    project_prefix: &str,
    mirror_path: &str,
    edits: &[super::DocumentEdit],
    expected_version: Option<i64>,
) -> Result<(DocumentDto, Vec<i32>), String> {
    if let Some(expected) = expected_version.filter(|itm| *itm != 0) {
        return Err(format!(
            "'{mirror_path}' is a file in a connected repository and has no version here — every read of one reports version 0, so `expected_version` of {expected} can never match. Drop it: what it guards against is answered by git, with `git diff` before you write and `git status` after"
        ));
    }

    let target = resolve_mirror_target(app, project_prefix, mirror_path)?;

    let bytes = workdir::read_file(&target.full_path)
        .map_err(|err| format!("'{mirror_path}' did not read: {err}"))?;

    let text = String::from_utf8(bytes).map_err(|_| {
        format!(
            "'{mirror_path}' is not text, so there is nothing in it to match against — replace it whole with documents_upload"
        )
    })?;

    let (edited, replacements) = super::apply_edits(&text, edits)?;

    check_size(edited.len())?;

    workdir::write_file(&target.full_path, edited.as_bytes())
        .map_err(|err| format!("'{mirror_path}' did not write: {err}"))?;

    crate::github::relist_connection(app, &target.project_id, &target.connection).await;

    Ok((document_of(&target, edited.into_bytes()), replacements))
}

/// Delete every file under one folder of a connected repository.
///
/// The same operation [`delete_mirror_document`] performs, repeated — which is also what deleting a
/// folder means anywhere else in this product, since folders are read off paths and never exist on their
/// own. Naming a connection with no folder under it empties that whole working copy, which is a real
/// thing to want and a `git checkout .` away from being undone.
///
/// **The one refusal is the reserved root itself.** `github` is not a folder in any repository — it is
/// where all of them are shown — and a call that emptied every connection of a project at once would be
/// the most destructive thing on this surface, reachable by typing one word.
pub async fn delete_mirror_folder(
    app: &AppContext,
    project_prefix: &str,
    folder: &str,
) -> Result<Vec<super::DeletedDocument>, String> {
    let rest = folder.trim().trim_start_matches('/');

    let Some(rest) = rest.strip_prefix(GITHUB_ROOT).and_then(|itm| itm.strip_prefix('/')) else {
        return Err(format!(
            "'{GITHUB_ROOT}' is not a folder in a repository — it is where every connected repository is shown. Name one of them, `{GITHUB_ROOT}/<connection>`, or a folder inside it"
        ));
    };

    let (connection_name, sub_folder) = match rest.split_once('/') {
        Some((connection_name, sub_folder)) => (connection_name, sub_folder),
        None => (rest, ""),
    };

    let (project_id, connection) = {
        let board = app.board.read();
        let project = resolve_project_by_prefix(&board, project_prefix)?;

        let connection = project
            .github_connection(connection_name)
            .ok_or_else(|| {
                format!("this project has no connected repository called '{connection_name}'")
            })?
            .clone();

        (project.id.clone(), connection)
    };

    // Off the listing rather than off the disk, so what this deletes is exactly what the caller was
    // looking at in `documents_list` — including the filters, which is the point: a file too big to be
    // listed is a file this call has no business removing.
    let mirror = app.github.get_or_pending(&project_id, connection_name);

    let prefix = match sub_folder.is_empty() {
        true => String::new(),
        false => format!("{sub_folder}/"),
    };

    let targets: Vec<String> = mirror
        .entries
        .iter()
        .filter(|itm| itm.path.starts_with(&prefix))
        .map(|itm| github_mirror_path(connection_name, &itm.path))
        .collect();

    if targets.is_empty() {
        return Err(format!(
            "nothing is under '{folder}' — no files to delete. Folders are read off the paths of the files in them, so one with nothing in it does not exist; check documents_list for the spelling, which is case-sensitive"
        ));
    }

    let total = targets.len();
    let mut deleted: Vec<super::DeletedDocument> = Vec::with_capacity(total);

    for mirror_path in targets {
        let target = resolve_mirror_target(app, project_prefix, &mirror_path)?;
        let root = workdir::connection_root(&target.clone_dir, &target.connection.repo_path);

        match workdir::delete_file(&target.full_path, &root) {
            Ok(()) => deleted.push(super::DeletedDocument {
                id: mirror_document_id(&target.project_prefix, &mirror_path),
                path: mirror_path,
            }),
            // Said with the count rather than as a bare failure: the ones already gone are a fact the
            // caller has to know about, and `git status` is where the whole picture is.
            Err(err) => {
                return Err(format!(
                    "{} of {total} under '{folder}' were deleted, then '{mirror_path}' stopped it: {err}",
                    deleted.len()
                ));
            }
        }
    }

    // Once, after all of them, rather than per file: this is one listing however many files went.
    crate::github::relist_connection(app, &project_id, &connection).await;

    Ok(deleted)
}

/// Refuse moving a document of the project's own INTO a connected repository.
///
/// Not a restriction on writing there — `documents_upload` names a `github/` path and writes the file.
/// What is refused is doing it under the name "move", because a move keeps a document's id, its version
/// and its history, and none of those can follow it into somebody's repository. What would actually
/// happen is a new file appearing in a working copy while a document quietly vanished from the board.
pub fn refuse_move_into_mirror(path: &str) -> Result<(), String> {
    if !task_manager_shared::github::is_github_path(path) {
        return Ok(());
    }

    Err(format!(
        "'{path}' is inside a connected repository, and a document of this project cannot be MOVED there — its id, its versions and every reference to it would not survive the trip. Write the file into the working copy with documents_upload naming that path, commit it with github_git, and delete the document here if it should only live in the repository"
    ))
}

/// Refuse a payload that is larger than a listing would ever show.
///
/// The same ceiling the working-copy listing filters on, applied on the way in: a file written past it
/// would disappear from `documents_list` the moment it was saved, which reads as a write that did not
/// happen.
fn check_size(size: usize) -> Result<(), String> {
    if size > super::MAX_BINARY_LEN {
        return Err(format!(
            "that would be {size} bytes — the limit for one file is {}",
            super::MAX_BINARY_LEN
        ));
    }

    Ok(())
}

/// One file of a working copy, in the shape every read of a document answers in.
///
/// Text only when the extension says text AND the bytes agree. Same rule, and the same reason, as the
/// archive unpack: storing a guess as text is what produces a document nobody can read back.
fn document_of(target: &MirrorTarget, bytes: Vec<u8>) -> DocumentDto {
    let is_binary = match task_manager_shared::documents::content_type_for_path(&target.relative) {
        Some(known) => !crate::documents::is_text_content_type(known),
        None => false,
    };

    let (content, binary_content) = if is_binary {
        (None, Some(bytes))
    } else {
        match String::from_utf8(bytes) {
            Ok(text) if !text.contains('\0') => (Some(text), None),
            Ok(text) => (None, Some(text.into_bytes())),
            Err(err) => (None, Some(err.into_bytes())),
        }
    };

    let content_size = content
        .as_ref()
        .map(|itm| itm.len() as i64)
        .or_else(|| binary_content.as_ref().map(|itm| itm.len() as i64));

    let now = DateTimeAsMicroseconds::now();

    DocumentDto {
        id: mirror_document_id(&target.project_prefix, &target.mirror_path),
        project_id: target.project_id.clone(),
        doc_path: target.mirror_path.clone(),
        content_type: Some(super::content_type_of(None, &target.relative)),
        content,
        binary_content,
        content_size,
        // Not a version. See `mirror_index_entries`.
        version: 0,
        created: now,
        updated: now,
        updated_by: MIRROR_AUTHOR.to_string(),
    }
}

/// Refuse a question about a mirrored file's past.
///
/// **Nothing is being prevented; the question is being sent where it can be answered.** This product
/// keeps no versions of a file it did not write — but git keeps all of them, and now that the repository
/// is cloned they are one command away rather than on another machine. Saying which command is the whole
/// content of the message.
pub fn refuse_reserved_history(id: &str) -> Result<(), String> {
    let Some((_, mirror_path)) = parse_mirror_document_id(id) else {
        return Ok(());
    };

    let Some((connection, relative)) = parse_github_mirror_path(mirror_path) else {
        return Ok(());
    };

    Err(format!(
        "'{mirror_path}' is a file in a connected repository, and this product keeps no versions of it — git does. Ask git instead, with github_git on '{connection}': `git log --follow -- {relative}` for what happened to it, `git diff -- {relative}` for what is uncommitted, `git show <commit> -- {relative}` for how it read then"
    ))
}

/// Refuse restoring a file that was never in this product's trash.
///
/// The same shape of answer as [`refuse_reserved_history`] and for the same reason: the undo exists, it
/// simply belongs to git.
pub fn refuse_reserved_restore(path_or_id: &str) -> Result<(), String> {
    let mirror_path = match parse_mirror_document_id(path_or_id) {
        Some((_, mirror_path)) => mirror_path,
        None if task_manager_shared::github::is_github_path(path_or_id) => path_or_id,
        None => return Ok(()),
    };

    let hint = match parse_github_mirror_path(mirror_path) {
        Some((connection, relative)) => format!(
            " Ask git instead, with github_git on '{connection}': `git checkout -- {relative}` puts back a file that was deleted or changed since the last commit, and `git restore --source <commit> -- {relative}` takes it from further back"
        ),
        None => String::new(),
    };

    Err(format!(
        "'{mirror_path}' is a file in a connected repository, and deleting one never put it in this product's trash — there is nothing here to restore it from.{hint}"
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

    /// The refusals that are left are the two that send somebody to git rather than stopping them — so
    /// the command to run has to actually be in the message.
    #[test]
    fn a_question_about_the_past_is_answered_with_the_git_command_that_answers_it() {
        let err = refuse_reserved_history("github:TM:github/specs/design/a.md").unwrap_err();

        assert!(err.contains("git log"), "{err}");
        assert!(err.contains("design/a.md"), "{err}");
        // Named, because a git command has to be run somewhere and there may be several connections.
        assert!(err.contains("specs"), "{err}");

        let err = refuse_reserved_restore("github/specs/design/a.md").unwrap_err();
        assert!(err.contains("git checkout"), "{err}");

        // A document of the project's own goes down the ordinary path, untouched by either.
        assert!(refuse_reserved_history("01HXYZ0123456789ABCDEFG").is_ok());
        assert!(refuse_reserved_restore("docs/a.md").is_ok());
    }
}
