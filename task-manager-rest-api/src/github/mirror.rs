use std::sync::Arc;

use ahash::AHashMap;
use arc_swap::ArcSwap;
use parking_lot::Mutex;
use rust_extensions::date_time::DateTimeAsMicroseconds;
use task_manager_shared::documents::normalise_document_path;

use crate::scripts::MAX_BINARY_LEN;

use super::client::TreeEntry;

/// The most files one connection may list.
///
/// A repository is not a documents folder — it has a `node_modules`, a `target`, a history of somebody's
/// build outputs — and a connection listing a hundred thousand of them would be answering
/// `documents_list` with a hundred thousand paths an agent has to read past to find the four documents
/// it wanted. The cap is a statement about what this is for.
pub const MAX_MIRROR_FILES: usize = 5_000;

/// One mirrored file, as everything above sees it: WHERE it is and WHICH bytes it is, never the bytes.
///
/// **There is no content here and none is ever stored.** This service holds references. A file's bytes
/// are fetched from GitHub at the moment somebody asks for that file and are let go immediately
/// afterwards, so a connected repository costs a list of paths however large it is, and the process is
/// never holding more than the one file being read.
///
/// There is no id and no version either, and that is the same point from the other side: a mirrored file
/// is a window onto somebody else's repository. It has no history in this product, nothing references
/// it, and the next listing may replace or remove it without anybody being told.
#[derive(Debug, Clone)]
pub struct MirrorEntry {
    /// Relative to the connection's root — so a connection rooted at `docs` reports `design/a.md`, not
    /// `docs/design/a.md`. What the reader chose is the root; what is under it is the tree.
    pub path: String,
    pub size: i64,
    /// The blob this file is, as GitHub names it. **This is the reference** — it is how the content is
    /// asked for, and it changes exactly when the file's content changes.
    pub sha: String,
    pub content_type: String,
    pub is_binary: bool,
}

/// What one connection currently knows, and how the last attempt to refresh it went.
///
/// **A failed listing never empties a mirror that worked.** `entries` and `commit` are what the last
/// SUCCESSFUL listing left; `state` and `error` are about the last attempt. A repository that goes
/// unreachable overnight is a warning beside a folder somebody can still read, not a folder that
/// silently emptied itself — and since nothing was ever downloaded, "still readable" means the files
/// are still fetchable the moment GitHub answers again.
#[derive(Debug, Clone)]
pub struct Mirror {
    pub state: &'static str,
    pub error: String,
    pub commit: String,
    pub listed: Option<DateTimeAsMicroseconds>,
    pub entries: Arc<Vec<MirrorEntry>>,
    /// How many files the repository holds that the listing deliberately does not — over the
    /// single-document size limit, or at a path this product will not name. Counted rather than listed:
    /// it is the difference between "that file is not there" and "that file is not there FOR US", and
    /// one number answers it.
    pub skipped_amount: usize,
    /// How many listings have FINISHED for this connection since the service started, whatever they
    /// finished as.
    ///
    /// **It exists so that a refresh somebody pressed can be watched to its end.** Every other field
    /// describes what the mirror holds, and none of them can say whether a particular run is over: a
    /// pull that finds nothing changed leaves every one of them exactly as it was, and a second failure
    /// looks precisely like the first. A number that only goes up says it in one comparison.
    pub pull_no: u64,
}

impl Mirror {
    fn empty(state: &'static str) -> Self {
        Self {
            state,
            error: String::new(),
            commit: String::new(),
            listed: None,
            entries: Arc::new(Vec::new()),
            skipped_amount: 0,
            pull_no: 0,
        }
    }

    pub fn entry(&self, path: &str) -> Option<&MirrorEntry> {
        self.entries.iter().find(|itm| itm.path == path)
    }
}

/// Every connection's file list, plus the keys that fetch behind them.
///
/// **Nothing here survives a restart, deliberately.** The connections do — they are rows on a project —
/// but a key is held only in this process's memory, and the listings are rebuilt by the first tick of
/// the timer. That first tick is cheap precisely because nothing is downloaded: it is one small request
/// per connection.
pub struct GithubMirrors {
    // Tokens, by connection. A `Mutex` rather than an `ArcSwap` because this is written as often as it
    // is read and is never read on a hot path — and because a key must not be cloned into a snapshot
    // that outlives the moment it is used.
    keys: Mutex<AHashMap<String, String>>,
    mirrors: ArcSwap<AHashMap<String, Arc<Mirror>>>,
    // Which connections have a listing running. Two at once would be wasted requests against an hourly
    // budget, and the loser's answer would overwrite the winner's for no gain.
    listing: Mutex<ahash::AHashSet<String>>,
}

/// Held for the length of one listing, and the reason a second one does not start.
///
/// A guard rather than a flag on the mirror because a listing that panics still has to release it, and a
/// `Drop` is the only release that cannot be forgotten.
pub struct PullGuard<'s> {
    mirrors: &'s GithubMirrors,
    key: String,
}

impl Drop for PullGuard<'_> {
    /// Release the claim, and count the run.
    ///
    /// **Counted here rather than at the three places a pull can end** — a fresh listing, a commit that
    /// had not moved, a failure — for the same reason the guard exists at all: one of them is easy to
    /// forget and a panic goes through none of them, and an uncounted run is a dialog that watches for
    /// an end that never arrives. It runs after whatever the pull last wrote, so what it bumps is the
    /// number on the mirror that pull produced.
    fn drop(&mut self) {
        self.mirrors.listing.lock().remove(&self.key);
        self.mirrors.count_finished_pull(&self.key);
    }
}

impl Default for GithubMirrors {
    fn default() -> Self {
        Self::new()
    }
}

impl GithubMirrors {
    pub fn new() -> Self {
        Self {
            keys: Mutex::new(AHashMap::new()),
            mirrors: ArcSwap::from_pointee(AHashMap::new()),
            listing: Mutex::new(ahash::AHashSet::new()),
        }
    }

    /// Claim a connection for one listing, or `None` when one is already running.
    ///
    /// `None` is not an error anywhere it is checked: the timer skipping a connection that a person just
    /// asked to refresh is the timer doing the right thing.
    pub fn try_begin_pull(&self, project_id: &str, name: &str) -> Option<PullGuard<'_>> {
        let key = key_of(project_id, name);

        if !self.listing.lock().insert(key.clone()) {
            return None;
        }

        Some(PullGuard { mirrors: self, key })
    }

    pub fn get(&self, project_id: &str, name: &str) -> Option<Arc<Mirror>> {
        self.mirrors.load().get(&key_of(project_id, name)).cloned()
    }

    /// The mirror for a connection, or an empty one saying nothing has been listed yet.
    ///
    /// A caller drawing a tree wants a mirror either way — "connected, nothing yet" is a state to draw,
    /// not an absence to branch on.
    pub fn get_or_pending(&self, project_id: &str, name: &str) -> Arc<Mirror> {
        match self.get(project_id, name) {
            Some(mirror) => mirror,
            None => Arc::new(Mirror::empty(
                task_manager_shared::github::GithubMirrorState::PENDING,
            )),
        }
    }

    pub fn put(&self, project_id: &str, name: &str, mirror: Mirror) {
        self.update(|map| {
            map.insert(key_of(project_id, name), Arc::new(mirror));
        });
    }

    /// Record what a listing is doing without touching what it has already delivered.
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

    /// Forget a connection entirely — its listing, and its key.
    ///
    /// Both, because this is called when somebody deletes the connection: leaving the key behind would
    /// mean a connection re-created under the same name silently inherits a credential nobody re-entered.
    pub fn forget(&self, project_id: &str, name: &str) {
        self.update(|map| {
            map.remove(&key_of(project_id, name));
        });

        self.keys.lock().remove(&key_of(project_id, name));
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

    /// Add one to a connection's finished-listings count. See [`PullGuard::drop`], which is its only
    /// caller.
    ///
    /// A connection detached while its listing was running has nothing to count, and nothing is put back
    /// for it: the row is gone, and re-creating an entry here would resurrect a mirror nobody owns.
    fn count_finished_pull(&self, key: &str) {
        self.update(|map| {
            let Some(existing) = map.get(key) else {
                return;
            };

            let mut mirror = (**existing).clone();
            mirror.pull_no += 1;

            map.insert(key.to_string(), Arc::new(mirror));
        });
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

/// Turn a repository's file list into the connection's, rooted at the folder that was connected.
///
/// Pure, and separate from everything that talks to GitHub, because this is where the rules that matter
/// live: what is in scope, what is a path this product will name, and what is too big to ever be read.
///
/// **A file over the single-document limit is dropped rather than listed.** Listing it would put a row
/// in the tree that every read and every sync refuses — the honest thing is for it not to be there, with
/// the count saying that something was left out.
pub fn entries_from_tree(tree: Vec<TreeEntry>, repo_path: &str) -> (Vec<MirrorEntry>, usize) {
    let mut entries: Vec<MirrorEntry> = Vec::new();
    let mut skipped: usize = 0;

    for item in tree {
        let Some(relative) = strip_root(&item.path, repo_path) else {
            // Outside the folder that was connected. Not a skip — it was never asked for.
            continue;
        };

        // The same rule every document path in this product goes through, which is also the sanitiser.
        let Ok(relative) = normalise_document_path(relative) else {
            skipped += 1;
            continue;
        };

        if entries.len() >= MAX_MIRROR_FILES || item.size > MAX_BINARY_LEN as i64 {
            skipped += 1;
            continue;
        }

        let content_type = crate::scripts::content_type_of(None, &relative);

        entries.push(MirrorEntry {
            size: item.size,
            sha: item.sha,
            is_binary: !crate::documents::is_text_content_type(&content_type),
            content_type,
            path: relative,
        });
    }

    // Sorted by path, exactly as the documents index is, so everything under one folder is contiguous
    // and a tree can be built by walking the list once.
    entries.sort_by(|left, right| left.path.cmp(&right.path));

    (entries, skipped)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn tree_entry(path: &str, size: i64) -> TreeEntry {
        TreeEntry {
            path: path.to_string(),
            size,
            sha: format!("sha-{path}"),
        }
    }

    #[test]
    fn the_connected_folder_becomes_the_root() {
        assert_eq!(strip_root("docs/design/a.md", "docs"), Some("design/a.md"));
        assert_eq!(strip_root("docs/a.md", ""), Some("docs/a.md"));

        // A sibling that merely starts with the same letters is not inside it.
        assert_eq!(strip_root("docs2/a.md", "docs"), None);
        assert_eq!(strip_root("src/main.rs", "docs"), None);
    }

    #[test]
    fn a_listing_keeps_what_is_in_scope_and_counts_what_it_will_not_name() {
        let (entries, skipped) = entries_from_tree(
            vec![
                tree_entry("docs/readme.md", 10),
                tree_entry("docs/img/logo.png", 20),
                tree_entry("src/main.rs", 30),
            ],
            "docs",
        );

        // `src/` was never in scope, so it is neither an entry nor a skip.
        assert_eq!(skipped, 0);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].path, "img/logo.png");
        assert!(entries[0].is_binary);
        assert_eq!(entries[1].path, "readme.md");
        assert!(!entries[1].is_binary);
        assert_eq!(entries[1].content_type, "text/markdown");
        // The reference, which is what a read is made with.
        assert_eq!(entries[1].sha, "sha-docs/readme.md");
    }

    /// A file nothing could ever read is not put in the tree — it would be a row that every read and
    /// every sync refuses. The count is what says something was left out.
    #[test]
    fn a_file_too_big_to_read_is_left_out_and_counted() {
        let (entries, skipped) = entries_from_tree(
            vec![
                tree_entry("big.bin", MAX_BINARY_LEN as i64 + 1),
                tree_entry("fine.md", 4),
            ],
            "",
        );

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, "fine.md");
        assert_eq!(skipped, 1);
    }

    /// The number a screen watching a refresh compares against: it moves once, when the run ENDS, and it
    /// moves whatever the run did.
    #[test]
    fn a_listing_is_counted_when_it_ends_and_not_before() {
        let mirrors = GithubMirrors::new();

        assert_eq!(mirrors.get_or_pending("P", "specs").pull_no, 0);

        {
            let _guard = mirrors
                .try_begin_pull("P", "specs")
                .expect("nothing running");

            // What a pull does first, and what a failing one does last — neither is the end of the run.
            mirrors.set_state(
                "P",
                "specs",
                task_manager_shared::github::GithubMirrorState::PULLING,
                String::new(),
            );

            assert!(
                mirrors.try_begin_pull("P", "specs").is_none(),
                "a second listing must not start while one is claimed"
            );

            assert_eq!(
                mirrors.get_or_pending("P", "specs").pull_no,
                0,
                "still running — nothing to report yet"
            );
        }

        assert_eq!(mirrors.get_or_pending("P", "specs").pull_no, 1);

        // And the claim is free again, which is what makes a second refresh possible at all.
        let _second = mirrors.try_begin_pull("P", "specs").expect("released");
    }

    /// A connection detached mid-listing is not re-created by the run ending.
    #[test]
    fn a_listing_that_outlived_its_connection_counts_nothing() {
        let mirrors = GithubMirrors::new();

        {
            let _guard = mirrors
                .try_begin_pull("P", "specs")
                .expect("nothing running");
            mirrors.forget("P", "specs");
        }

        assert!(mirrors.get("P", "specs").is_none());
    }

    /// A path this product will not name is counted rather than mangled into one it would.
    #[test]
    fn a_path_that_is_not_a_document_path_is_counted() {
        let (entries, skipped) = entries_from_tree(vec![tree_entry("../escape.md", 4)], "");

        assert!(entries.is_empty());
        assert_eq!(skipped, 1);
    }
}
