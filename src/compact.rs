//! The compact reference: a parsed SNAP edge list as interned `u32` ids and a
//! CSR, for the tiers whose parsed `Graph` does not fit a host.
//!
//! §46 measured the harness client's footprint: the oracle's index is small
//! (0.23 GB at web-Google), the parsed `grust::Graph` is the weight, about
//! 430 bytes per edge -- ~47 GB at com-Orkut's 117 M edges, more than any
//! host has. Above a threshold the harness therefore never materialises the
//! whole `Graph`: it parses into this structure, builds the oracle over the
//! CSR, and feeds the store in chunks of transient `Graph`s. The LOAD row
//! discloses that path. Below the threshold nothing changes.

use std::collections::{BTreeMap, HashMap};

use grust::{Edge, Graph, Node, Props};

use crate::dataset::pairs::PairFormat;
use crate::dataset::{EDGE_LABEL, LoadStats, NODE_LABEL};

/// Interned vertices and a CSR over the out-edges. Node index = position in
/// `ids`, which is sorted, so the sample and hub choices match the `Graph`
/// path's (its loaders sort node ids too).
pub struct CompactGraph {
    pub ids: Vec<String>,
    /// `out_offsets[v]..out_offsets[v+1]` index `out_targets`.
    pub out_offsets: Vec<u32>,
    pub out_targets: Vec<u32>,
    pub edge_count: usize,
}

impl CompactGraph {
    pub fn node_count(&self) -> usize {
        self.ids.len()
    }

    pub fn index_of(&self, id: &str) -> Option<usize> {
        self.ids
            .binary_search_by(|probe| probe.as_str().cmp(id))
            .ok()
    }

    pub fn out(&self, v: usize) -> &[u32] {
        &self.out_targets[self.out_offsets[v] as usize..self.out_offsets[v + 1] as usize]
    }

    /// The vertices as a node-only `Graph`, in batches: what a store's
    /// `put_graph` needs before any edge that references them.
    pub fn node_chunks(&self, batch: usize) -> impl Iterator<Item = Graph> + '_ {
        self.ids.chunks(batch.max(1)).map(|ids| {
            Graph::new(
                ids.iter()
                    .map(|id| Node::new(NODE_LABEL, id.as_str(), Props::new()))
                    .collect(),
                Vec::new(),
            )
        })
    }

    /// The edges as `Graph`s of at most `batch` edges each, in CSR order,
    /// so the store sees every edge exactly once. Edge-only unless
    /// `with_endpoints`, when each chunk also carries the distinct vertices
    /// its edges touch (for an adapter that resolves endpoints within the
    /// batch; see `BackendKind::edge_batches_carry_endpoints`).
    pub fn edge_chunks(
        &self,
        batch: usize,
        with_endpoints: bool,
    ) -> impl Iterator<Item = Graph> + '_ {
        let batch = batch.max(1);
        let mut v = 0usize;
        let mut i = 0usize; // position within v's out-list
        std::iter::from_fn(move || {
            let mut edges = Vec::with_capacity(batch.min(self.edge_count));
            let mut touched: Vec<u32> = Vec::new();
            while edges.len() < batch && v < self.ids.len() {
                let out = self.out(v);
                if i < out.len() {
                    edges.push(Edge::new(
                        EDGE_LABEL,
                        self.ids[v].as_str(),
                        self.ids[out[i] as usize].as_str(),
                        Props::new(),
                    ));
                    if with_endpoints {
                        touched.push(v as u32);
                        touched.push(out[i]);
                    }
                    i += 1;
                } else {
                    v += 1;
                    i = 0;
                }
            }
            if edges.is_empty() {
                return None;
            }
            touched.sort_unstable();
            touched.dedup();
            let nodes = touched
                .into_iter()
                .map(|t| Node::new(NODE_LABEL, self.ids[t as usize].as_str(), Props::new()))
                .collect();
            Some(Graph::new(nodes, edges))
        })
    }
}

impl CompactGraph {
    /// The first `max_edges` edges in CSR order with every vertex they
    /// touch, plus vertex 0 even when it has no out-edge: a bounded
    /// `Graph` for the reference executor's policy checks.
    pub fn prefix_subgraph(&self, max_edges: usize) -> Graph {
        let mut edges = Vec::new();
        let mut touched: Vec<u32> = vec![0];
        'outer: for v in 0..self.ids.len() {
            for &t in self.out(v) {
                if edges.len() >= max_edges {
                    break 'outer;
                }
                edges.push(Edge::new(
                    EDGE_LABEL,
                    self.ids[v].as_str(),
                    self.ids[t as usize].as_str(),
                    Props::new(),
                ));
                touched.push(v as u32);
                touched.push(t);
            }
        }
        touched.sort_unstable();
        touched.dedup();
        let nodes = touched
            .into_iter()
            .map(|v| Node::new(NODE_LABEL, self.ids[v as usize].as_str(), Props::new()))
            .collect();
        Graph::new(nodes, edges)
    }
}

/// Parse a SNAP edge list into a `CompactGraph`, with the same dedup,
/// self-loop and truncation accounting as the `Graph` loader, so the
/// `LoadStats` a row carries do not depend on which path built them.
pub fn load_snap_compact(
    path: &std::path::Path,
    limit: Option<usize>,
    format: PairFormat,
) -> std::io::Result<(CompactGraph, LoadStats)> {
    let mut source = crate::dataset::pairs::PairSource::open(path, format)?;
    let keep_parallel = format.keeps_parallel_edges();
    let mut intern: HashMap<String, u32> = HashMap::new();
    let mut ids: Vec<String> = Vec::new();
    let mut pairs: Vec<(u32, u32)> = Vec::new();
    let mut self_loops = 0usize;
    let mut truncated_at = None;
    let id_of = |s: &str, intern: &mut HashMap<String, u32>, ids: &mut Vec<String>| -> u32 {
        if let Some(&i) = intern.get(s) {
            return i;
        }
        let i = ids.len() as u32;
        ids.push(s.to_string());
        intern.insert(s.to_string(), i);
        i
    };
    while let Some((from, to)) = source.next_pair()? {
        if from == to {
            self_loops += 1;
        }
        let f = id_of(&from, &mut intern, &mut ids);
        let t = id_of(&to, &mut intern, &mut ids);
        pairs.push((f, t));
        if let Some(max) = limit
            && pairs.len() >= max
        {
            truncated_at = Some(max);
            break;
        }
    }
    let lines = source.lines;
    drop(intern);
    // Dedup exactly as the Graph loader does (first occurrence kept): sort a
    // permutation by (from,to) string order? The Graph loader dedups on the
    // raw strings, and truncation counts kept edges; both hold here because
    // interning is injective on the strings. Under a temporal format the
    // repeats are parallel edges and stay; the CSR simply lists a target
    // more than once.
    let before = pairs.len();
    pairs.sort_unstable();
    let (duplicates, parallel) = if keep_parallel {
        let mut distinct = pairs.clone();
        distinct.dedup();
        (0, before - distinct.len())
    } else {
        pairs.dedup();
        (before - pairs.len(), 0)
    };
    // Renumber so that index order == sorted id order, matching the Graph
    // loader's `node_ids.sort_unstable()`.
    let mut order: Vec<u32> = (0..ids.len() as u32).collect();
    order.sort_unstable_by(|&a, &b| ids[a as usize].cmp(&ids[b as usize]));
    let mut rank = vec![0u32; ids.len()];
    for (r, &old) in order.iter().enumerate() {
        rank[old as usize] = r as u32;
    }
    let sorted_ids: Vec<String> = order.iter().map(|&o| ids[o as usize].clone()).collect();
    drop(ids);
    for p in pairs.iter_mut() {
        *p = (rank[p.0 as usize], rank[p.1 as usize]);
    }
    drop(rank);
    pairs.sort_unstable();
    let n = sorted_ids.len();
    let mut out_offsets = vec![0u32; n + 1];
    for &(f, _) in &pairs {
        out_offsets[f as usize + 1] += 1;
    }
    for v in 0..n {
        out_offsets[v + 1] += out_offsets[v];
    }
    let out_targets: Vec<u32> = pairs.iter().map(|&(_, t)| t).collect();
    let edge_count = pairs.len();
    drop(pairs);
    let stats = LoadStats {
        file: path.display().to_string(),
        format: format.name().to_string(),
        lines,
        nodes: n,
        edges: edge_count,
        duplicate_edges_dropped: duplicates,
        parallel_edges: parallel,
        dangling_edges_dropped: 0,
        self_loops,
        truncated_at,
        node_labels: BTreeMap::from([(NODE_LABEL.to_string(), n)]),
        relationship_labels: BTreeMap::from([(EDGE_LABEL.to_string(), edge_count)]),
    };
    Ok((
        CompactGraph {
            ids: sorted_ids,
            out_offsets,
            out_targets,
            edge_count,
        },
        stats,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csr_matches_the_edge_list_and_chunks_cover_every_edge_once() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("g.txt");
        std::fs::write(&p, "# c\n3 1\n1 2\n1 2\n2 3\n3 3\n1 3\n").unwrap();
        let (g, s) = load_snap_compact(&p, None, PairFormat::SnapEdgeList).unwrap();
        assert_eq!(g.ids, vec!["1", "2", "3"]);
        assert_eq!(s.edges, 5);
        assert_eq!(s.duplicate_edges_dropped, 1);
        assert_eq!(s.self_loops, 1);
        assert_eq!(g.out(0), &[1, 2]); // 1->2, 1->3
        assert_eq!(g.out(1), &[2]); // 2->3
        assert_eq!(g.out(2), &[0, 2]); // 3->1, 3->3
        let mut seen = 0;
        for chunk in g.edge_chunks(2, false) {
            seen += chunk.edges.len();
            assert!(chunk.edges.len() <= 2);
            assert!(chunk.nodes.is_empty());
        }
        assert_eq!(seen, 5);
        let carried: Vec<Graph> = g.edge_chunks(2, true).collect();
        assert_eq!(carried.iter().map(|c| c.edges.len()).sum::<usize>(), 5);
        // first chunk: 1->2, 1->3 touches vertices 1, 2, 3
        assert_eq!(carried[0].nodes.len(), 3);
        assert_eq!(g.node_chunks(2).map(|c| c.nodes.len()).sum::<usize>(), 3);
        let prefix = g.prefix_subgraph(3);
        assert_eq!(prefix.edges.len(), 3);
        assert_eq!(prefix.nodes.len(), 3);
        assert_eq!(g.index_of("2"), Some(1));
        assert_eq!(g.index_of("9"), None);
        // The temporal format keeps the repeated 1->2 as a parallel edge.
        let (g, s) = load_snap_compact(&p, None, PairFormat::SnapTemporal).unwrap();
        assert_eq!(s.edges, 6);
        assert_eq!(s.parallel_edges, 1);
        assert_eq!(s.duplicate_edges_dropped, 0);
        assert_eq!(g.out(0), &[1, 1, 2]);
        assert_eq!(
            g.edge_chunks(4, false)
                .map(|c| c.edges.len())
                .sum::<usize>(),
            6
        );
    }
}
