use super::{Snapshot, UploadedGraph, WikiDocument, config, graph, settings_view, source_hash};
use crate::{app::AppContext, postgres::AppSettingDto};
use std::{path::Path, sync::Arc};
use task_manager_shared::ai_settings::*;

const MAX_WIKI_FILES: usize = 3000;
const MAX_FILE_BYTES: usize = 128 * 1024;
const MAX_WIKI_BYTES: usize = 24 * 1024 * 1024;

fn checked_root(clone: &Path, repo_path: &str) -> Result<std::path::PathBuf, String> {
    let clone = clone
        .canonicalize()
        .map_err(|_| "The connected repository is not available.")?;
    let root = crate::github::workdir::connection_root(&clone, repo_path)
        .canonicalize()
        .map_err(|_| "The configured repository folder is not available.")?;
    if !root.starts_with(&clone) {
        return Err("The connected folder points outside its repository.".into());
    }
    Ok(root)
}

async fn git_value(clone: &Path, args: &[&str]) -> Result<String, String> {
    let output = tokio::process::Command::new("git")
        .arg("-C")
        .arg(clone)
        .args(args)
        .output()
        .await
        .map_err(|_| "The connected checkout could not be inspected.")?;
    if !output.status.success() {
        return Err(
            "The connected checkout could not be inspected. Refresh the repository connection."
                .into(),
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub fn source_matches(path: &str, roots: &[String]) -> bool {
    roots.iter().any(|root| {
        path == root
            || path
                .strip_prefix(root)
                .is_some_and(|suffix| suffix.starts_with('/'))
    })
}

fn safe_file(root: &Path, relative: &str, max_bytes: usize) -> Result<Vec<u8>, String> {
    let relative = super::relative_path(relative)?;
    let root = root
        .canonicalize()
        .map_err(|_| "Connected repository is not available on this server.")?;
    let file = root
        .join(relative)
        .canonicalize()
        .map_err(|_| "The source file was not found in the connected repository.")?;
    if !file.starts_with(&root) {
        return Err("A source symlink points outside the connected repository.".into());
    }
    let metadata =
        std::fs::metadata(&file).map_err(|_| "The source file could not be inspected.")?;
    if !metadata.is_file() || metadata.len() > max_bytes as u64 {
        return Err("The source is not a regular file within the configured size limit.".into());
    }
    let bytes = std::fs::read(file).map_err(|_| "The source file could not be read.")?;
    if bytes.len() > max_bytes {
        return Err("The source grew beyond the configured size limit.".into());
    }
    Ok(bytes)
}

fn collect_wiki(root: &Path, files: Vec<String>) -> (Vec<WikiDocument>, String) {
    let mut documents = Vec::new();
    let mut skipped = files.len().saturating_sub(MAX_WIKI_FILES);
    let mut bytes_total = 0;
    for path in files.into_iter().take(MAX_WIKI_FILES) {
        let Ok(bytes) = safe_file(root, &path, MAX_FILE_BYTES) else {
            skipped += 1;
            continue;
        };
        if bytes_total + bytes.len() > MAX_WIKI_BYTES {
            skipped += 1;
            continue;
        }
        let content_hash = crate::documents::content_hash(&bytes);
        let Ok(text) = String::from_utf8(bytes) else {
            skipped += 1;
            continue;
        };
        let title = text
            .lines()
            .find_map(|line| line.strip_prefix("# "))
            .unwrap_or(&path)
            .trim();
        let title = super::super::semantic::clip(title, 512);
        bytes_total += text.len();
        documents.push(WikiDocument {
            path,
            title,
            text,
            content_hash,
        });
    }
    let mut notices = Vec::new();
    if documents.is_empty() {
        notices.push("No readable wiki documents matched the configured paths.".to_string());
    }
    if skipped > 0 {
        notices.push(format!("Skipped {skipped} files: unavailable, non-UTF8 or beyond the 128 KiB/file, 3,000-file or 24 MiB index limits."));
    }
    (documents, notices.join(" "))
}

pub async fn refresh(app: &AppContext, prefix: &str) -> Result<KnowledgeSettingsResponse, String> {
    let _refresh = app
        .knowledge
        .refreshes
        .try_lock()
        .map_err(|_| "Knowledge refresh is already running. Retry after it finishes.")?;
    let project = crate::scripts::resolve_project_by_prefix(&app.board.read(), prefix)?;
    let (config, revision) = config(app, &project.id)?;
    if !config.enabled {
        return Err("Enable project knowledge and save its settings before refreshing.".into());
    }
    let connection = project
        .github_connection(&config.connection)
        .ok_or("The configured repository connection no longer exists.")?
        .clone();
    let repository = format!("{}/{}", connection.owner, connection.repo);
    let clone_dir =
        crate::github::workdir::connection_dir(&app.git_repos_path, &project.id, &connection.name);
    let lock = app.github.workdir_lock(&project.id, &connection.name);
    let _read = lock.read().await;
    let root = checked_root(&clone_dir, &connection.repo_path)?;
    let mirror = app.github.get_or_pending(&project.id, &connection.name);
    if mirror.listed.is_none() {
        return Err("The repository has not been indexed yet. Refresh its connection in Projects setup first.".into());
    }
    let output = tokio::process::Command::new("git")
        .arg("-C")
        .arg(&clone_dir)
        .args(["rev-parse", "HEAD"])
        .output()
        .await
        .map_err(|_| "The repository commit could not be read.")?;
    let commit = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !output.status.success() || !graph::valid_commit(&commit) {
        return Err(
            "The repository needs a valid source commit before knowledge can be indexed.".into(),
        );
    }
    if !graph::same_commit(&commit, &mirror.commit) {
        return Err("The repository listing is stale. Refresh its connection first.".into());
    }
    let remote = git_value(&clone_dir, &["config", "--get", "remote.origin.url"]).await?;
    let expected_remote = crate::github::workdir::repo_url(&connection);
    if !remote
        .trim_end_matches(".git")
        .eq_ignore_ascii_case(expected_remote.trim_end_matches(".git"))
    {
        return Err("The checkout does not match the configured repository. Refresh its connection before indexing.".into());
    }
    if !connection.branch.is_empty() {
        let branch = git_value(&clone_dir, &["symbolic-ref", "--quiet", "--short", "HEAD"])
            .await
            .unwrap_or_default();
        if branch != connection.branch {
            let tag = format!("refs/tags/{}^{{commit}}", connection.branch);
            let tagged = if branch.is_empty() {
                git_value(
                    &clone_dir,
                    &["rev-parse", "--verify", "--end-of-options", &tag],
                )
                .await
                .ok()
            } else {
                None
            };
            if tagged.as_deref() != Some(commit.as_str()) {
                return Err("The checkout is not on the configured branch or tag. Refresh its connection before indexing.".into());
            }
        }
    }
    let mut files: Vec<_> = mirror
        .entries
        .iter()
        .filter(|entry| !entry.is_binary && source_matches(&entry.path, &config.wiki_paths))
        .filter(|entry| {
            matches!(
                Path::new(&entry.path)
                    .extension()
                    .and_then(|s| s.to_str())
                    .map(str::to_ascii_lowercase)
                    .as_deref(),
                Some("md" | "markdown" | "txt")
            )
        })
        .map(|entry| entry.path.clone())
        .collect();
    files.sort();
    let uploaded = app.knowledge.uploads.read().get(&project.id).cloned();
    let graph_upload_hash = if config.graph_source == "upload" {
        uploaded
            .as_ref()
            .map_or_else(String::new, |g| g.content_hash.clone())
    } else {
        String::new()
    };
    let graph_source = config.graph_source.clone();
    let graph_path = config.graph_path.clone();
    let graph_connection = connection.name.clone();
    let graph_repository = repository.clone();
    let has_wiki = !config.wiki_paths.is_empty();
    let (documents, graph, notice) = tokio::task::spawn_blocking(move || {
        let (documents, wiki_notice) = if has_wiki {
            collect_wiki(&root, files)
        } else {
            (Vec::new(), String::new())
        };
        let graph = match graph_source.as_str() {
            "repository" => safe_file(&root, &graph_path, graph::MAX_GRAPH_BYTES)
                .and_then(|bytes| graph::parse(&bytes))
                .map(Some),
            "upload" => uploaded
                .filter(|upload| {
                    upload.connection == graph_connection && upload.repository == graph_repository
                })
                .map(|upload| Some(upload.graph.clone()))
                .ok_or_else(|| "Upload a Graphify export for this repository.".to_string()),
            _ => Ok(None),
        };
        let (graph, graph_notice) = match graph {
            Ok(graph) => (graph, String::new()),
            Err(error) => (None, error),
        };
        (
            documents,
            graph,
            [wiki_notice, graph_notice]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(" "),
        )
    })
    .await
    .map_err(|_| "Knowledge indexing was interrupted.")?;
    let source_fingerprint = source_hash(&config, &connection);
    let snapshot = Snapshot {
        project_id: project.id.clone(),
        connection: connection.name,
        repository,
        repo_path: connection.repo_path,
        config_hash: source_fingerprint,
        commit,
        documents,
        graph,
        graph_upload_hash,
        updated: rust_extensions::date_time::DateTimeAsMicroseconds::now().unix_microseconds
            / 1_000_000,
        notice,
    };
    let _mutation = app.configuration.mutations.lock().await;
    if app
        .configuration
        .revision(&format!("knowledge:{}", project.id))
        != revision
    {
        return Err("Knowledge settings changed while sources were read. Refresh again.".into());
    }
    let row = AppSettingDto {
        id: format!("knowledge-snapshot:{}", project.id),
        value: serde_json::to_string(&snapshot)
            .map_err(|_| "Knowledge index could not be encoded.")?,
        revision: snapshot.updated,
        updated_by: "knowledge-indexer".into(),
    };
    app.settings_repo.upsert(&row).await?;
    app.knowledge
        .snapshots
        .write()
        .insert(project.id.clone(), Arc::new(snapshot));
    app.knowledge.errors.write().remove(&project.id);
    drop(_mutation);
    drop(_read);
    settings_view(app, &project.prefix)
}

pub async fn upload_graph(
    app: &AppContext,
    input: UploadGraphInput,
    who: &str,
) -> Result<KnowledgeSettingsResponse, String> {
    let project = crate::scripts::resolve_project_by_prefix(&app.board.read(), &input.project)?;
    let (config, revision) = config(app, &project.id)?;
    if revision != input.revision {
        return Err("Knowledge settings changed. Reload before uploading the graph.".into());
    }
    if config.graph_source != "upload" {
        return Err("Select Uploaded export and save before uploading Graphify JSON.".into());
    }
    let connection = project
        .github_connection(&config.connection)
        .ok_or("Choose an existing repository connection.")?
        .clone();
    let content_hash = crate::documents::content_hash(&input.content);
    let graph = tokio::task::spawn_blocking(move || graph::parse(&input.content))
        .await
        .map_err(|_| "Graph validation was interrupted.")??;
    let upload = UploadedGraph {
        project_id: project.id.clone(),
        connection: connection.name,
        repository: format!("{}/{}", connection.owner, connection.repo),
        content_hash,
        graph,
    };
    let _guard = app.configuration.mutations.lock().await;
    if app
        .configuration
        .revision(&format!("knowledge:{}", project.id))
        != revision
    {
        return Err(
            "Knowledge settings changed during upload. Upload again after reloading.".into(),
        );
    }
    let row = AppSettingDto {
        id: format!("graph-upload:{}", project.id),
        value: serde_json::to_string(&upload).map_err(|_| "Graph could not be encoded.")?,
        revision,
        updated_by: who.into(),
    };
    app.settings_repo.upsert(&row).await?;
    app.knowledge
        .uploads
        .write()
        .insert(project.id.clone(), Arc::new(upload));
    // Retire a prior graph immediately; a failed subsequent wiki refresh must not reuse it.
    app.knowledge.snapshots.write().remove(&project.id);
    drop(_guard);
    match refresh(app, &project.prefix).await {
        Ok(view) => Ok(view),
        Err(error) => {
            app.knowledge.errors.write().insert(
                project.id.clone(),
                format!("Graph saved. Knowledge refresh needs attention: {error}"),
            );
            settings_view(app, &project.prefix)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roots_match_path_segments_and_bounded_sources_are_read_as_text() {
        let roots = vec!["docs/wiki".to_string()];
        assert!(source_matches("docs/wiki/start.md", &roots));
        assert!(!source_matches("docs/wiki-old/start.md", &roots));
        let root = std::env::temp_dir().join(format!("tasks-knowledge-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("docs/wiki")).unwrap();
        std::fs::write(
            root.join("docs/wiki/start.md"),
            "# Start\nUseful project evidence",
        )
        .unwrap();
        let (docs, notice) = collect_wiki(&root, vec!["docs/wiki/start.md".into()]);
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].title, "Start");
        assert!(notice.is_empty());
        assert!(safe_file(&root, "../outside", 128).is_err());
        #[cfg(unix)]
        {
            let outside = root.with_extension("outside");
            std::fs::write(&outside, "private").unwrap();
            std::os::unix::fs::symlink(&outside, root.join("escape.md")).unwrap();
            assert!(safe_file(&root, "escape.md", 128).is_err());
            std::fs::remove_file(outside).unwrap();
            let outside_dir = root.with_extension("outside-directory");
            std::fs::create_dir(&outside_dir).unwrap();
            std::os::unix::fs::symlink(&outside_dir, root.join("escape-root")).unwrap();
            assert!(checked_root(&root, "escape-root").is_err());
            std::fs::remove_dir(outside_dir).unwrap();
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}
