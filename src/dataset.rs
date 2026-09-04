//! Dataset loaders. Every loader produces a plain `grust::Graph` whose node
//! ids are the source ids as strings, so the same graph can be handed to any
//! backend and to the in-memory oracle without translation.

use std::collections::HashSet;
use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;

use flate2::read::GzDecoder;
use grust::{Edge, Graph, Node, Props};

pub const NODE_LABEL: &str = "V";
pub const EDGE_LABEL: &str = "E";

/// Summary of what a loader saw, recorded in the report for provenance.
#[derive(Debug, Clone, serde::Serialize)]
pub struct LoadStats {
    pub file: String,
    pub lines: usize,
    pub nodes: usize,
    pub edges: usize,
    pub duplicate_edges_dropped: usize,
    pub self_loops: usize,
    pub truncated_at: Option<usize>,
}

fn open_maybe_gz(path: &Path) -> std::io::Result<Box<dyn Read>> {
    let file = File::open(path)?;
    if path.extension().is_some_and(|e| e == "gz") {
        Ok(Box::new(GzDecoder::new(file)))
    } else {
        Ok(Box::new(file))
    }
}

/// Load a SNAP-style edge list (`#` comments, whitespace-separated `from to`
/// per line). Exact duplicate edges are dropped and counted so that the
/// oracle and every backend see the same multiset; self-loops are kept and
/// counted. `limit` truncates after that many edges for smoke runs.
pub fn load_snap_edge_list(path: &Path, limit: Option<usize>) -> std::io::Result<(Graph, LoadStats)> {
    let reader = BufReader::with_capacity(1 << 20, open_maybe_gz(path)?);
    let mut nodes: HashSet<String> = HashSet::new();
    let mut seen: HashSet<(String, String)> = HashSet::new();
    let mut edges: Vec<Edge> = Vec::new();
    let mut lines = 0usize;
    let mut duplicates = 0usize;
    let mut self_loops = 0usize;
    let mut truncated_at = None;
    for line in reader.lines() {
        let line = line?;
        lines += 1;
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split(|c: char| c == '\t' || c == ' ' || c == ',');
        let (Some(from), Some(to)) = (parts.next(), parts.next()) else {
            continue;
        };
        if from == to {
            self_loops += 1;
        }
        if !seen.insert((from.to_string(), to.to_string())) {
            duplicates += 1;
            continue;
        }
        nodes.insert(from.to_string());
        nodes.insert(to.to_string());
        edges.push(Edge::new(EDGE_LABEL, from, to, Props::new()));
        if let Some(max) = limit
            && edges.len() >= max
        {
            truncated_at = Some(max);
            break;
        }
    }
    let mut node_ids: Vec<String> = nodes.into_iter().collect();
    node_ids.sort_unstable();
    let node_records: Vec<Node> = node_ids
        .into_iter()
        .map(|id| Node::new(NODE_LABEL, id, Props::new()))
        .collect();
    let stats = LoadStats {
        file: path.display().to_string(),
        lines,
        nodes: node_records.len(),
        edges: edges.len(),
        duplicate_edges_dropped: duplicates,
        self_loops,
        truncated_at,
    };
    Ok((Graph::new(node_records, edges), stats))
}

/// Deterministic synthetic star: one hub with `spokes` out-edges. Used for
/// hot-node contention when a real hub would be too large for a smoke run.
pub fn synthetic_star(spokes: usize) -> Graph {
    let mut nodes = Vec::with_capacity(spokes + 1);
    nodes.push(Node::new(NODE_LABEL, "hub", Props::new()));
    let mut edges = Vec::with_capacity(spokes);
    for i in 0..spokes {
        let id = format!("s{i}");
        nodes.push(Node::new(NODE_LABEL, id.clone(), Props::new()));
        edges.push(Edge::new(EDGE_LABEL, "hub", id, Props::new()));
    }
    Graph::new(nodes, edges)
}
