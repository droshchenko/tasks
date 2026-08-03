//! Connected GitHub repositories, mirrored into a project's documents under `github/`.
//!
//! **A connection is configuration; a mirror is a cache.** The four things a person set up — a name, a
//! repository, a branch, a folder — are a row on the project and survive everything. What was actually
//! downloaded lives in a temp directory inside the container, and the key that reached it lives in this
//! process's memory. Neither survives a restart, and neither is meant to.
//!
//! The reason to keep them apart is the key. A token written to Postgres is a token in every backup and
//! every dump of that table, for as long as anybody keeps one; a token held in memory is gone when the
//! process is. The price is that a private repository stops mirroring after a deploy until somebody
//! types its key again, and that price was accepted deliberately — see `GithubMirrors`.
//!
//! What the rest of the service sees of all this is a path: `github/<connection>/<file>`, served by the
//! same `documents_list` and `documents_get` that serve the project's own documents, and refused by
//! every write. Bringing a file in for real is a sync, which copies it into a folder of the project's
//! own where it becomes a document with an id, a version and a history.

mod client;
mod connection;
mod mirror;
mod puller;

// `client` is deliberately not re-exported: nothing outside this module talks to GitHub directly, and
// a `GithubError` escaping into the rest of the service would be an invitation to handle transport
// failures somewhere other than the puller.
pub use connection::*;
pub use mirror::*;
pub use puller::*;

