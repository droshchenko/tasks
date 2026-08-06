use std::sync::Arc;
use std::time::Duration;

use rust_extensions::date_time::DateTimeAsMicroseconds;
use task_manager_shared::github::GithubMirrorState;

use crate::app::AppContext;
use crate::board::GithubConnectionModel;

use super::mirror::Mirror;
use super::workdir;

/// How often every connection's working copy is brought up to date.
///
/// Ten minutes is the number this was asked for and it is still the right shape of number, but what it
/// costs has changed: a tick is now a `git fetch`, which over an unchanged repository is one small
/// exchange and no objects. What it buys is that a specification an agent reads is at most ten minutes
/// behind the branch it came from.
pub const PULL_INTERVAL: Duration = Duration::from_secs(10 * 60);

/// Keep every connection's working copy fresh, for as long as the process runs.
///
/// **Started after the board is loaded and never awaited.** The first pass runs immediately rather than
/// after the first interval, and it is the pass that matters most: on a fresh volume it is the one that
/// clones every connection in the first place.
pub fn run_puller(app: Arc<AppContext>) {
    tokio::spawn(async move {
        loop {
            pull_every_connection(&app).await;
            tokio::time::sleep(PULL_INTERVAL).await;
        }
    });
}

/// One pass over every connection of every project.
///
/// Sequential on purpose. These are clones on one disk against one host, and running eight of them at
/// once would turn a background refresh into a spike of network and IO that the requests being served
/// alongside it would feel.
pub async fn pull_every_connection(app: &Arc<AppContext>) {
    // The list is taken as a snapshot and the lock released: a fetch is a network call, and holding the
    // board's guard across one would stall every reader of it.
    let work: Vec<(String, GithubConnectionModel)> = {
        let board = app.board.read();

        board
            .projects()
            .iter()
            .flat_map(|project| {
                project
                    .github_connections
                    .iter()
                    .map(|connection| (project.id.clone(), connection.clone()))
                    .collect::<Vec<_>>()
            })
            .collect()
    };

    for (project_id, connection) in work {
        pull_connection(app, &project_id, &connection).await;
    }
}

/// Bring one connection's working copy up to date, and re-read what is in it.
///
/// **Two halves that fail independently, and that separation is the whole behaviour worth knowing.**
/// The first half talks to GitHub — a clone if there is nothing on disk, a fetch and a safe
/// fast-forward if there is. The second walks the working copy and rebuilds the listing. The second runs
/// whether or not the first worked, because once a clone exists the files are on this disk: GitHub being
/// unreachable, or the key being gone after a restart, stops the exchange with the remote and stops
/// nothing else. The connection reads `needs-key` or `failed` with the reason on it, AND lists every file
/// it has, AND every git command in it keeps working.
///
/// Nothing here returns an error, because there is nobody to return one to: this runs on a timer, and
/// what a failure produces is a state on the mirror that a screen shows and the next pass retries.
pub async fn pull_connection(
    app: &Arc<AppContext>,
    project_id: &str,
    connection: &GithubConnectionModel,
) {
    let Some(_guard) = app.github.try_begin_pull(project_id, &connection.name) else {
        // Already running — the person who pressed Refresh and the timer arriving together.
        return;
    };

    let previous = app.github.get_or_pending(project_id, &connection.name);

    app.github.set_state(
        project_id,
        &connection.name,
        GithubMirrorState::PULLING,
        previous.error.clone(),
    );

    let key = app.github.key(project_id, &connection.name);

    let clone_dir = workdir::connection_dir(&app.git_repos_path, project_id, &connection.name);

    let exchange = if workdir::is_cloned(&clone_dir) {
        workdir::refresh_clone(&clone_dir, connection, key.as_deref()).await
    } else {
        workdir::clone_repository(&clone_dir, connection, key.as_deref()).await
    };

    // Nothing on disk and the clone did not happen: there is no working copy to list, so the failure is
    // the whole of what this connection is right now.
    if !workdir::is_cloned(&clone_dir) {
        let message = exchange
            .err()
            .unwrap_or_else(|| "the repository was not cloned".to_string());

        record_failure(app, project_id, &connection.name, &previous, message);
        return;
    }

    let listing = workdir::list_working_copy(&clone_dir, &connection.repo_path).await;

    let (entries, skipped_amount) = match listing {
        Ok(listing) => listing,
        Err(err) => {
            // The clone is there and could not be read — a disk problem rather than a network one, and
            // the one case where what the mirror held is better than what this pass produced.
            record_failure(app, project_id, &connection.name, &previous, err);
            return;
        }
    };

    let error = exchange.err().unwrap_or_default();

    let state = if error.is_empty() {
        GithubMirrorState::READY
    } else if needs_key(&error) {
        GithubMirrorState::NEEDS_KEY
    } else {
        GithubMirrorState::FAILED
    };

    if !error.is_empty() {
        // Said once per failure rather than swallowed: a connection that has been failing for a day is
        // something a log should be able to show, and the screen only shows the latest. The key is never
        // part of the message — it travels as a header and git does not echo it.
        println!("github: {} of project {project_id} — {error}", connection.name);
    }

    app.github.put(
        project_id,
        &connection.name,
        Mirror {
            state,
            error,
            commit: workdir::head_commit(&clone_dir).await,
            listed: Some(DateTimeAsMicroseconds::now()),
            entries: Arc::new(entries),
            skipped_amount,
            // Carried over rather than bumped: the run is counted once, by the guard's `Drop`, after this
            // has been written. Every other branch out of this function carries it the same way.
            pull_no: previous.pull_no,
        },
    );
}

/// Re-read one connection's working copy WITHOUT touching the network.
///
/// **What a write calls the moment it has changed a file.** The timer is a ten-minute clock and an edit
/// is now: a file created through `documents_upload` that did not appear in `documents_list` until the
/// next tick would read as an upload that silently did nothing. This is also why it does not fetch —
/// there is nothing to ask GitHub, the change is on this disk.
pub async fn relist_connection(
    app: &AppContext,
    project_id: &str,
    connection: &GithubConnectionModel,
) {
    let clone_dir = workdir::connection_dir(&app.git_repos_path, project_id, &connection.name);

    let Ok((entries, skipped_amount)) =
        workdir::list_working_copy(&clone_dir, &connection.repo_path).await
    else {
        return;
    };

    let previous = app.github.get_or_pending(project_id, &connection.name);

    let mut mirror = (*previous).clone();
    mirror.entries = Arc::new(entries);
    mirror.skipped_amount = skipped_amount;
    mirror.commit = workdir::head_commit(&clone_dir).await;
    mirror.listed = Some(DateTimeAsMicroseconds::now());

    app.github.put(project_id, &connection.name, mirror);
}

/// Whether what git said is something a key would fix.
///
/// **Read out of git's own words rather than an exit code**, because git exits 128 for everything from a
/// missing key to a repository that is not there. The distinction earns its keep for the same reason it
/// did against the API: a private repository is indistinguishable from one that does not exist, so
/// "needs a key" when the name is genuinely misspelled is a better wrong answer than "no such
/// repository" when the key simply expired — the first sends somebody to look at both.
///
/// `could not read Username` is the one that matters most in practice: it is what `GIT_TERMINAL_PROMPT=0`
/// produces when a private repository is reached with no credential at all, which is every connection
/// after a restart.
fn needs_key(message: &str) -> bool {
    let message = message.to_lowercase();

    [
        "could not read username",
        "could not read password",
        "authentication failed",
        "terminal prompts disabled",
        "repository not found",
        "permission denied",
        "403 forbidden",
        "invalid username or token",
    ]
    .iter()
    .any(|itm| message.contains(itm))
}

/// Record why a refresh did not happen, WITHOUT touching what the last good one left behind.
///
/// Reached only when there is no working copy to list, or when the disk would not answer — a fetch that
/// failed over an existing clone goes down the ordinary path, because the files are still there and
/// listing them is still the right answer.
fn record_failure(
    app: &Arc<AppContext>,
    project_id: &str,
    name: &str,
    previous: &Mirror,
    message: String,
) {
    let state = if needs_key(&message) {
        GithubMirrorState::NEEDS_KEY
    } else {
        GithubMirrorState::FAILED
    };

    println!("github: {name} of project {project_id} — {message}");

    let mut mirror = previous.clone();
    mirror.state = state;
    mirror.error = message;

    app.github.put(project_id, name, mirror);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The sentence a restart produces on every private connection, and the one that must send somebody
    /// to the key dialog rather than to GitHub to look for a repository that is right there.
    #[test]
    fn what_a_missing_key_sounds_like_is_told_apart_from_what_a_real_failure_does() {
        for message in [
            "fatal: could not read Username for 'https://github.com': terminal prompts disabled",
            "remote: Repository not found.",
            "fatal: Authentication failed for 'https://github.com/o/r.git/'",
        ] {
            assert!(needs_key(message), "{message}");
        }

        for message in [
            "fatal: unable to access 'https://github.com/o/r.git/': Could not resolve host: github.com",
            "error: Your local changes to the following files would be overwritten by merge",
            "fatal: refusing to merge unrelated histories",
        ] {
            assert!(!needs_key(message), "{message}");
        }
    }
}
