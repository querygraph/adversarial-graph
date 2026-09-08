//! Dataset loaders. Every loader produces a plain `grust::Graph` whose node
//! ids are the source ids as strings, so the same graph can be handed to any
//! backend and to the in-memory oracle without translation. The SNAP loader
//! gives one node label and one relationship type; the typed loaders (LDBC
//! SNB CsvBasic, ICIJ Offshore Leaks) give labelled nodes with typed
//! properties and named relationship types under the same duplicate,
//! self-loop and truncation rules, and report the schema they produced.

pub mod icij;
pub mod snb;
pub mod typed;

use std::collections::{BTreeMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;

use flate2::read::GzDecoder;
use grust::{Edge, Graph, Node, Props};

pub use typed::DatasetSchema;

pub const NODE_LABEL: &str = "V";
pub const EDGE_LABEL: &str = "E";
pub const SNAP_FORMAT: &str = "snap-edge-list";

/// Summary of what a loader saw, recorded in the report for provenance.
#[derive(Debug, Clone, serde::Serialize)]
pub struct LoadStats {
    pub file: String,
    pub format: String,
    pub lines: usize,
    pub nodes: usize,
    pub edges: usize,
    pub duplicate_edges_dropped: usize,
    pub dangling_edges_dropped: usize,
    pub self_loops: usize,
    pub truncated_at: Option<usize>,
    pub node_labels: BTreeMap<String, usize>,
    pub relationship_labels: BTreeMap<String, usize>,
}

/// Which loader a dataset file needs, from its name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatasetFormat {
    Snap,
    LdbcSnbCsvBasic,
    IcijOffshoreLeaks,
}

impl DatasetFormat {
    pub fn of(file: &str) -> Self {
        if file.contains("CsvBasic") && file.ends_with(".tar.zst") {
            Self::LdbcSnbCsvBasic
        } else if file.starts_with("full-oldb") && file.ends_with(".zip") {
            Self::IcijOffshoreLeaks
        } else {
            Self::Snap
        }
    }
}

/// What a loader produced: the parsed `Graph`, or the compact reference
/// for a SNAP tier whose `Graph` would not fit the host (`crate::compact`).
pub enum LoadedGraph {
    Full(Graph),
    Compact(crate::compact::CompactGraph),
}

impl LoadedGraph {
    pub fn reference_name(&self) -> &'static str {
        match self {
            Self::Full(_) => "materialized",
            Self::Compact(_) => "compact",
        }
    }
}

/// Load a dataset by its manifest file name, dispatching on the format.
/// `compact` asks for the compact reference; only the SNAP loader has one,
/// the typed loaders always materialize.
pub fn load_dataset(
    path: &Path,
    limit: Option<usize>,
    compact: bool,
) -> std::io::Result<(LoadedGraph, LoadStats, DatasetSchema)> {
    let file = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    match DatasetFormat::of(file) {
        DatasetFormat::Snap if compact => {
            let (graph, stats) = crate::compact::load_snap_compact(path, limit)?;
            let schema = DatasetSchema::untyped(&stats);
            Ok((LoadedGraph::Compact(graph), stats, schema))
        }
        DatasetFormat::Snap => {
            let (graph, stats) = load_snap_edge_list(path, limit)?;
            let schema = DatasetSchema::of(&graph);
            Ok((LoadedGraph::Full(graph), stats, schema))
        }
        DatasetFormat::LdbcSnbCsvBasic => {
            snb::load(path, limit).map(|(g, s, d)| (LoadedGraph::Full(g), s, d))
        }
        DatasetFormat::IcijOffshoreLeaks => {
            icij::load(path, limit).map(|(g, s, d)| (LoadedGraph::Full(g), s, d))
        }
    }
}

pub fn open_maybe_gz(path: &Path) -> std::io::Result<Box<dyn Read>> {
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
pub fn load_snap_edge_list(
    path: &Path,
    limit: Option<usize>,
) -> std::io::Result<(Graph, LoadStats)> {
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
        let mut parts = line.split(['\t', ' ', ',']);
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
        format: SNAP_FORMAT.to_string(),
        lines,
        nodes: node_records.len(),
        edges: edges.len(),
        duplicate_edges_dropped: duplicates,
        dangling_edges_dropped: 0,
        self_loops,
        truncated_at,
        node_labels: BTreeMap::from([(NODE_LABEL.to_string(), node_records.len())]),
        relationship_labels: BTreeMap::from([(EDGE_LABEL.to_string(), edges.len())]),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_are_recognised_from_the_manifest_file_name() {
        assert_eq!(DatasetFormat::of("wiki-Talk.txt.gz"), DatasetFormat::Snap);
        assert_eq!(
            DatasetFormat::of("social_network-sf0.1-CsvBasic-LongDateFormatter.tar.zst"),
            DatasetFormat::LdbcSnbCsvBasic
        );
        assert_eq!(
            DatasetFormat::of("full-oldb.LATEST.zip"),
            DatasetFormat::IcijOffshoreLeaks
        );
    }
}
