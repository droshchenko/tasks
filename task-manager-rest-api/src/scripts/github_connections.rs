use std::sync::Arc;

use task_manager_shared::github::{
    GithubConnectionResponse, GithubMirrorState, MAX_CONNECTIONS_PER_PROJECT,
    normalise_connection_name,
};

use crate::app::AppContext;
use crate::board::GithubConnectionModel;
use crate::github::{normalise_branch, normalise_repo_path, parse_repo_url};

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

    start_pull(app, project_id, connection);

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

    start_pull(app, project_id, connection);

    Ok(())
}

/// Detach a repository: the row, the tree on disk, and the key, in that order.
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

    // After the row, so a crash between the two leaves a connection whose mirror rebuilds itself rather
    // than a tree on disk that nothing owns.
    app.github.forget(&project_id, name);

    Ok(())
}

/// Refresh one connection now, rather than at the next tick.
pub async fn pull_github_connection(
    app: &Arc<AppContext>,
    project_prefix: &str,
    name: &str,
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

    start_pull(app, project_id, connection);

    Ok(())
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
fn start_pull(app: &Arc<AppContext>, project_id: String, connection: GithubConnectionModel) {
    let app = app.clone();

    tokio::spawn(async move {
        crate::github::pull_connection(&app, &project_id, &connection).await;
    });
}
