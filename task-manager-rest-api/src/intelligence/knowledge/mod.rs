mod graph;
mod retrieval;
mod sources;

use crate::{app::AppContext, postgres::AppSettingDto};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc};
use task_manager_shared::ai_settings::*;

pub use retrieval::{search, search_project};
pub use sources::{refresh, upload_graph};

#[derive(Clone, Serialize, Deserialize)]
pub struct WikiDocument {
    pub path: String,
    pub title: String,
    pub text: String,
    pub content_hash: String,
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Snapshot {
    pub project_id: String,
    pub connection: String,
    pub repository: String,
    pub repo_path: String,
    pub config_hash: String,
    pub commit: String,
    pub documents: Vec<WikiDocument>,
    pub graph: Option<graph::GraphExport>,
    #[serde(default)]
    pub graph_upload_hash: String,
    pub updated: i64,
    pub notice: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct UploadedGraph {
    pub project_id: String,
    pub connection: String,
    pub repository: String,
    pub content_hash: String,
    pub graph: graph::GraphExport,
}

#[derive(Default)]
pub struct KnowledgeCache {
    pub snapshots: RwLock<HashMap<String, Arc<Snapshot>>>,
    pub uploads: RwLock<HashMap<String, Arc<UploadedGraph>>>,
    pub refreshes: tokio::sync::Mutex<()>,
    pub errors: RwLock<HashMap<String, String>>,
}

impl KnowledgeCache {
    pub fn restore(rows: Vec<AppSettingDto>) -> Self {
        let cache = Self::default();
        for row in rows {
            if let Some(id) = row.id.strip_prefix("knowledge-snapshot:") {
                match serde_json::from_str::<Snapshot>(&row.value) {
                    Ok(snapshot) if snapshot.project_id == id => {
                        cache
                            .snapshots
                            .write()
                            .insert(id.into(), Arc::new(snapshot));
                    }
                    _ => {
                        cache.errors.write().insert(
                            id.into(),
                            "Saved knowledge index is invalid. Refresh sources to rebuild it."
                                .into(),
                        );
                    }
                }
            } else if let Some(id) = row.id.strip_prefix("graph-upload:") {
                match serde_json::from_str::<UploadedGraph>(&row.value) {
                    Ok(graph) if graph.project_id == id => {
                        cache.uploads.write().insert(id.into(), Arc::new(graph));
                    }
                    _ => {
                        cache.errors.write().insert(
                            id.into(),
                            "Saved Graphify export is invalid. Upload it again.".into(),
                        );
                    }
                }
            }
        }
        cache
    }
}

pub fn config(app: &AppContext, project_id: &str) -> Result<(KnowledgeConfig, i64), String> {
    let Some(row) = app.configuration.get(&format!("knowledge:{project_id}")) else {
        return Ok((
            KnowledgeConfig {
                graph_source: "none".into(),
                auto_refresh: true,
                ..Default::default()
            },
            0,
        ));
    };
    serde_json::from_str(&row.value)
        .map(|config| (config, row.revision))
        .map_err(|_| "Project knowledge settings could not be read.".into())
}

pub fn config_hash(config: &KnowledgeConfig) -> String {
    crate::documents::content_hash(
        serde_json::json!({"connection":config.connection,"wiki_paths":config.wiki_paths,
        "graph_source":config.graph_source,"graph_path":config.graph_path})
        .to_string()
        .as_bytes(),
    )
}

pub fn source_hash(
    config: &KnowledgeConfig,
    connection: &crate::board::GithubConnectionModel,
) -> String {
    crate::documents::content_hash(
        serde_json::json!({"settings":config_hash(config),"owner":connection.owner,
        "repository":connection.repo,"branch":connection.branch,"root":connection.repo_path})
        .to_string()
        .as_bytes(),
    )
}

pub fn relative_path(path: &str) -> Result<String, String> {
    let path = path.trim().trim_end_matches('/');
    if path.is_empty()
        || path.len() > 2048
        || path.starts_with('/')
        || path.contains('\\')
        || path.contains(':')
        || path.chars().any(char::is_control)
        || path
            .split('/')
            .any(|part| part.is_empty() || matches!(part, "." | ".." | ".git"))
    {
        return Err(
            "Use a relative repository path without traversal, drive names or .git metadata."
                .into(),
        );
    }
    Ok(path.into())
}

pub fn normalize_config(mut config: KnowledgeConfig) -> Result<KnowledgeConfig, String> {
    config.connection = config.connection.trim().to_string();
    if config.enabled && config.connection.is_empty() {
        return Err("Choose a connected repository for project knowledge.".into());
    }
    if config.wiki_paths.len() > 20 {
        return Err("Use at most 20 wiki folders or files.".into());
    }
    config.wiki_paths = config
        .wiki_paths
        .iter()
        .filter(|path| !path.trim().is_empty())
        .map(|path| relative_path(path))
        .collect::<Result<Vec<_>, _>>()?;
    config.wiki_paths.sort();
    config.wiki_paths.dedup();
    if !matches!(
        config.graph_source.as_str(),
        "none" | "repository" | "upload"
    ) {
        return Err("Choose no graph, a repository export or an uploaded export.".into());
    }
    config.graph_path = config.graph_path.trim().to_string();
    if config.graph_source == "repository" {
        config.graph_path = relative_path(&config.graph_path)?;
    }
    if config.enabled && config.wiki_paths.is_empty() && config.graph_source == "none" {
        return Err("Add wiki paths or a Graphify source before enabling knowledge.".into());
    }
    Ok(config)
}

pub fn settings_view(app: &AppContext, prefix: &str) -> Result<KnowledgeSettingsResponse, String> {
    let project = crate::scripts::resolve_project_by_prefix(&app.board.read(), prefix)?;
    let (config, revision) = config(app, &project.id)?;
    let connections = project
        .github_connections
        .iter()
        .map(|connection| {
            let mirror = app.github.get_or_pending(&project.id, &connection.name);
            KnowledgeConnection {
                name: connection.name.clone(),
                repository: format!("{}/{}", connection.owner, connection.repo),
                branch: connection.branch.clone(),
                root_path: connection.repo_path.clone(),
                state: mirror.state.into(),
                commit: mirror.commit.clone(),
            }
        })
        .collect();
    let status = status(app, &project, &config);
    Ok(KnowledgeSettingsResponse {
        project: project.prefix.clone(),
        revision,
        config,
        connections,
        status,
    })
}

pub fn status(
    app: &AppContext,
    project: &crate::board::ProjectModel,
    config: &KnowledgeConfig,
) -> KnowledgeStatus {
    let snapshot = app.knowledge.snapshots.read().get(&project.id).cloned();
    let Some(snapshot) = snapshot else {
        let upload = app.knowledge.uploads.read().get(&project.id).cloned();
        return KnowledgeStatus {
            state: if config.enabled {
                "not-indexed"
            } else {
                "disabled"
            }
            .into(),
            graph_nodes: upload.as_ref().map_or(0, |g| g.graph.nodes.len() as i32),
            graph_edges: upload.as_ref().map_or(0, |g| g.graph.links.len() as i32),
            graph_commit: upload.map_or_else(String::new, |g| g.graph.built_at_commit.clone()),
            notice: app
                .knowledge
                .errors
                .read()
                .get(&project.id)
                .cloned()
                .unwrap_or_default(),
            ..Default::default()
        };
    };
    let mirror = app.github.get_or_pending(&project.id, &config.connection);
    let repository_matches =
        project
            .github_connection(&config.connection)
            .is_some_and(|connection| {
                snapshot.repository == format!("{}/{}", connection.owner, connection.repo)
                    && snapshot.repo_path == connection.repo_path
                    && snapshot.config_hash == source_hash(config, connection)
            });
    let upload_matches = config.graph_source != "upload"
        || snapshot.graph_upload_hash
            == app
                .knowledge
                .uploads
                .read()
                .get(&project.id)
                .map_or("", |g| g.content_hash.as_str());
    let fresh = repository_matches
        && upload_matches
        && graph::same_commit(&snapshot.commit, &mirror.commit);
    let graph_fresh = snapshot
        .graph
        .as_ref()
        .is_none_or(|graph| graph.built_at_commit == snapshot.commit);
    let mut notices = vec![snapshot.notice.clone()];
    if config.enabled && !fresh {
        notices.push("Repository or source settings changed. Refresh the knowledge index.".into());
    }
    if !graph_fresh {
        notices.push("Graphify export belongs to a different commit and is excluded from retrieval. Upload a current export.".into());
    }
    if let Some(error) = app.knowledge.errors.read().get(&project.id) {
        notices.push(error.clone());
    }
    KnowledgeStatus {
        state: if !config.enabled {
            "disabled"
        } else if !fresh {
            "stale"
        } else if !graph_fresh || !snapshot.notice.is_empty() {
            "partial"
        } else {
            "ready"
        }
        .into(),
        wiki_documents: snapshot.documents.len() as i32,
        graph_nodes: snapshot.graph.as_ref().map_or(0, |g| g.nodes.len() as i32),
        graph_edges: snapshot.graph.as_ref().map_or(0, |g| g.links.len() as i32),
        source_commit: snapshot.commit.clone(),
        graph_commit: snapshot
            .graph
            .as_ref()
            .map_or_else(String::new, |g| g.built_at_commit.clone()),
        updated_unix_seconds: Some(snapshot.updated),
        notice: notices
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" "),
    }
}

pub async fn save(
    app: &AppContext,
    input: &SaveKnowledgeInput,
    who: &str,
) -> Result<KnowledgeSettingsResponse, String> {
    let project = crate::scripts::resolve_project_by_prefix(&app.board.read(), &input.project)?;
    let config = normalize_config(KnowledgeConfig {
        enabled: input.enabled,
        connection: input.connection.clone(),
        wiki_paths: input.wiki_paths.clone(),
        graph_source: input.graph_source.clone(),
        graph_path: input.graph_path.clone(),
        auto_refresh: input.auto_refresh,
    })?;
    if !config.connection.is_empty() && project.github_connection(&config.connection).is_none() {
        return Err("Choose a repository connected to this project.".into());
    }
    let id = format!("knowledge:{}", project.id);
    let _guard = app.configuration.mutations.lock().await;
    if app.configuration.revision(&id) != input.revision {
        return Err("Knowledge settings changed in another session. Reload before saving.".into());
    }
    let row = AppSettingDto {
        id,
        value: serde_json::to_string(&config)
            .map_err(|_| "Knowledge settings could not be encoded")?,
        revision: input.revision + 1,
        updated_by: who.into(),
    };
    app.settings_repo.upsert(&row).await?;
    app.configuration.install(row);
    drop(_guard);
    settings_view(app, &project.prefix)
}

pub fn run_refresher(app: Arc<AppContext>) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            let projects: Vec<_> = app
                .board
                .read()
                .projects()
                .iter()
                .filter(|p| !p.is_archived())
                .cloned()
                .collect();
            for project in projects {
                let Ok((config, _)) = config(&app, &project.id) else {
                    continue;
                };
                if !config.enabled || !config.auto_refresh {
                    continue;
                }
                let state = status(&app, &project, &config).state;
                if matches!(state.as_str(), "not-indexed" | "stale") {
                    if let Err(error) = refresh(&app, &project.prefix).await {
                        app.knowledge
                            .errors
                            .write()
                            .insert(project.id.clone(), error);
                    }
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn knowledge_paths_and_scope_are_explicit() {
        for path in [
            "../secret",
            "/etc/passwd",
            "C:/secret",
            "docs/../secret",
            ".git/config",
            "docs\\secret",
        ] {
            assert!(relative_path(path).is_err());
        }
        assert_eq!(relative_path("docs/wiki/").unwrap(), "docs/wiki");
        assert!(
            normalize_config(KnowledgeConfig {
                enabled: true,
                ..Default::default()
            })
            .is_err()
        );
        let config = normalize_config(KnowledgeConfig {
            enabled: true,
            connection: "source".into(),
            graph_source: "none".into(),
            wiki_paths: vec!["docs/wiki/".into(), "docs/wiki".into()],
            ..Default::default()
        })
        .unwrap();
        assert_eq!(config.wiki_paths, vec!["docs/wiki"]);
        let foreign = AppSettingDto {
            id: "knowledge-snapshot:other".into(),
            value: serde_json::to_string(&Snapshot {
                project_id: "source".into(),
                ..Default::default()
            })
            .unwrap(),
            revision: 1,
            updated_by: "test".into(),
        };
        let cache = KnowledgeCache::restore(vec![foreign]);
        assert!(cache.snapshots.read().is_empty());
        assert!(cache.errors.read().contains_key("other"));
    }

    #[test]
    fn connection_branch_changes_invalidate_the_knowledge_source() {
        let config = KnowledgeConfig {
            connection: "source".into(),
            graph_source: "none".into(),
            ..Default::default()
        };
        let mut connection = crate::board::GithubConnectionModel {
            name: "source".into(),
            owner: "example".into(),
            repo: "repository".into(),
            branch: "main".into(),
            repo_path: String::new(),
        };
        let before = source_hash(&config, &connection);
        connection.branch = "release".into();
        assert_ne!(before, source_hash(&config, &connection));
    }
}
