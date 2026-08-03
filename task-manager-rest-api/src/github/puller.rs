use std::sync::Arc;
use std::time::Duration;

use rust_extensions::date_time::DateTimeAsMicroseconds;
use task_manager_shared::github::GithubMirrorState;

use crate::app::AppContext;
use crate::board::GithubConnectionModel;

use super::client::{GithubError, RepoRef};
use super::mirror::Mirror;

/// How often every connection is checked.
///
/// Ten minutes is the number this was asked for, and it is the right shape of number: the check itself
/// is one small request per connection — the archive is only downloaded when the head commit has moved
/// — so the cost of being wrong on the low side is a few hundred bytes, and the cost of being wrong on
/// the high side is an agent reading a specification that changed an hour ago.
pub const PULL_INTERVAL: Duration = Duration::from_secs(10 * 60);

/// Keep every connection's mirror fresh, for as long as the process runs.
///
/// **Started after the board is loaded and never awaited.** The first pass runs immediately rather than
/// after the first interval: a restart comes up with every mirror empty, and waiting ten minutes to fill
/// the ones that need no key would make every deploy look like the feature broke.
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
/// Sequential on purpose. The work is bounded by how many repositories a team attached, the whole point
/// of the head-commit check is that a pass over unchanged repositories is nearly free, and running them
/// together would mean several archive downloads sharing the memory of one process for no gain in a
/// ten-minute budget.
pub async fn pull_every_connection(app: &Arc<AppContext>) {
    // The list is taken as a snapshot and the lock released: a pull is a network call, and the board's
    // guard is `!Send` — which is the compiler enforcing what we want anyway.
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

/// Bring one connection's mirror up to date.
///
/// **Two requests at most, and usually one.** The head commit is asked for first and the archive is only
/// downloaded when it differs from what the mirror holds — which is what makes a ten-minute poll over a
/// repository nobody is touching cost a few hundred bytes rather than a repository.
///
/// Nothing here returns an error, because there is nobody to return one to: a pull is something the
/// service does on a timer, and what a failure produces is a state on the mirror that a screen shows and
/// the next pass retries.
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

    // Nothing has moved and something is already on disk. The mirror keeps everything it had and only
    // its freshness changes — which is what makes the common case of this whole feature nearly free.
    if commit == previous.commit && !previous.entries.is_empty() {
        let mut mirror = (*previous).clone();
        mirror.state = GithubMirrorState::READY;
        mirror.error = String::new();
        mirror.pulled = Some(DateTimeAsMicroseconds::now());

        app.github.put(project_id, &connection.name, mirror);
        return;
    }

    let archive = match super::client::download_archive(&repo_ref, key.as_deref()).await {
        Ok(archive) => archive,
        Err(err) => {
            record_failure(app, project_id, &connection.name, &previous, err);
            return;
        }
    };

    let dir = app.github.dir_for(project_id, &connection.name);
    let repo_path = connection.repo_path.clone();

    // Unzipping and writing a few thousand files is blocking work, and doing it on a runtime thread would
    // stall every request that thread was also serving.
    let installed = tokio::task::spawn_blocking(move || {
        super::mirror::install_archive(&dir, &archive, &repo_path)
    })
    .await;

    let installed = match installed {
        Ok(installed) => installed,
        Err(err) => Err(format!("unpacking the download did not finish: {err}")),
    };

    match installed {
        Ok((entries, skipped_amount)) => {
            app.github.put(
                project_id,
                &connection.name,
                Mirror {
                    state: GithubMirrorState::READY,
                    error: String::new(),
                    commit,
                    pulled: Some(DateTimeAsMicroseconds::now()),
                    dir: app.github.dir_for(project_id, &connection.name),
                    entries: Arc::new(entries),
                    skipped_amount,
                },
            );
        }
        Err(err) => {
            // **The one failure that DOES empty a mirror, and it has to.** Everything else that can go
            // wrong — the network, the key, a rate limit — happens before a byte on disk is touched, so
            // the previous tree is still there and is kept. An install failure is different: the swap
            // removes the old directory before it renames the new one into place, so by the time this is
            // reached the files are genuinely gone.
            //
            // Keeping the entry list would be worse than useless. It would report a tree that cannot be
            // read, and — because the commit would still match — the next tick would take the unchanged
            // fast path and call it READY for ever. Clearing both is what makes the next pull download
            // again.
            app.github.put(
                project_id,
                &connection.name,
                Mirror {
                    state: GithubMirrorState::FAILED,
                    error: err,
                    commit: String::new(),
                    pulled: previous.pulled,
                    dir: app.github.dir_for(project_id, &connection.name),
                    entries: Arc::new(Vec::new()),
                    skipped_amount: 0,
                },
            );
        }
    }
}

/// Record why a pull did not happen, WITHOUT touching what the last good one left behind.
///
/// A missing key is its own state rather than a failure, because the two send a reader to different
/// places: one is a person typing something, the other is somebody looking at a repository.
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
    // something a log should be able to show, and the screen only shows the latest.
    println!(
        "github: {} of project {project_id} — {}",
        name,
        err.message()
    );

    let mut mirror = previous.clone();
    mirror.state = state;
    mirror.error = err.message().to_string();

    app.github.put(project_id, name, mirror);
}
