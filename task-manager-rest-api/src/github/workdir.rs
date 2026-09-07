//! The working copy of one connected repository: where it lives on disk, how it is brought into
//! existence, and how the files in it are listed, read and written.
//!
//! **A connection is a real clone now, not a window.** What is under `github/<name>/…` is a folder on a
//! mounted volume, produced by `git clone` and kept current by `git fetch`. Reading a file is reading a
//! file; editing one edits it in the working tree, where git can then see it as a change. That is the
//! whole reason the design moved off holding blob references: a reference can be read and cannot be
//! edited, and the point of connecting a repository is to work in it.
//!
//! Two consequences worth stating because everything here depends on them:
//!
//! * **The working copy is a VIEW, and nothing this product offers writes into it.** A connected
//!   repository is read-only on the documents surface: files are listed and read, and the way to change
//!   one is to change it in the repository. That is what makes the volume disposable, and what makes
//!   [`reclone_repository`] — Refresh deleting the folder and cloning it again — an operation that only
//!   ever throws away a copy of something GitHub still has.
//! * **The key still does not survive a restart, and that is now survivable.** The clone stays; only
//!   reaching GitHub needs the token again. A connection that comes up `needs-key` after a deploy is
//!   still fully readable — it just cannot fetch until somebody types the key.

use std::path::{Path, PathBuf};

use task_manager_shared::documents::normalise_document_path;

use crate::board::GithubConnectionModel;
use crate::scripts::MAX_BINARY_LEN;

use super::git::{run_git_in_parent, run_git_ok};
use super::mirror::{MAX_MIRROR_FILES, MirrorEntry};

/// Where one connection's clone lives, under the configured root.
///
/// By project id and then by connection name, because a name is only unique within a project — two boards
/// may each connect a repository called `specs`, and they are two different connections that must not
/// share a working tree.
///
/// Neither segment can escape the root: a project id is a `SortableId` (digits, hex and dashes) and a
/// connection name has already been through `normalise_connection_name`, which refuses slashes, `..` and
/// everything but letters, digits, `-`, `_` and `.`.
pub fn connection_dir(git_repos_path: &str, project_id: &str, connection_name: &str) -> PathBuf {
    Path::new(git_repos_path)
        .join(project_id)
        .join(connection_name)
}

/// The root of what a connection SHOWS, which is the clone plus the folder inside the repository that was
/// connected.
pub fn connection_root(clone_dir: &Path, repo_path: &str) -> PathBuf {
    if repo_path.is_empty() {
        clone_dir.to_path_buf()
    } else {
        clone_dir.join(repo_path)
    }
}

/// The url git is pointed at.
///
/// https rather than ssh because the credential is a token: an https remote takes one as a header, where
/// an ssh remote would need a key pair written to the volume and an ssh agent to hold it.
pub fn repo_url(connection: &GithubConnectionModel) -> String {
    format!(
        "https://github.com/{}/{}.git",
        connection.owner, connection.repo
    )
}

/// Whether this folder is a clone already.
pub fn is_cloned(clone_dir: &Path) -> bool {
    clone_dir.join(".git").exists()
}

/// Clone the repository if it is not there yet.
///
/// `--branch` only when the connection named one: without it git takes the repository's default branch,
/// which is exactly what an empty branch means and saves asking which one that is.
pub async fn clone_repository(
    clone_dir: &Path,
    connection: &GithubConnectionModel,
    key: Option<&str>,
) -> Result<(), String> {
    let parent = clone_dir
        .parent()
        .ok_or_else(|| format!("'{}' has no parent folder", clone_dir.display()))?;

    let url = repo_url(connection);

    let name = clone_dir
        .file_name()
        .and_then(|itm| itm.to_str())
        .ok_or_else(|| format!("'{}' is not a folder name", clone_dir.display()))?;

    let mut args: Vec<&str> = vec!["clone"];

    if !connection.branch.is_empty() {
        args.push("--branch");
        args.push(&connection.branch);
    }

    args.push(&url);
    args.push(name);

    let output = run_git_in_parent(parent, &args, key).await?;

    if !output.success() {
        // A half-made folder would make `is_cloned` false and every command in it fail on a missing
        // `.git` for ever after. Removed so the next pull tries the clone again from nothing.
        let _ = std::fs::remove_dir_all(clone_dir);

        return Err(output.message());
    }

    Ok(())
}

/// Delete the working copy and clone it again.
///
/// **What the Refresh button does, and it is a destroy-and-replace rather than a fetch.** A fetch answers
/// "what has changed since"; this answers "give me exactly what is on GitHub now", which is the question
/// somebody is actually asking when a folder looks wrong: a file that a `.gitignore` stopped tracking, a
/// branch that was force-pushed, a working copy left on a stale branch by a connection whose row was
/// edited after it was cloned. Every one of those survives any number of fetches and none of them
/// survives this.
///
/// **It is only safe to be the button because the folder holds nothing of anybody's.** Nothing on the
/// documents surface writes into a connected repository, so what is deleted here is a copy of what the
/// remote has. The exception is somebody's own work through `github_git` — a commit that was never
/// pushed, a stash — and that goes with the folder, which is why the dialog says so before the press.
///
/// The delete comes first and on its own: a clone into a folder that already exists fails on the folder
/// rather than replacing it, so a half-deleted tree would leave the connection unusable until somebody
/// looked at the disk.
pub async fn reclone_repository(
    clone_dir: &Path,
    connection: &GithubConnectionModel,
    key: Option<&str>,
) -> Result<(), String> {
    remove_clone(clone_dir)?;

    clone_repository(clone_dir, connection, key).await
}

/// Remove a connection's folder, and ONLY when that folder is a clone.
///
/// **`remove_dir_all` is the most dangerous line in this file, so it is guarded by what the folder is
/// rather than by where it is.** The path is composed from a root, a project id and a connection name,
/// and every one of those is validated — but a check next to the delete is the one that survives somebody
/// changing how the path is built. A folder holding no `.git` is not a clone and is not this function's
/// to remove; `git clone` then makes it if it is absent or empty and refuses by name if it is not, which
/// is a better answer than deleting whatever is in there.
///
/// A folder that is not there is not a failure either: that is the state a first clone starts from, and
/// the caller's next move is the same.
fn remove_clone(clone_dir: &Path) -> Result<(), String> {
    if !is_cloned(clone_dir) {
        return Ok(());
    }

    match std::fs::remove_dir_all(clone_dir) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(format!(
            "'{}' did not delete, so there was nothing to clone into: {err}",
            clone_dir.display()
        )),
    }
}

/// Bring an existing clone up to date WITHOUT ever losing a local change.
///
/// **This is the TIMER's pass, not the button's.** Refresh deletes the folder and clones it again — see
/// [`reclone_repository`]. What runs every ten minutes has to be cheap over eight repositories that have
/// mostly not moved, so it is a fetch, and it is careful with a working tree that `github_git` may have
/// left something in.
///
/// **Fetch always, merge only when there is nothing to lose.** The timer runs this every ten minutes
/// against a working tree somebody may have run a git command in, so:
///
/// * `git fetch` is unconditional and touches nothing in the working tree;
/// * the fast-forward is attempted only when the tree is clean — no modified file, no untracked file,
///   nothing staged;
/// * only when the branch checked out is the one the fast-forward is FOR;
/// * and it is `--ff-only`, so a branch that has diverged stops rather than opening a merge nobody asked
///   for and leaving conflict markers in files somebody is about to read.
///
/// A refresh that declines to merge is not a failure and is not reported as one: the clone is exactly as
/// usable as it was, and the reason it did not move is a change somebody made. What moves it then is a
/// `git` command the caller runs deliberately — `git stash`, `git commit`, `git pull`, `git merge` — which
/// is the whole point of handing over git rather than a fixed set of buttons.
pub async fn refresh_clone(
    clone_dir: &Path,
    connection: &GithubConnectionModel,
    key: Option<&str>,
) -> Result<(), String> {
    run_git_ok(clone_dir, &["fetch", "--prune", "origin"], key).await?;

    if !is_clean(clone_dir).await? {
        return Ok(());
    }

    // Which upstream to fast-forward onto. An unset branch follows whatever the remote's HEAD is, resolved
    // here rather than assumed to be `main` — plenty of repositories are still on `master`, and some are
    // on neither.
    //
    // A clone that does not record `origin/HEAD` is not a failure and must not be reported as one: the
    // fetch above worked, every file is current on disk, and the only thing missing is the name to
    // fast-forward onto. Reported as an error it would put the connection on `failed` with a sentence
    // about a symbolic ref, which describes nothing a reader could act on.
    let upstream = match connection.branch.is_empty() {
        true => match remote_head(clone_dir).await {
            Some(head) => head,
            None => return Ok(()),
        },
        false => format!("origin/{}", connection.branch),
    };

    // **The fast-forward has to land on the branch it is FOR, and git will not check that for us.**
    // `git merge --ff-only origin/dev` run in a working copy sitting on `main` moves MAIN onto dev's tip:
    // it is a merge, and a merge does not care that the two names differ. Two ordinary things reach that
    // state. Somebody edits a connection's branch after it was cloned — the clone is never re-checked-out,
    // so the row says `dev` and the working copy is still on `main`. Or somebody does the thing the
    // `github_git` description recommends and works on a branch of their own, at which point a timer that
    // fast-forwarded it onto the connection's branch would be rewriting their work in the background.
    //
    // Either way nothing would report it: the merge succeeds, the state stays `ready`, and one branch's
    // commits are on another branch's ref. So the names are compared first, and a mismatch declines the
    // merge exactly as a dirty tree does.
    let Some(wanted) = upstream.strip_prefix("origin/") else {
        return Ok(());
    };

    // Detached HEAD answers nothing, which is also a state not to merge into.
    let Some(checked_out) = current_branch(clone_dir).await else {
        return Ok(());
    };

    if checked_out != wanted {
        return Ok(());
    }

    // `--ff-only` failing is the ordinary answer for a branch with local commits on it, not an error to
    // report: the clone stays where it is and the caller can merge or rebase when they choose to.
    let _ = run_git_ok(clone_dir, &["merge", "--ff-only", &upstream], key).await;

    Ok(())
}

/// The branch the working copy has checked out, or `None` on a detached HEAD.
async fn current_branch(clone_dir: &Path) -> Option<String> {
    let branch = run_git_ok(clone_dir, &["symbolic-ref", "--short", "HEAD"], None)
        .await
        .ok()?;

    let branch = branch.trim();

    match branch.is_empty() {
        true => None,
        false => Some(branch.to_string()),
    }
}

/// Whether the working tree has nothing in it that a fast-forward could destroy.
///
/// `--porcelain` is empty exactly when there is nothing staged, nothing modified and nothing untracked
/// that is not ignored — which is the condition a fast-forward is safe under.
async fn is_clean(clone_dir: &Path) -> Result<bool, String> {
    let status = run_git_ok(clone_dir, &["status", "--porcelain"], None).await?;

    Ok(status.trim().is_empty())
}

/// What the remote calls its default branch, as a ref this side can merge, or `None` when the clone does
/// not record one.
///
/// Written by `git clone`, so this is a read of the clone rather than a question for the network — and
/// `None` rather than an error because a clone without it is merely one this cannot fast-forward
/// automatically, not one that is broken.
async fn remote_head(clone_dir: &Path) -> Option<String> {
    let head = run_git_ok(
        clone_dir,
        &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"],
        None,
    )
    .await
    .ok()?;

    let head = head.trim();

    match head.is_empty() {
        true => None,
        false => Some(head.to_string()),
    }
}

/// The commit the working copy is on, short form. Empty when the clone has no commits yet.
pub async fn head_commit(clone_dir: &Path) -> String {
    run_git_ok(clone_dir, &["rev-parse", "--short", "HEAD"], None)
        .await
        .map(|itm| itm.trim().to_string())
        .unwrap_or_default()
}

/// Every file the connection shows, read off the working copy.
///
/// **`git ls-files --cached --others --exclude-standard` rather than a directory walk**, and the three
/// flags are the whole of why:
///
/// * `--cached` is what git tracks;
/// * `--others` adds what is in the working tree and not yet tracked — which is how a file just created
///   through `documents_upload` appears in the listing immediately, before anybody commits it;
/// * `--exclude-standard` applies `.gitignore`, so a `target/` or a `node_modules/` somebody built inside
///   the clone does not become forty thousand rows in a documents listing.
///
/// It also never returns anything inside `.git`, which a directory walk would have had to remember to
/// skip.
pub async fn list_working_copy(
    clone_dir: &Path,
    repo_path: &str,
) -> Result<(Vec<MirrorEntry>, usize), String> {
    let listed = run_git_ok(
        clone_dir,
        &["ls-files", "--cached", "--others", "--exclude-standard", "-z"],
        None,
    )
    .await?;

    let root = connection_root(clone_dir, repo_path);

    let mut entries: Vec<MirrorEntry> = Vec::new();
    let mut skipped: usize = 0;

    // NUL-separated, which is the only listing git offers that a filename containing a newline cannot lie
    // about — and repositories do contain those.
    for path in listed.split('\0').filter(|itm| !itm.is_empty()) {
        // Asked for the size only once the path is known to be in scope, so a repository whose connected
        // folder is one directory of forty does not `stat` the other thirty-nine.
        let Some(relative) = in_scope(path, repo_path) else {
            continue;
        };

        // A path git lists and the filesystem does not have is a staged deletion, or a file removed from
        // under us between the listing and this line. Neither is a file to show.
        let Ok(metadata) = std::fs::metadata(root.join(&relative)) else {
            continue;
        };

        match classify(&relative, metadata.len() as i64, entries.len()) {
            Listed::Entry(entry) => entries.push(entry),
            Listed::Skipped => skipped += 1,
        }
    }

    // Sorted by path, exactly as the documents index is, so everything under one folder is contiguous and
    // a tree can be built by walking the list once.
    entries.sort_by(|left, right| left.path.cmp(&right.path));

    Ok((entries, skipped))
}

/// What became of one file git listed.
///
/// There is no `Outside` here on purpose: scope is decided by [`in_scope`] before a file is looked at, and
/// a file that was never asked for is not a file that was left out. Only the difference between shown and
/// deliberately-not-shown is worth counting, because only that one answers "is the file I want missing
/// because it was filtered, or because it is not there".
enum Listed {
    Entry(MirrorEntry),
    Skipped,
}

/// Whether a path git listed is inside the connected folder, and what it is called once it is.
fn in_scope(path: &str, repo_path: &str) -> Option<String> {
    let relative = strip_root(path, repo_path)?;

    // The same rule every document path in this product goes through, which is also the sanitiser. A
    // repository path this product would not name is dropped rather than mangled into one it would.
    normalise_document_path(relative).ok()
}

/// Decide whether one in-scope file is shown, and as what.
///
/// Pure, and separate from everything that touches a disk, because this is where the rules that matter
/// live: what is too big to ever be read, how many files one connection may show, and what a browser is
/// told a file is.
///
/// **A file over the single-document limit is skipped rather than listed.** Listing it would put a row in
/// the tree that every read refuses — the honest thing is for it not to be there, with the count saying
/// something was left out.
fn classify(relative: &str, size: i64, shown_so_far: usize) -> Listed {
    if shown_so_far >= MAX_MIRROR_FILES || size > MAX_BINARY_LEN as i64 {
        return Listed::Skipped;
    }

    let content_type = crate::scripts::content_type_of(None, relative);

    // **What we CLAIM and what we try to READ are two different questions, and an unknown extension
    // answers them differently.** The type above says `application/octet-stream` when the table has never
    // heard of the extension, because a browser must not be told a file is something nobody checked. But a
    // file with an unfamiliar extension in a repository is a `.rst`, a `.gradle` or somebody's own suffix
    // far more often than it is a binary — so it is still READ as text first, and the read falls back to
    // bytes the moment the bytes disagree.
    let is_binary = match task_manager_shared::documents::content_type_for_path(relative) {
        Some(known) => !crate::documents::is_text_content_type(known),
        None => false,
    };

    Listed::Entry(MirrorEntry {
        path: relative.to_string(),
        size,
        content_type,
        is_binary,
    })
}

/// A repository path with the connected folder taken off the front, or `None` when it is not under it.
///
/// The folder somebody connected becomes the ROOT of the tree — that is what makes a path mean what a
/// reader expects when they browse into a repository and copy the address.
fn strip_root<'s>(path: &'s str, repo_path: &str) -> Option<&'s str> {
    if repo_path.is_empty() {
        return Some(path);
    }

    // The slash is what makes `docs` not match `docs2/a.md`.
    path.strip_prefix(repo_path)?.strip_prefix('/')
}

/// Where one file of a connection is on disk, refusing anything that would land outside it.
///
/// **Belt and braces, and both are load-bearing.** `normalise_document_path` has already refused `..` and
/// absolute paths by the time most callers get here — but this is the function that turns a caller's
/// string into a filesystem path, and a check next to the thing it protects is the one that survives a
/// refactor. The second half re-checks the result rather than the input, which is what catches a
/// component that only becomes an escape once the pieces are joined.
pub fn file_path(clone_dir: &Path, repo_path: &str, relative: &str) -> Result<PathBuf, String> {
    let relative = normalise_document_path(relative)?;

    let root = connection_root(clone_dir, repo_path);
    let full = root.join(&relative);

    // Compared against the root as it is spelled, since neither exists yet in the create case and
    // `canonicalize` needs a file that is there.
    if !full.starts_with(&root) {
        return Err(format!(
            "'{relative}' does not name a file inside this connection"
        ));
    }

    Ok(full)
}

/// Read one file of the working copy.
pub fn read_file(path: &Path) -> Result<Vec<u8>, String> {
    let metadata = std::fs::metadata(path)
        .map_err(|_| "that file is not in the working copy".to_string())?;

    if metadata.len() > MAX_BINARY_LEN as u64 {
        return Err(format!(
            "that file is {} bytes — the limit for one document is {MAX_BINARY_LEN}",
            metadata.len()
        ));
    }

    std::fs::read(path).map_err(|err| format!("that file did not read: {err}"))
}

// There is deliberately no `write_file` and no `delete_file` here. A connected repository is read-only on
// this surface, and the place that rule is worth being unable to break is the module that owns the paths:
// a helper that writes into a working copy is one call away from being reached again.

#[cfg(test)]
mod tests {
    use super::*;

    fn connection(branch: &str, repo_path: &str) -> GithubConnectionModel {
        GithubConnectionModel {
            name: "specs".to_string(),
            owner: "MyJetTools".to_string(),
            repo: "fl-url".to_string(),
            branch: branch.to_string(),
            repo_path: repo_path.to_string(),
        }
    }

    #[test]
    fn a_clone_lives_under_its_project_and_then_its_name() {
        let dir = connection_dir("/root/git-repos", "01HX-abc", "specs");

        assert_eq!(dir, PathBuf::from("/root/git-repos/01HX-abc/specs"));

        // Two boards may each connect something called `specs`, and they are two working trees.
        assert_ne!(dir, connection_dir("/root/git-repos", "01HY-def", "specs"));
    }

    #[test]
    fn the_connected_folder_becomes_the_root() {
        assert_eq!(strip_root("docs/design/a.md", "docs"), Some("design/a.md"));
        assert_eq!(strip_root("docs/a.md", ""), Some("docs/a.md"));

        // A sibling that merely starts with the same letters is not inside it.
        assert_eq!(strip_root("docs2/a.md", "docs"), None);
        assert_eq!(strip_root("src/main.rs", "docs"), None);

        let clone = Path::new("/repos/p/specs");
        assert_eq!(connection_root(clone, ""), clone);
        assert_eq!(
            connection_root(clone, "docs"),
            PathBuf::from("/repos/p/specs/docs")
        );
    }

    /// The function that turns a caller's string into a filesystem path is the one that must not be
    /// talked out of the folder it belongs to.
    #[test]
    fn a_path_that_would_leave_the_connection_is_refused() {
        let clone = Path::new("/repos/p/specs");

        assert_eq!(
            file_path(clone, "docs", "design/a.md").unwrap(),
            PathBuf::from("/repos/p/specs/docs/design/a.md")
        );

        // `..` is refused outright, at any depth and in any position.
        for bad in ["../../../etc/passwd", "a/../../b", "..", "docs/../../x"] {
            assert!(file_path(clone, "docs", bad).is_err(), "{bad}");
        }

        // An ABSOLUTE path is not refused — it is contained, which is the same reading every document
        // path in this product gets: the leading slash is a spelling of the root, and the root is here.
        // Worth a test of its own precisely because "it was not refused" looks alarming until you see
        // where it landed.
        assert_eq!(
            file_path(clone, "docs", "/etc/passwd").unwrap(),
            PathBuf::from("/repos/p/specs/docs/etc/passwd")
        );
    }

    fn entry_of(relative: &str, size: i64) -> Option<MirrorEntry> {
        match classify(relative, size, 0) {
            Listed::Entry(entry) => Some(entry),
            Listed::Skipped => None,
        }
    }

    /// A path this product will not name is counted rather than mangled into one it would.
    #[test]
    fn a_path_that_is_not_a_document_path_is_out_of_scope() {
        assert_eq!(in_scope("docs/a.md", "docs"), Some("a.md".to_string()));
        assert_eq!(in_scope("../escape.md", ""), None);
        assert_eq!(in_scope("src/main.rs", "docs"), None);
    }

    /// **A page in a repository is more than one file, and the listing is where its types are decided.**
    /// This is the regression test for a mirrored page rendering bare: a stylesheet listed as anything but
    /// `text/css` is refused by the browser.
    #[test]
    fn a_pages_own_assets_are_listed_as_what_they_are() {
        for (path, expected) in [
            ("index.html", "text/html"),
            ("css/design-system.css", "text/css"),
            ("js/design-system.js", "text/javascript"),
            ("build.py", "text/plain"),
            ("img/icon.png", "image/png"),
        ] {
            assert_eq!(entry_of(path, 10).unwrap().content_type, expected, "{path}");
        }
    }

    /// The two questions an unknown extension answers differently: it is OFFERED as bytes, because
    /// claiming a type nobody checked is what broke the page above — and it is still READ as text, because
    /// a file with an unfamiliar suffix in a repository is somebody's own convention far more often than it
    /// is a binary, and the read verifies the bytes anyway.
    #[test]
    fn an_extension_nobody_knows_is_offered_as_bytes_and_still_read_as_text() {
        let unknown = entry_of("notes.unknownext", 10).unwrap();

        assert_eq!(unknown.content_type, "application/octet-stream");
        assert!(!unknown.is_binary, "still read as text first");

        // What the table DOES know is binary stays binary, and is never read as text at all.
        let image = entry_of("logo.png", 10).unwrap();
        assert_eq!(image.content_type, "image/png");
        assert!(image.is_binary);
    }

    /// A file nothing could ever read is not put in the tree — it would be a row every read refuses. The
    /// count is what says something was left out.
    #[test]
    fn a_file_too_big_to_read_is_left_out_and_counted() {
        assert!(entry_of("big.bin", MAX_BINARY_LEN as i64 + 1).is_none());
        assert!(entry_of("fine.md", 4).is_some());

        // And the ceiling on how many one connection shows at all.
        assert!(matches!(
            classify("fine.md", 4, MAX_MIRROR_FILES),
            Listed::Skipped
        ));
    }

    #[test]
    fn the_remote_is_https_so_the_token_can_be_a_header() {
        let url = repo_url(&connection("main", ""));

        assert_eq!(url, "https://github.com/MyJetTools/fl-url.git");
        assert!(url.starts_with("https://"), "a token is not an ssh key");
    }
}
