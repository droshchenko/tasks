use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ahash::AHashMap;
use arc_swap::ArcSwap;
use parking_lot::Mutex;
use rust_extensions::date_time::DateTimeAsMicroseconds;
use task_manager_shared::documents::normalise_document_path;
use task_manager_shared::github::GithubMirrorState;

use crate::scripts::MAX_BINARY_LEN;

/// The most files one mirror may hold.
///
/// A repository is not a documents folder — it has a `node_modules`, a `target`, a history of somebody's
/// build outputs — and a connection that pulled a hundred thousand of them would be answering
/// `documents_list` with a hundred thousand paths an agent has to read past to find the four documents
/// it wanted. The cap is a statement about what this is for.
pub const MAX_MIRROR_FILES: usize = 5_000;

/// The most a mirror may occupy on disk, unpacked.
///
/// Counted as entries are written rather than read from the zip's headers, which are written by whoever
/// made the archive — the same reason the upload path counts rather than trusts.
pub const MAX_MIRROR_BYTES: u64 = 128 * 1024 * 1024;

/// One mirrored file, as everything above the disk sees it.
///
/// **There is no id and no version here, and that is the point.** A mirrored file is a window onto
/// somebody else's repository: it has no history in this product, nothing references it, and the next
/// pull may replace or remove it without anybody being told. What it has is a path, a size and a kind —
/// exactly what a tree needs to draw it and what a reader needs to decide whether to open it.
#[derive(Debug, Clone)]
pub struct MirrorEntry {
    /// Relative to the connection's root — so a connection rooted at `docs` reports `design/a.md`, not
    /// `docs/design/a.md`. What the reader chose is the root; what is under it is the tree.
    pub path: String,
    pub size: i64,
    pub content_type: String,
    pub is_binary: bool,
}

/// What one connection currently holds, and how the last attempt to fill it went.
///
/// **A failed pull never empties a mirror that worked.** `entries` and `commit` are what the last
/// SUCCESSFUL pull left; `state` and `error` are about the last attempt. A repository that goes
/// unreachable overnight is a warning beside a folder somebody can still read, not a folder that
/// silently emptied itself.
#[derive(Debug, Clone)]
pub struct Mirror {
    pub state: &'static str,
    pub error: String,
    pub commit: String,
    pub pulled: Option<DateTimeAsMicroseconds>,
    /// Where the files are, inside the container. Empty until something has been pulled.
    pub dir: PathBuf,
    pub entries: Arc<Vec<MirrorEntry>>,
    /// How many files the repository holds that the mirror deliberately does not — over the size limit,
    /// or at a path this product will not name. Counted rather than listed: it is the difference between
    /// "that file is not there" and "that file is not there YET", and one number answers it.
    pub skipped_amount: usize,
}

impl Mirror {
    fn empty(dir: PathBuf, state: &'static str) -> Self {
        Self {
            state,
            error: String::new(),
            commit: String::new(),
            pulled: None,
            dir,
            entries: Arc::new(Vec::new()),
            skipped_amount: 0,
        }
    }

    pub fn entry(&self, path: &str) -> Option<&MirrorEntry> {
        self.entries.iter().find(|itm| itm.path == path)
    }

    /// Where one mirrored file is on disk, or `None` when the mirror does not hold it.
    ///
    /// Resolved through the entry list rather than by joining the path onto the directory, which is the
    /// difference between serving a file this mirror published and serving whatever a caller's path
    /// happened to reach.
    pub fn file_on_disk(&self, path: &str) -> Option<PathBuf> {
        let entry = self.entry(path)?;
        Some(self.dir.join(&entry.path))
    }
}

/// Every mirror the process holds, plus the keys that fill them.
///
/// **Nothing here survives a restart, deliberately.** The connections do — they are rows on a project —
/// but a key is held only in this process's memory and the trees are only in its temp directory, so a
/// restart comes up with the configuration intact and nothing downloaded. The first tick of the timer
/// refills whatever can be refilled anonymously; a private repository waits for somebody to type its key
/// again. That is the cost of never writing a credential down, and it was chosen.
pub struct GithubMirrors {
    root: PathBuf,
    // Tokens, by connection. A `Mutex` rather than an `ArcSwap` because this is written as often as it
    // is read and is never read on a hot path — and because a key must not be cloned into a snapshot
    // that outlives the moment it is used.
    keys: Mutex<AHashMap<String, String>>,
    mirrors: ArcSwap<AHashMap<String, Arc<Mirror>>>,
    // Which connections have a pull running. Two pulls of one connection would race over the same
    // directory, and the loser would swap its tree in under the winner's entry list.
    pulling: Mutex<ahash::AHashSet<String>>,
}

/// Held for the length of one pull, and the reason a second one does not start.
///
/// A guard rather than a flag on the mirror because a pull that panics still has to release it, and a
/// `Drop` is the only release that cannot be forgotten.
pub struct PullGuard<'s> {
    mirrors: &'s GithubMirrors,
    key: String,
}

impl Drop for PullGuard<'_> {
    fn drop(&mut self) {
        self.mirrors.pulling.lock().remove(&self.key);
    }
}

impl GithubMirrors {
    /// Take a temp directory and start from nothing in it.
    ///
    /// **The wipe is on purpose and is the recovery story.** Whatever is in there belongs to a process
    /// that is no longer running: its connections may have been deleted, retargeted or renamed while
    /// this one was not looking, so every tree in there is of unknown provenance. Rebuilding from the
    /// repositories costs one pull per connection and is the only way to be sure what is on disk is what
    /// the configuration says.
    pub fn new(root: PathBuf) -> Self {
        // Best effort, and a failure is not fatal: the per-connection install below replaces a directory
        // wholesale anyway, so the worst case of a failed wipe is disk held by a connection nobody has.
        let _ = std::fs::remove_dir_all(&root);

        if let Err(err) = std::fs::create_dir_all(&root) {
            // Said out loud rather than panicked on: a service that cannot mirror is still a service
            // that runs a board, and every read below degrades to "nothing pulled".
            println!(
                "github: could not create the mirror directory {}: {err}",
                root.to_string_lossy()
            );
        }

        Self {
            root,
            keys: Mutex::new(AHashMap::new()),
            mirrors: ArcSwap::from_pointee(AHashMap::new()),
            pulling: Mutex::new(ahash::AHashSet::new()),
        }
    }

    /// Claim a connection for one pull, or `None` when one is already running.
    ///
    /// `None` is not an error anywhere it is checked: the timer skipping a connection that a person just
    /// asked to refresh is the timer doing the right thing.
    pub fn try_begin_pull(&self, project_id: &str, name: &str) -> Option<PullGuard<'_>> {
        let key = key_of(project_id, name);

        if !self.pulling.lock().insert(key.clone()) {
            return None;
        }

        Some(PullGuard { mirrors: self, key })
    }

    /// Where one connection's files live.
    ///
    /// The name is sanitised for the filesystem AND suffixed with a hash of the real one, so two names
    /// that sanitise to the same string cannot land in one directory. The readable half is kept because
    /// somebody debugging this will be looking at `ls` inside a container.
    pub fn dir_for(&self, project_id: &str, name: &str) -> PathBuf {
        self.root
            .join(sanitise_for_disk(project_id))
            .join(sanitise_for_disk(name))
    }

    pub fn get(&self, project_id: &str, name: &str) -> Option<Arc<Mirror>> {
        self.mirrors.load().get(&key_of(project_id, name)).cloned()
    }

    /// The mirror for a connection, or an empty one saying nothing has been pulled yet.
    ///
    /// A caller drawing a tree wants a mirror either way — "connected, nothing yet" is a state to draw,
    /// not an absence to branch on.
    pub fn get_or_pending(&self, project_id: &str, name: &str) -> Arc<Mirror> {
        match self.get(project_id, name) {
            Some(mirror) => mirror,
            None => Arc::new(Mirror::empty(
                self.dir_for(project_id, name),
                GithubMirrorState::PENDING,
            )),
        }
    }

    pub fn put(&self, project_id: &str, name: &str, mirror: Mirror) {
        self.update(|map| {
            map.insert(key_of(project_id, name), Arc::new(mirror));
        });
    }

    /// Record what a pull is doing without touching what it has already delivered.
    ///
    /// This is what keeps a failure from emptying a folder: the entries, the commit and the moment of
    /// the last success are carried over from whatever was there.
    pub fn set_state(&self, project_id: &str, name: &str, state: &'static str, error: String) {
        let previous = self.get_or_pending(project_id, name);

        let mut mirror = (*previous).clone();
        mirror.state = state;
        mirror.error = error;

        self.put(project_id, name, mirror);
    }

    /// Forget a connection entirely — its tree, and its key.
    ///
    /// Both, and in that order, because this is called when somebody deletes the connection: leaving the
    /// key behind would mean a connection re-created under the same name silently inherits a credential
    /// nobody re-entered.
    pub fn forget(&self, project_id: &str, name: &str) {
        let dir = self
            .get(project_id, name)
            .map(|itm| itm.dir.clone())
            .unwrap_or_else(|| self.dir_for(project_id, name));

        self.update(|map| {
            map.remove(&key_of(project_id, name));
        });

        self.keys.lock().remove(&key_of(project_id, name));

        let _ = std::fs::remove_dir_all(dir);
    }

    /// Hold a key for a connection, or forget the one being held when it is empty.
    pub fn set_key(&self, project_id: &str, name: &str, key: &str) {
        let key = key.trim();
        let map_key = key_of(project_id, name);

        let mut keys = self.keys.lock();

        if key.is_empty() {
            keys.remove(&map_key);
        } else {
            keys.insert(map_key, key.to_string());
        }
    }

    /// The key for a connection, cloned for the duration of one call.
    ///
    /// Cloned rather than handed out behind the lock so a slow request cannot hold every other
    /// connection's key store — and never stored anywhere by the caller.
    pub fn key(&self, project_id: &str, name: &str) -> Option<String> {
        self.keys.lock().get(&key_of(project_id, name)).cloned()
    }

    pub fn has_key(&self, project_id: &str, name: &str) -> bool {
        self.keys.lock().contains_key(&key_of(project_id, name))
    }

    fn update(&self, apply: impl FnOnce(&mut AHashMap<String, Arc<Mirror>>)) {
        // Read-copy-update. The map is small — one entry per connection across the whole product — and is
        // read on every documents listing, which is the shape `ArcSwap` is for.
        let mut next = (**self.mirrors.load()).clone();
        apply(&mut next);
        self.mirrors.store(Arc::new(next));
    }
}

fn key_of(project_id: &str, name: &str) -> String {
    // A newline separates them because neither a project id nor a connection name can contain one, so
    // no pair of (project, name) can collide with another.
    format!("{project_id}\n{name}")
}

/// A name that is safe as one directory component, and still recognisable.
///
/// The hash is what makes it correct rather than merely tidy: `спеки` and `docs` both sanitise to
/// underscores and `d_c_`, and two connections sharing a directory would serve each other's files.
fn sanitise_for_disk(src: &str) -> String {
    let mut safe = String::with_capacity(src.len());

    for character in src.chars() {
        if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
            safe.push(character);
        } else {
            safe.push('_');
        }
    }

    // `.` and `..` are directory names with a meaning of their own, and an empty one is not a name.
    if safe.is_empty() || safe.chars().all(|itm| itm == '.') {
        safe = "x".to_string();
    }

    format!("{safe}-{:08x}", short_hash(src))
}

/// FNV-1a, 32 bits. Not a cryptographic choice and does not need to be: it separates two names that
/// sanitise alike, and both names are already ours rather than an attacker's.
fn short_hash(src: &str) -> u32 {
    let mut hash: u32 = 0x811c9dc5;

    for byte in src.as_bytes() {
        hash ^= *byte as u32;
        hash = hash.wrapping_mul(0x0100_0193);
    }

    hash
}

/// Unpack an archive into a connection's directory, replacing whatever was there.
///
/// **Written beside the live directory and swapped in, never written over it.** A pull that dies halfway
/// through must not leave a folder somebody is reading half-replaced, and a rename is the only step here
/// that is atomic enough to promise that.
///
/// The `{owner}-{repo}-{sha}/` wrapper GitHub puts around a zipball is stripped, and then `repo_path`
/// is stripped too — so a connection rooted at `docs` produces `design/a.md`, and the folder the reader
/// chose is the root of the tree rather than the first thing inside it.
pub fn install_archive(
    dir: &Path,
    archive: &[u8],
    repo_path: &str,
) -> Result<(Vec<MirrorEntry>, usize), String> {
    let staging = staging_dir(dir);

    // A staging directory left by a pull that died is of unknown content, so it goes before anything is
    // written into it.
    let _ = std::fs::remove_dir_all(&staging);

    std::fs::create_dir_all(&staging)
        .map_err(|err| format!("could not make room for the download: {err}"))?;

    let result = unpack_into(&staging, archive, repo_path);

    let (entries, skipped) = match result {
        Ok(result) => result,
        Err(err) => {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(err);
        }
    };

    // The swap. `rename` cannot replace a non-empty directory, so the old one goes first — which leaves a
    // window where neither exists. That window is why the entry list above is what readers go through:
    // a read landing inside it finds a file missing rather than finding somebody else's.
    let _ = std::fs::remove_dir_all(dir);

    if let Some(parent) = dir.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("could not make room for the download: {err}"))?;
    }

    std::fs::rename(&staging, dir).map_err(|err| {
        let _ = std::fs::remove_dir_all(&staging);
        format!("could not put the download in place: {err}")
    })?;

    Ok((entries, skipped))
}

fn staging_dir(dir: &Path) -> PathBuf {
    let mut name = dir
        .file_name()
        .map(|itm| itm.to_string_lossy().to_string())
        .unwrap_or_else(|| "mirror".to_string());

    name.push_str(".pulling");

    match dir.parent() {
        Some(parent) => parent.join(name),
        None => PathBuf::from(name),
    }
}

/// Read the zip and write what belongs in the mirror, counting what does not.
///
/// Separate from the swap above so the rules — what is a directory, what escapes, what is too big — can
/// be tested against a real archive without a filesystem dance around them.
fn unpack_into(
    into: &Path,
    archive: &[u8],
    repo_path: &str,
) -> Result<(Vec<MirrorEntry>, usize), String> {
    if archive.is_empty() {
        return Err("GitHub sent an empty archive".to_string());
    }

    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(archive))
        .map_err(|err| format!("that download is not a zip we can read: {err}"))?;

    let mut entries: Vec<MirrorEntry> = Vec::new();
    let mut skipped: usize = 0;
    let mut written: u64 = 0;

    for index in 0..zip.len() {
        let mut file = zip
            .by_index(index)
            .map_err(|err| format!("the archive could not be read at entry {index}: {err}"))?;

        let name = file.name().to_string();

        if file.is_dir() || name.ends_with('/') {
            continue;
        }

        let Some(relative) = strip_roots(&name, repo_path) else {
            // Outside the folder that was connected. Not a skip — it was never asked for.
            continue;
        };

        // The same rule every document path in this product goes through, which is also the sanitiser:
        // `..` is refused here, so nothing below can write outside the directory it was given.
        let Ok(relative) = normalise_document_path(relative) else {
            skipped += 1;
            continue;
        };

        if entries.len() >= MAX_MIRROR_FILES {
            skipped += 1;
            continue;
        }

        let declared = file.size();

        if declared > MAX_BINARY_LEN as u64 {
            skipped += 1;
            continue;
        }

        if written + declared > MAX_MIRROR_BYTES {
            skipped += 1;
            continue;
        }

        let mut bytes = Vec::with_capacity(declared as usize);

        if file.read_to_end(&mut bytes).is_err() {
            skipped += 1;
            continue;
        }

        // An empty file is a real thing in a repository and nothing this can serve — and it is exactly
        // what the sync path would refuse one step later, with a worse message.
        if bytes.is_empty() {
            skipped += 1;
            continue;
        }

        let target = into.join(&relative);

        if let Some(parent) = target.parent() {
            if std::fs::create_dir_all(parent).is_err() {
                skipped += 1;
                continue;
            }
        }

        if std::fs::write(&target, &bytes).is_err() {
            skipped += 1;
            continue;
        }

        written += bytes.len() as u64;

        let content_type = crate::scripts::content_type_of(None, &relative);

        entries.push(MirrorEntry {
            size: bytes.len() as i64,
            is_binary: !crate::documents::is_text_content_type(&content_type),
            content_type,
            path: relative,
        });
    }

    // Sorted by path, exactly as the documents index is, so everything under one folder is contiguous
    // and a tree can be built by walking the list once.
    entries.sort_by(|left, right| left.path.cmp(&right.path));

    Ok((entries, skipped))
}

/// Strip GitHub's archive wrapper and then the connected folder, or say this entry is not in scope.
///
/// A zipball's every path starts with one folder named `{owner}-{repo}-{sha}` — the sha is why it cannot
/// be matched by name and is taken as "whatever the first segment is" instead.
fn strip_roots<'s>(name: &'s str, repo_path: &str) -> Option<&'s str> {
    let (_, rest) = name.split_once('/')?;

    if repo_path.is_empty() {
        return Some(rest);
    }

    let rest = rest.strip_prefix(repo_path)?;

    // The folder itself, or a sibling whose name merely starts the same way — `docs2/a.md` is not in
    // `docs`.
    rest.strip_prefix('/')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
        use zip::write::SimpleFileOptions;

        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));

        for (name, bytes) in files {
            writer
                .start_file(*name, SimpleFileOptions::default())
                .unwrap();
            std::io::Write::write_all(&mut writer, bytes).unwrap();
        }

        writer.finish().unwrap().into_inner()
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tm-github-test-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// The wrapper GitHub puts round a zipball carries a sha, so it is stripped by position rather than
    /// by name.
    #[test]
    fn the_archive_wrapper_is_stripped_whatever_it_is_called() {
        assert_eq!(strip_roots("owner-repo-abc123/readme.md", ""), Some("readme.md"));
        assert_eq!(
            strip_roots("owner-repo-abc123/docs/a.md", ""),
            Some("docs/a.md")
        );

        // Nothing above the wrapper is an entry at all.
        assert_eq!(strip_roots("readme.md", ""), None);
    }

    /// The folder somebody connected becomes the ROOT of the tree — that is what makes `path` mean what
    /// a reader expects when they browse into a repository and copy the address.
    #[test]
    fn the_connected_folder_becomes_the_root() {
        assert_eq!(
            strip_roots("owner-repo-abc/docs/design/a.md", "docs"),
            Some("design/a.md")
        );

        // A sibling that merely starts with the same letters is not inside it.
        assert_eq!(strip_roots("owner-repo-abc/docs2/a.md", "docs"), None);
        // Nor is anything else in the repository.
        assert_eq!(strip_roots("owner-repo-abc/src/main.rs", "docs"), None);
    }

    #[test]
    fn a_real_archive_lands_as_files_on_disk() {
        let dir = temp_dir("install");

        let archive = zip_of(&[
            ("owner-repo-abc/docs/readme.md", b"# hello"),
            ("owner-repo-abc/docs/img/logo.png", &[0x89, 0x50, 0x4E, 0x47]),
            ("owner-repo-abc/src/main.rs", b"fn main() {}"),
        ]);

        let (entries, skipped) = install_archive(&dir, &archive, "docs").unwrap();

        assert_eq!(skipped, 0);
        // `src/` was never in scope, so it is not a skip and not an entry.
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].path, "img/logo.png");
        assert!(entries[0].is_binary);
        assert_eq!(entries[1].path, "readme.md");
        assert!(!entries[1].is_binary);
        assert_eq!(entries[1].content_type, "text/markdown");

        assert_eq!(
            std::fs::read_to_string(dir.join("readme.md")).unwrap(),
            "# hello"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The second pull replaces the first wholesale — a file deleted upstream must not survive in the
    /// mirror, which is exactly what an in-place unpack would leave behind.
    #[test]
    fn a_second_pull_replaces_the_tree_rather_than_merging_into_it() {
        let dir = temp_dir("replace");

        let first = zip_of(&[
            ("owner-repo-a/one.md", b"one"),
            ("owner-repo-a/two.md", b"two"),
        ]);
        install_archive(&dir, &first, "").unwrap();
        assert!(dir.join("two.md").exists());

        let second = zip_of(&[("owner-repo-b/one.md", b"one again")]);
        let (entries, _) = install_archive(&dir, &second, "").unwrap();

        assert_eq!(entries.len(), 1);
        assert!(!dir.join("two.md").exists(), "a deleted file must not survive");
        assert_eq!(
            std::fs::read_to_string(dir.join("one.md")).unwrap(),
            "one again"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A path that would escape the mirror is counted and dropped, not written.
    #[test]
    fn an_entry_that_would_escape_is_skipped() {
        let dir = temp_dir("escape");

        let archive = zip_of(&[
            ("owner-repo-a/../../etc/passwd", b"nope"),
            ("owner-repo-a/empty.md", b""),
            ("owner-repo-a/fine.md", b"fine"),
        ]);

        let (entries, skipped) = install_archive(&dir, &archive, "").unwrap();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, "fine.md");
        assert_eq!(skipped, 2, "the escape and the empty file");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Two names that sanitise to the same string must not share a directory, or one connection serves
    /// the other's files.
    #[test]
    fn two_names_that_sanitise_alike_still_get_their_own_directory() {
        assert_ne!(sanitise_for_disk("a b"), sanitise_for_disk("a/b"));
        assert_ne!(sanitise_for_disk("спеки"), sanitise_for_disk("docs"));

        // The readable half survives, because somebody will be reading this with `ls`.
        assert!(sanitise_for_disk("specs").starts_with("specs-"));

        // A name that is all dots is a directory name with a meaning of its own.
        assert!(!sanitise_for_disk("..").starts_with(".."));
    }

    #[test]
    fn what_is_not_an_archive_is_refused() {
        assert!(unpack_into(&temp_dir("bad"), b"", "").is_err());
        assert!(unpack_into(&temp_dir("bad"), b"not a zip at all", "").is_err());
    }
}
