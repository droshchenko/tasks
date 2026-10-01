use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const MAX_GRAPH_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Serialize, Deserialize)]
pub struct GraphNode {
    pub id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub source_file: String,
    #[serde(default, deserialize_with = "nullable_string")]
    pub source_location: String,
}

fn nullable_string<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    Ok(Option::<String>::deserialize(deserializer)?.unwrap_or_default())
}

#[derive(Clone, Serialize, Deserialize)]
pub struct GraphEdge {
    pub source: String,
    pub target: String,
    #[serde(default)]
    pub relation: String,
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct GraphExport {
    #[serde(default)]
    pub built_at_commit: String,
    pub nodes: Vec<GraphNode>,
    #[serde(default, alias = "edges")]
    pub links: Vec<GraphEdge>,
}

pub fn parse(bytes: &[u8]) -> Result<GraphExport, String> {
    if bytes.len() > MAX_GRAPH_BYTES {
        return Err("Graphify export exceeds the 64 MiB limit.".into());
    }
    let mut graph: GraphExport = serde_json::from_slice(bytes)
        .map_err(|_| "Expected a Graphify JSON export containing nodes and links.")?;
    if graph.nodes.len() > 100_000 || graph.links.len() > 300_000 {
        return Err("Graphify export exceeds 100,000 nodes or 300,000 links.".into());
    }
    if !valid_commit(&graph.built_at_commit) {
        return Err("The graph needs built_at_commit containing its full source commit SHA. Rebuild or export Graphify from the intended revision.".into());
    }
    graph.built_at_commit.make_ascii_lowercase();
    let mut ids = HashSet::new();
    for node in &mut graph.nodes {
        if node.id.is_empty()
            || node.id.len() > 2048
            || node.id.chars().any(char::is_control)
            || !ids.insert(node.id.clone())
        {
            return Err("Graph nodes must have unique, bounded string IDs.".into());
        }
        if node.label.is_empty() {
            node.label = node.id.clone();
        }
        if node.label.len() > 4096 || node.source_location.len() > 256 {
            return Err("Graph node metadata is too long.".into());
        }
        // Graphify includes Node.js built-ins as virtual sources, not repository files.
        if node.source_file.starts_with("node:") {
            node.source_file.clear();
        } else if !node.source_file.is_empty() {
            node.source_file = super::relative_path(&node.source_file)?;
        }
    }
    for edge in &graph.links {
        if !ids.contains(&edge.source) || !ids.contains(&edge.target) || edge.relation.len() > 256 {
            return Err("Every graph link must name existing nodes and a bounded relation.".into());
        }
    }
    Ok(graph)
}

pub fn valid_commit(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|c| c.is_ascii_hexdigit())
}

pub fn same_commit(full: &str, current: &str) -> bool {
    !current.is_empty() && current.len() >= 7 && full.starts_with(current)
}

pub fn describe_neighbors(graph: &GraphExport, node: &GraphNode) -> String {
    graph
        .links
        .iter()
        .filter(|edge| edge.source == node.id || edge.target == node.id)
        .take(6)
        .map(|edge| {
            let (other, direction) = if edge.source == node.id {
                (&edge.target, "outgoing")
            } else {
                (&edge.source, "incoming")
            };
            format!("{direction} {}: {other}", edge.relation)
        })
        .collect::<Vec<_>>()
        .join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> serde_json::Value {
        serde_json::json!({"built_at_commit":"a".repeat(40), "nodes":[
            {"id":"a","label":"run","source_file":"src/run.rs","source_location":"L5","extra":"ignored"},
            {"id":"b","label":"parse","source_file":"src/parse.rs","source_location":null}],
            "links":[{"source":"a","target":"b","relation":"calls","confidence":1.0}]})
    }
    #[test]
    fn graphify_exports_keep_citable_symbols_and_validate_relationships() {
        let value = fixture();
        let graph = parse(&serde_json::to_vec(&value).unwrap()).unwrap();
        assert_eq!(graph.nodes.len(), 2);
        assert!(describe_neighbors(&graph, &graph.nodes[0]).contains("calls: b"));
        assert!(same_commit(&graph.built_at_commit, "aaaaaaa"));
        assert!(!same_commit(&graph.built_at_commit, "bbbbbbb"));
        for mutation in 0..3 {
            let mut invalid = fixture();
            if mutation == 0 {
                invalid["nodes"][0]["source_file"] = "../credentials".into();
            }
            if mutation == 1 {
                invalid["links"][0]["target"] = "missing".into();
            }
            if mutation == 2 {
                invalid["built_at_commit"] = "main".into();
            }
            assert!(parse(&serde_json::to_vec(&invalid).unwrap()).is_err());
        }
    }

    #[test]
    #[ignore = "requires an explicitly supplied TASKS_TEST_GRAPHIFY_EXPORT fixture"]
    fn real_graphify_export_is_compatible() {
        let path = std::env::var("TASKS_TEST_GRAPHIFY_EXPORT").expect("fixture path required");
        let graph = parse(&std::fs::read(path).unwrap()).unwrap();
        assert!(!graph.nodes.is_empty());
        eprintln!(
            "Validated Graphify export: {} nodes, {} links",
            graph.nodes.len(),
            graph.links.len()
        );
    }

    #[test]
    fn virtual_node_builtins_are_retained_as_neighbors_without_file_references() {
        let mut value = fixture();
        value["nodes"][1]["source_file"] = "node:fs".into();
        let graph = parse(&serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(graph.nodes[1].source_file.is_empty());
        assert_eq!(graph.links.len(), 1);
    }
}
