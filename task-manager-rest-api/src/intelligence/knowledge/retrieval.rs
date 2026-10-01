use super::{Snapshot, config, graph, source_hash};
use crate::app::AppContext;
use std::sync::Arc;
use task_manager_shared::{
    ai_settings::*,
    github::{github_mirror_path, mirror_document_id},
};

fn tokens(query: &str) -> Vec<String> {
    query
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .map(str::to_lowercase)
        .filter(|s| {
            s.len() >= 2
                && !matches!(
                    s.as_str(),
                    "the"
                        | "and"
                        | "for"
                        | "with"
                        | "this"
                        | "that"
                        | "from"
                        | "task"
                        | "для"
                        | "как"
                        | "или"
                        | "при"
                        | "что"
                )
        })
        .take(24)
        .collect()
}

fn score(title: &str, text: &str, query: &str, tokens: &[String]) -> usize {
    let title = title.to_lowercase();
    let text = text.to_lowercase();
    let query = query.to_lowercase();
    let exact = if title == query {
        100
    } else if title.contains(&query) {
        30
    } else {
        0
    };
    exact
        + tokens
            .iter()
            .map(|word| usize::from(title.contains(word)) * 5 + usize::from(text.contains(word)))
            .sum::<usize>()
}

fn excerpt(text: &str, tokens: &[String]) -> String {
    let pattern = tokens
        .iter()
        .map(|s| regex::escape(s))
        .collect::<Vec<_>>()
        .join("|");
    let start = if pattern.is_empty() {
        0
    } else {
        regex::RegexBuilder::new(&pattern)
            .case_insensitive(true)
            .build()
            .ok()
            .and_then(|re| re.find(text).map(|m| m.start()))
            .unwrap_or(0)
    };
    let mut begin = start.saturating_sub(100);
    while !text.is_char_boundary(begin) {
        begin += 1;
    }
    let clipped = super::super::semantic::clip(&text[begin..], 900);
    format!("{}{clipped}", if begin > 0 { "…" } else { "" })
}

fn rank_snapshot(
    snapshot: &Snapshot,
    prefix: &str,
    query: &str,
    limit: usize,
) -> Vec<KnowledgeHit> {
    let words = tokens(query);
    let mut ranked = Vec::new();
    let mut documents = Vec::new();
    for doc in &snapshot.documents {
        let rank = score(
            &format!("{} {}", doc.title, doc.path),
            &doc.text,
            query,
            &words,
        );
        if rank > 0 {
            documents.push((rank, doc));
        }
    }
    documents.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.path.cmp(&b.1.path)));
    for (rank, doc) in documents.into_iter().take(limit.clamp(1, 10)) {
        ranked.push((
            rank,
            KnowledgeHit {
                source: "wiki".into(),
                reference: mirror_document_id(
                    prefix,
                    &github_mirror_path(&snapshot.connection, &doc.path),
                ),
                title: doc.title.clone(),
                excerpt: excerpt(&doc.text, &words),
                content_hash: doc.content_hash.clone(),
                commit: snapshot.commit.clone(),
            },
        ));
    }
    if let Some(graph) = snapshot
        .graph
        .as_ref()
        .filter(|graph| graph.built_at_commit == snapshot.commit)
    {
        let mut candidates = Vec::new();
        for node in &graph.nodes {
            let relative = if snapshot.repo_path.is_empty() {
                Some(node.source_file.as_str())
            } else {
                node.source_file
                    .strip_prefix(&format!("{}/", snapshot.repo_path))
            };
            let Some(relative) = relative.filter(|path| !path.is_empty()) else {
                continue;
            };
            let rank = score(&node.label, &node.source_file, query, &words);
            if rank > 0 {
                candidates.push((rank, node, relative));
            }
        }
        candidates.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.id.cmp(&b.1.id)));
        for (rank, node, relative) in candidates.into_iter().take(limit.clamp(1, 10)) {
            let body = format!(
                "{} — {} {}. {}",
                node.label,
                node.source_file,
                node.source_location,
                graph::describe_neighbors(graph, node)
            );
            ranked.push((
                rank,
                KnowledgeHit {
                    source: "graphify".into(),
                    reference: mirror_document_id(
                        prefix,
                        &github_mirror_path(&snapshot.connection, relative),
                    ),
                    title: node.label.clone(),
                    excerpt: super::super::semantic::clip(&body, 900),
                    content_hash: crate::documents::content_hash(body.as_bytes()),
                    commit: graph.built_at_commit.clone(),
                },
            ));
        }
    }
    ranked.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.reference.cmp(&b.1.reference)));
    ranked
        .into_iter()
        .take(limit.clamp(1, 10))
        .map(|(_, hit)| hit)
        .collect()
}

pub fn search(
    app: &AppContext,
    prefix: &str,
    query: &str,
    limit: usize,
) -> Result<KnowledgePreviewResponse, String> {
    let project = crate::scripts::resolve_project_by_prefix(&app.board.read(), prefix)?;
    search_project(app, &project, query, limit)
}

pub fn search_project(
    app: &AppContext,
    project: &crate::board::ProjectModel,
    query: &str,
    limit: usize,
) -> Result<KnowledgePreviewResponse, String> {
    let query = query.trim();
    if query.is_empty() || query.len() > 4096 {
        return Err("Use a knowledge query between 1 and 4096 bytes.".into());
    }
    let (config, _) = config(app, &project.id)?;
    if !config.enabled {
        return Ok(KnowledgePreviewResponse {
            hits: vec![],
            notice: "Project knowledge is disabled.".into(),
        });
    }
    let Some(connection) = project.github_connection(&config.connection) else {
        return Ok(KnowledgePreviewResponse {
            hits: vec![],
            notice: "The knowledge repository connection no longer exists.".into(),
        });
    };
    let mirror = app.github.get_or_pending(&project.id, &config.connection);
    let snapshot: Option<Arc<Snapshot>> = app.knowledge.snapshots.read().get(&project.id).cloned();
    let upload_hash = app
        .knowledge
        .uploads
        .read()
        .get(&project.id)
        .map_or_else(String::new, |g| g.content_hash.clone());
    let Some(snapshot) = snapshot.filter(|snapshot| {
        snapshot.project_id == project.id
            && snapshot.config_hash == source_hash(&config, connection)
            && (config.graph_source != "upload" || snapshot.graph_upload_hash == upload_hash)
            && snapshot.repository == format!("{}/{}", connection.owner, connection.repo)
            && snapshot.repo_path == connection.repo_path
            && graph::same_commit(&snapshot.commit, &mirror.commit)
    }) else {
        return Ok(KnowledgePreviewResponse { hits: vec![], notice: "Project knowledge is missing or stale. Refresh sources in Settings → Knowledge & indexing.".into() });
    };
    let mut notice = snapshot.notice.clone();
    if snapshot
        .graph
        .as_ref()
        .is_some_and(|graph| graph.built_at_commit != snapshot.commit)
    {
        notice.push_str(
            " Graphify belongs to an older commit and is excluded; upload a current export.",
        );
    }
    let hits = rank_snapshot(&snapshot, &project.prefix, query, limit);
    Ok(KnowledgePreviewResponse {
        hits,
        notice: notice.trim().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retrieval_cites_project_files_and_excludes_stale_graphs() {
        let mut snapshot = Snapshot {
            project_id: "project-a".into(),
            connection: "source".into(),
            commit: "a".repeat(40),
            documents: vec![super::super::WikiDocument {
                path: "docs/wiki/queues.md".into(),
                title: "Queues".into(),
                text: "# Queues\nValkey stores job queues.".into(),
                content_hash: "hash".into(),
            }],
            graph: Some(graph::GraphExport {
                built_at_commit: "a".repeat(40),
                nodes: vec![graph::GraphNode {
                    id: "valkey".into(),
                    label: "ValkeyClient".into(),
                    source_file: "src/valkey.rs".into(),
                    source_location: "L1".into(),
                }],
                links: vec![],
            }),
            ..Default::default()
        };
        let hits = rank_snapshot(&snapshot, "DEMO", "Valkey", 5);
        assert_eq!(hits.len(), 2);
        assert!(hits.iter().all(|hit| hit.reference.contains("DEMO")));
        snapshot.graph.as_mut().unwrap().built_at_commit = "b".repeat(40);
        let hits = rank_snapshot(&snapshot, "OTHER", "Valkey", 5);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].source, "wiki");
        assert!(hits[0].reference.contains("OTHER"));
        assert!(excerpt("İ Unicode строка Valkey", &tokens("строка")).contains("строка"));
    }
}
