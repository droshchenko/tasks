use std::sync::Arc;
use std::time::Duration;

use rust_extensions::date_time::DateTimeAsMicroseconds;
use task_manager_shared::github::GithubMirrorState;

use crate::app::AppContext;
use crate::board::GithubConnectionModel;

use super::client::{GithubError, RepoRef};
use super::mirror::{Mirror, entries_from_tree};

/// How often every connection's file list is refreshed.
///
/// Ten minutes is the number this was asked for, and it is the right shape of number: the check itself
/// is one small request per connection — the file list is only re-read when the head commit has moved —
/// so the cost of being wrong on the low side is a few hundred bytes, and the cost of being wrong on the
/// high side is an agent reading a specification that changed an hour ago.
pub const PULL_INTERVAL: Duration = Duration::from_secs(10 * 60);

/// Keep every connection's file list fresh, for as long as the process runs.
///
/// **Started after the board is loaded and never awaited.** The first pass runs immediately rather than
/// after the first interval: a restart comes up with every listing empty, and waiting ten minutes to
/// rebuild the ones that need no key would make every deploy look like the feature broke.
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
/// Sequential on purpose. GitHub's budget is hourly and shared across every connection this service
/// has, so there is nothing to win by asking for several listings at once — and a pass over unchanged
/// repositories is one small request each, which is the whole point of checking the commit first.
pub async fn pull_every_connection(app: &Arc<AppContext>) {
    // The list is taken as a snapshot and the lock released: a listing is a network call, and holding
    // the board's guard across one would stall every reader of it.
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

/// Bring one connection's file list up to date.
///
/// **Two requests at most, and usually one.** The head commit is asked for first and the tree is only
/// re-read when it differs from what the mirror holds — which is what makes a ten-minute poll over a
/// repository nobody is touching cost a few hundred bytes. Neither request carries any file content:
/// what this produces is a list of paths, sizes and blob shas.
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

    let repo_ref = RepoRef {
        owner: &connection.owner,
        repo: &connection.repo,
        branch: &connection.branch,
    };

    let commit = match super::client::resolve_head_commit(&repo_ref, key.as_deref()).await {
        Ok(commit) => commit,
        Err(err) => {
            record_failure(app, project_id, &connection.name, &previous, err);
            return;
        }
    };

    // Nothing has moved and something is already listed. The mirror keeps everything it had and only its
    // freshness changes — which is what makes the common case of this whole feature nearly free.
    if commit == previous.commit && !previous.entries.is_empty() {
        let mut mirror = (*previous).clone();
        mirror.state = GithubMirrorState::READY;
        mirror.error = String::new();
        mirror.listed = Some(DateTimeAsMicroseconds::now());

        app.github.put(project_id, &connection.name, mirror);
        return;
    }

    let tree = match super::client::list_tree(&repo_ref, &commit, key.as_deref()).await {
        Ok(tree) => tree,
        Err(err) => {
            record_failure(app, project_id, &connection.name, &previous, err);
            return;
        }
    };

    let truncated = tree.truncated;
    let (entries, skipped_amount) = entries_from_tree(tree.entries, &connection.repo_path);

    app.github.put(
        project_id,
        &connection.name,
        Mirror {
            state: GithubMirrorState::READY,
            error: if truncated {
                "GitHub cut this repository's file list off — it is too large to list in one answer, so some files are not shown. Connect a folder inside it rather than the whole thing".to_string()
            } else {
                String::new()
            },
            commit,
            listed: Some(DateTimeAsMicroseconds::now()),
            entries: Arc::new(entries),
            skipped_amount,
        },
    );
}

/// Record why a listing did not happen, WITHOUT touching what the last good one left behind.
///
/// Nothing was ever downloaded, so "what the last good one left" is a list of references — and those
/// stay usable the moment GitHub answers again. A missing key is its own state rather than a failure,
/// because the two send a reader to different places: one is a person typing something, the other is
/// somebody looking at a repository.
fn record_failure(
    app: &Arc<AppContext>,
    project_id: &str,
    name: &str,
    previous: &Mirror,
    err: GithubError,
) {
    let state = if err.needs_key() {
        GithubMirrorState::NEEDS_KEY
    } else {
        GithubMirrorState::FAILED
    };

    // Said once per failure rather than swallowed: a connection that has been failing for a day is
    // something a log should be able to show, and the screen only shows the latest. The message is
    // GitHub's own and the key is never part of it.
    println!("github: {name} of project {project_id} — {}", err.message());

    let mut mirror = previous.clone();
    mirror.state = state;
    mirror.error = err.message().to_string();

    app.github.put(project_id, name, mirror);
}

/// Fetch one mirrored file's bytes from GitHub, now.
///
/// **The only place content enters this feature, and it holds it for exactly as long as the caller
/// does.** Named by blob sha rather than by path, so what comes back is the bytes the listing described
/// rather than whatever is at that path this second.
pub async fn read_mirror_file(
    app: &AppContext,
    project_id: &str,
    connection: &GithubConnectionModel,
    sha: &str,
) -> Result<Vec<u8>, String> {
    let key = app.github.key(project_id, &connection.name);

    let repo_ref = RepoRef {
        owner: &connection.owner,
        repo: &connection.repo,
        branch: &connection.branch,
    };

    super::client::read_blob(&repo_ref, sha, key.as_deref())
        .await
        .map_err(|err| err.message().to_string())
}
