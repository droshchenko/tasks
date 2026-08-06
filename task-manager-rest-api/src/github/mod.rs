//! Connected GitHub repositories, cloned onto a volume and shown in a project's documents under
//! `github/`.
//!
//! **A connection is configuration; a clone is a working copy.** The four things a person set up — a
//! name, a repository, a branch, a folder — are a row on the project and survive everything. What was
//! cloned lives on a mounted disk and survives a restart with them. The key that reaches GitHub lives in
//! this process's memory and does not.
//!
//! The reason the key is kept apart is what it is. A token written to Postgres is a token in every backup
//! and every dump of that table, for as long as anybody keeps one; a token held in memory is gone when
//! the process is. The price used to be that a private repository stopped working entirely after a
//! deploy. It no longer is: the clone is still there, so the files still list, still read, still edit and
//! still commit — only fetching and pushing wait for somebody to type the key again.
//!
//! What the rest of the service sees of all this is a path: `github/<connection>/<file>`, served by the
//! same `documents_list` and `documents_get` that serve the project's own documents, and now written by
//! the same `documents_upload`, `documents_edit` and `documents_delete`. A write lands in the working
//! tree, where it is a change git can see; what happens to it after that is a git command somebody runs
//! through `github_git`.

mod connection;
mod mirror;
mod puller;

/// Running git, and the one rule about what may be run. See [`git::parse_git_command`].
pub mod git;
/// The working copy on disk: where it is, how it gets there, and reading and writing files in it.
pub mod workdir;

pub use connection::*;
pub use mirror::*;
pub use puller::*;
