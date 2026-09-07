use std::sync::Arc;

use task_manager_shared::github::{
    GithubConnectionResponse, GithubMirrorState, MAX_CONNECTIONS_PER_PROJECT,
    normalise_connection_name,
};

use crate::app::AppContext;
use crate::board::GithubConnectionModel;
use crate::github::{PullMode, normalise_branch, normalise_repo_path, parse_repo_url};

use super::resolve_project_by_prefix;

/// Attach a repository to a project, or change one that is attached.
///
/// **The key is handled separately from everything else here, and never touches the row.** It goes into
/// the process's memory and the connection goes into Postgres, which is why `key: None` means "leave
/// whatever key is being held" rather than "clear it": editing the branch of a connection must not
/// silently sign it out. An empty string is how you say forget it.
///
/// The url is parsed rather than demanded in pieces — a `/tree/<branch>/<folder>` address, which is what
/// the browser's address bar gives you after clicking into a folder, fills the branch and the folder in
/// by itself. Anything given explicitly wins over what the url carried, because the person typing into
/// the box is more recent than the thing they pasted.
///
/// A pull is started as soon as this returns, and is not waited for: connecting a repository should feel
/// like connecting it, and the mirror shows `pulling` until it is there.
pub async fn set_github_connection(
    app: &Arc<AppContext>,
    project_prefix: &str,
    name: &str,
    url: &str,
    branch: Option<&str>,
    repo_path: Option<&str>,
    key: Option<&str>,
) -> Result<(), String> {
    let name = normalise_connection_name(name)?;
    let parsed = parse_repo_url(url)?;

    // What the url carried is the default; what was typed beside it wins. `Some("")` is a person
    // deliberately clearing the box, which is why this is not `filter(|itm| !itm.is_empty())`.
    let branch = match branch {
        Some(branch) => normalise_branch(branch)?,
        None => normalise_branch(&parsed.branch)?,
    };

    let repo_path = match repo_path {
        Some(repo_path) => normalise_repo_path(repo_path)?,
        None => normalise_repo_path(&parsed.path)?,
    };

    let connection = GithubConnectionModel {
        name: name.clone(),
        owner: parsed.owner,
        repo: parsed.repo,
        branch,
        repo_path,
    };

    let project_id = {
        let board = app.board.read();
        let project = resolve_project_by_prefix(&board, project_prefix)?;
        let mut project = super::projects::load(&board, &project.id)?;

        match project
            .github_connections
            .iter_mut()
            .find(|itm| itm.name == name)
        {
            Some(existing) => *existing = connection.clone(),
            None => {
                if project.github_connections.len() >= MAX_CONNECTIONS_PER_PROJECT {
                    return Err(format!(
                        "this project already has {MAX_CONNECTIONS_PER_PROJECT} connected repositories, which is the limit"
                    ));
                }

                project.github_connections.push(connection.clone());
            }
        }

        let project_id = project.id.clone();
        super::projects::save(app, project).await;
        project_id
    };

    // After the row is saved, so a key can never be held for a connection that failed to save.
    if let Some(key) = key {
        app.github.set_key(&project_id, &name, key);
    }

    start_pull(app, project_id, connection, PullMode::Fetch);

    Ok(())
}

/// Hand the server a key for a connection that already exists.
///
/// The call somebody makes after a restart, and the reason a private repository is usable at all without
/// a credential in the database. It pulls immediately — the point of typing a key is to see the folder
/// fill up.
pub async fn set_github_key(
    app: &Arc<AppContext>,
    project_prefix: &str,
    name: &str,
    key: &str,
) -> Result<(), String> {
    let (project_id, connection) = {
        let board = app.board.read();
        let project = resolve_project_by_prefix(&board, project_prefix)?;

        let connection = project
            .github_connection(name)
            .ok_or_else(|| format!("this project has no connection called '{name}'"))?
            .clone();

        (project.id.clone(), connection)
    };

    app.github.set_key(&project_id, name, key);

    start_pull(app, project_id, connection, PullMode::Fetch);

    Ok(())
}

/// Detach a repository: the row, then the listing and the key.
///
/// **The clone on the volume is left where it is.** Nothing here removes
/// `<git_repos_path>/<project id>/<name>`: a detach is a change to the board, and deleting a folder on a
/// disk in the same breath is a second thing nobody asked for — one that also takes whatever somebody
/// left in it through `github_git`. Reclaiming the space is a deliberate act on the host.
///
/// **What follows is that the next connection of the same name on the same project ADOPTS that folder**,
/// `origin` included — see [`crate::github::pull_connection`], which on the timer's pass fetches whatever
/// `.git/config` already points at. That used to be permanent; it no longer is. Pressing **Refresh** on
/// the connection deletes the folder and clones it from the url the row now carries, which is what makes
/// detaching and re-connecting under the same name land where somebody expects.
pub async fn delete_github_connection(
    app: &Arc<AppContext>,
    project_prefix: &str,
    name: &str,
) -> Result<(), String> {
    let project_id = {
        let board = app.board.read();
        let project = resolve_project_by_prefix(&board, project_prefix)?;
        let mut project = super::projects::load(&board, &project.id)?;

        let before = project.github_connections.len();
        project.github_connections.retain(|itm| itm.name != name);

        if project.github_connections.len() == before {
            return Err(format!("this project has no connection called '{name}'"));
        }

        let project_id = project.id.clone();
        super::projects::save(app, project).await;
        project_id
    };

    // After the row, so a crash between the two leaves a listing nothing on the board owns any more —
    // inert, and gone with the process — rather than a live connection whose key has already been thrown
    // away and has to be typed in again.
    app.github.forget(&project_id, name);

    Ok(())
}

/// Refresh one connection now: delete its working copy and clone it again.
///
/// **A refresh is not a fetch, and the difference is the point.** The timer fetches every ten minutes and
/// is already keeping the folder current; somebody who presses this is asking for something a fetch does
/// not give them — the folder as GitHub has it, with a force-pushed branch, a file that stopped being
/// tracked, and anything a git command left behind all gone. Nothing on this surface writes into a
/// connected repository, so what is thrown away is a copy.
///
/// Answers with the connection's finished-listings count as it stood the moment the ask was accepted —
/// which is what makes the run watchable, since nothing about it is finished when this returns. See
/// [`crate::github::Mirror::pull_no`].
pub async fn pull_github_connection(
    app: &Arc<AppContext>,
    project_prefix: &str,
    name: &str,
) -> Result<i64, String> {
    let (project_id, connection) = {
        let board = app.board.read();
        let project = resolve_project_by_prefix(&board, project_prefix)?;

        let connection = project
            .github_connection(name)
            .ok_or_else(|| format!("this project has no connection called '{name}'"))?
            .clone();

        (project.id.clone(), connection)
    };

    // Read BEFORE the pull is started, or a listing that finished in between would be handed back as
    // the baseline — and the caller would take somebody else's run for its own and stop watching at
    // once.
    let pull_no = app
        .github
        .get_or_pending(&project_id, &connection.name)
        .pull_no as i64;

    start_pull(app, project_id, connection, PullMode::Reclone);

    Ok(pull_no)
}

/// Every connection of one project, with what its mirror currently holds.
///
/// The configured half comes from the board and the live half from memory, joined here — which is the
/// only place they meet. Neither knows about the other, and that is what lets a restart keep the
/// connections while losing every mirror.
pub fn list_github_connections(
    app: &AppContext,
    project_prefix: &str,
) -> Result<Vec<GithubConnectionResponse>, String> {
    let project = {
        let board = app.board.read();
        resolve_project_by_prefix(&board, project_prefix)?
    };

    let connections = project
        .github_connections
        .iter()
        .map(|connection| {
            let mirror = app.github.get_or_pending(&project.id, &connection.name);
            let has_key = app.github.has_key(&project.id, &connection.name);

            // A connection with no key that has never been pulled is not "pending" in any useful sense —
            // it is waiting for somebody. Said here rather than by the puller, because the puller has not
            // run yet at exactly the moment this matters most: the first render after a restart.
            let state = if mirror.state == GithubMirrorState::PENDING
                && !has_key
                && mirror.entries.is_empty()
            {
                GithubMirrorState::PENDING
            } else {
                mirror.state
            };

            GithubConnectionResponse {
                name: connection.name.clone(),
                repo: connection.full_name(),
                branch: connection.branch.clone(),
                path: connection.repo_path.clone(),
                has_key,
                state: state.to_string(),
                error: mirror.error.clone(),
                // Short form: a full sha is forty characters of noise beside a folder name, and the
                // first seven are what anybody would paste into a `git show`.
                commit: mirror.commit.chars().take(7).collect(),
                pulled_unix_seconds: mirror
                    .listed
                    .map(|itm| itm.unix_microseconds / 1_000_000)
                    .unwrap_or(0),
                files_amount: mirror.entries.len() as i32,
                skipped_amount: mirror.skipped_amount as i32,
                pull_no: mirror.pull_no as i64,
            }
        })
        .collect();

    Ok(connections)
}

/// Start a pull and return without it.
///
/// Every caller here is answering a person who pressed something, and a repository can take half a
/// minute to arrive. The mirror carries the state, so the screen has something true to show the whole
/// time — which a request held open for thirty seconds does not.
fn start_pull(
    app: &Arc<AppContext>,
    project_id: String,
    connection: GithubConnectionModel,
    mode: PullMode,
) {
    let app = app.clone();

    tokio::spawn(async move {
        crate::github::pull_connection(&app, &project_id, &connection, mode).await;
    });
}
