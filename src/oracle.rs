//! Ground truth computed from the loaded graph. Every structural scenario
//! compares a backend's answer with these numbers; a mismatch is a
//! `wrong_answer` hard-gate failure.
//!
//! The reference is either the parsed `grust::Graph` under Grust's in-memory
//! `GraphIndex` (typed datasets and the SNAP tiers that fit), or the compact
//! CSR of `crate::compact` for the SNAP tiers whose `Graph` would not fit a
//! host (§46: ~430 bytes per edge). Both answer the same questions from the
//! same vertex order, so a row does not depend on which one built it.

use std::collections::{BTreeMap, HashSet, VecDeque};

use grust::{Graph, GraphIndex, Label, NodeId};

use crate::compact::CompactGraph;
use crate::dataset::{DatasetSchema, EDGE_LABEL};

pub enum Reference<'g> {
    Indexed { graph: &'g Graph, index: GraphIndex },
    Compact(&'g CompactGraph),
}

pub struct Oracle<'g> {
    pub reference: Reference<'g>,
    /// Labels and relationship types with their counts, as the loader
    /// produced them; one of each for a SNAP edge list.
    pub schema: DatasetSchema,
}

/// Which edges a traversal follows: every edge, or one relationship type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // the typed traversal families select a relationship type; A1–A7 follow every edge
pub enum EdgeFilter<'a> {
    Any,
    Label(&'a Label),
}

impl<'g> Oracle<'g> {
    #[allow(dead_code)]
    pub fn new(graph: &'g Graph) -> grust::Result<Self> {
        Self::with_schema(graph, DatasetSchema::of(graph))
    }

    pub fn with_schema(graph: &'g Graph, schema: DatasetSchema) -> grust::Result<Self> {
        Ok(Self {
            reference: Reference::Indexed {
                graph,
                index: GraphIndex::new(graph)?,
            },
            schema,
        })
    }

    /// The oracle over the compact reference; nothing here can fail.
    pub fn compact(graph: &'g CompactGraph, schema: DatasetSchema) -> Self {
        Self {
            reference: Reference::Compact(graph),
            schema,
        }
    }

    pub fn node_count(&self) -> usize {
        match &self.reference {
            Reference::Indexed { graph, .. } => graph.nodes.len(),
            Reference::Compact(c) => c.node_count(),
        }
    }

    fn node_id(&self, index: usize) -> NodeId {
        match &self.reference {
            Reference::Indexed { graph, .. } => graph.nodes[index].id.clone(),
            Reference::Compact(c) => NodeId::from(c.ids[index].as_str()),
        }
    }

    /// The lowest-id vertex (loaders sort node ids): A2's deterministic
    /// start, usually far from a hub on road graphs.
    pub fn first_vertex(&self) -> NodeId {
        self.node_id(0)
    }

    fn vertex_index(&self, id: &NodeId) -> Option<usize> {
        match &self.reference {
            Reference::Indexed { index, .. } => index.require_vertex_index(id).ok(),
            Reference::Compact(c) => c.index_of(id.as_str()),
        }
    }

    /// Call `f` with the target of every out-edge of `v` that `filter`
    /// admits (once per edge, so parallel edges repeat their target).
    fn for_each_out(&self, v: usize, filter: EdgeFilter<'_>, mut f: impl FnMut(usize)) {
        match &self.reference {
            Reference::Indexed { index, .. } => {
                for &e in index.outgoing_by_vertex(v) {
                    if self.follows(e, filter) {
                        f(index.edge_endpoints(e).1);
                    }
                }
            }
            Reference::Compact(c) => {
                if compact_follows(filter) {
                    for &t in c.out(v) {
                        f(t as usize);
                    }
                }
            }
        }
    }

    fn out_degree_over(&self, v: usize, filter: EdgeFilter<'_>) -> usize {
        match (&self.reference, filter) {
            (Reference::Indexed { index, .. }, EdgeFilter::Any) => index.out_degree(v),
            (Reference::Indexed { index, .. }, EdgeFilter::Label(_)) => index
                .outgoing_by_vertex(v)
                .iter()
                .filter(|&&e| self.follows(e, filter))
                .count(),
            (Reference::Compact(c), _) => {
                if compact_follows(filter) {
                    c.out(v).len()
                } else {
                    0
                }
            }
        }
    }

    /// Node count per label, in label order.
    #[allow(dead_code)] // the M2 families (A8 first) read the schema; A1–A7 do not
    pub fn node_counts(&self) -> &BTreeMap<String, usize> {
        &self.schema.node_labels
    }

    /// Edge count per relationship type, in type order.
    #[allow(dead_code)]
    pub fn relationship_counts(&self) -> &BTreeMap<String, usize> {
        &self.schema.relationship_labels
    }

    fn follows(&self, edge: usize, filter: EdgeFilter<'_>) -> bool {
        match (filter, &self.reference) {
            (EdgeFilter::Any, _) => true,
            (EdgeFilter::Label(label), Reference::Indexed { graph, .. }) => {
                &graph.edges[edge].label == label
            }
            (EdgeFilter::Label(_), Reference::Compact(_)) => compact_follows(filter),
        }
    }

    /// Out-degree of one vertex in the untyped view (0 for an unknown id).
    pub fn out_degree(&self, id: &NodeId) -> usize {
        self.vertex_index(id)
            .map(|v| self.out_degree_over(v, EdgeFilter::Any))
            .unwrap_or(0)
    }

    /// A deterministic sample of up to `n` vertex ids spread evenly through
    /// the node list, so a stream over it touches the graph broadly and the
    /// same sample recurs run to run.
    pub fn sample_vertices(&self, n: usize) -> Vec<NodeId> {
        let total = self.node_count();
        if total == 0 || n == 0 {
            return Vec::new();
        }
        let step = (total / n).max(1);
        (0..total)
            .step_by(step)
            .take(n)
            .map(|i| self.node_id(i))
            .collect()
    }

    pub fn max_out_degree_vertex(&self) -> (NodeId, usize) {
        self.max_out_degree_vertex_over(EdgeFilter::Any)
    }

    /// Vertex with the largest out-degree counting only the edges `filter`
    /// admits (ties broken by id order, which is stable because loaders
    /// sort node ids).
    pub fn max_out_degree_vertex_over(&self, filter: EdgeFilter<'_>) -> (NodeId, usize) {
        let mut best = (0usize, 0usize);
        for index in 0..self.node_count() {
            let degree = self.out_degree_over(index, filter);
            if degree > best.1 {
                best = (index, degree);
            }
        }
        (self.node_id(best.0), best.1)
    }

    /// Distinct vertices reachable in exactly 1..=k out-hops, per layer, and
    /// the distinct union (excluding the start vertex itself).
    pub fn khop_layers(&self, start: &NodeId, k: usize) -> (Vec<usize>, usize) {
        self.khop_layers_over(start, k, EdgeFilter::Any)
    }

    pub fn khop_layers_over(
        &self,
        start: &NodeId,
        k: usize,
        filter: EdgeFilter<'_>,
    ) -> (Vec<usize>, usize) {
        let start_index = self.vertex_index(start).expect("start vertex present");
        let mut visited: HashSet<usize> = HashSet::new();
        visited.insert(start_index);
        let mut frontier = vec![start_index];
        let mut layers = Vec::with_capacity(k);
        for _ in 0..k {
            let mut next = Vec::new();
            for v in &frontier {
                self.for_each_out(*v, filter, |to| {
                    if visited.insert(to) {
                        next.push(to);
                    }
                });
            }
            layers.push(next.len());
            frontier = next;
            if frontier.is_empty() {
                break;
            }
        }
        (layers, visited.len() - 1)
    }

    /// Distinct vertices reachable from `start` following out-edges to depth
    /// `max_depth` (undirected graphs are loaded with both directions, so
    /// this doubles as undirected BFS for them).
    pub fn bfs_depth(&self, start: &NodeId, max_depth: usize) -> (usize, usize) {
        self.bfs_depth_over(start, max_depth, EdgeFilter::Any)
    }

    pub fn bfs_depth_over(
        &self,
        start: &NodeId,
        max_depth: usize,
        filter: EdgeFilter<'_>,
    ) -> (usize, usize) {
        let start_index = self.vertex_index(start).expect("start vertex present");
        let mut visited = vec![false; self.node_count()];
        visited[start_index] = true;
        let mut queue = VecDeque::from([(start_index, 0usize)]);
        let mut reached = 0usize;
        let mut deepest = 0usize;
        while let Some((v, d)) = queue.pop_front() {
            if d >= max_depth {
                continue;
            }
            self.for_each_out(v, filter, |to| {
                if !visited[to] {
                    visited[to] = true;
                    reached += 1;
                    deepest = deepest.max(d + 1);
                    queue.push_back((to, d + 1));
                }
            });
        }
        (reached, deepest)
    }
}

/// The compact reference carries the SNAP shape only: one relationship
/// type, so a label filter admits every edge or none.
fn compact_follows(filter: EdgeFilter<'_>) -> bool {
    match filter {
        EdgeFilter::Any => true,
        EdgeFilter::Label(label) => label.as_str() == EDGE_LABEL,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use grust::{Edge, Node, Props};

    #[test]
    fn label_filters_change_the_hub_and_the_layers() {
        let node = |label: &str, id: &str| Node::new(label, id, Props::new());
        let edge = |label: &str, from: &str, to: &str| Edge::new(label, from, to, Props::new());
        let graph = Graph::new(
            vec![
                node("Person", "a"),
                node("Person", "b"),
                node("Person", "c"),
                node("Tag", "t"),
            ],
            vec![
                edge("KNOWS", "a", "b"),
                edge("KNOWS", "b", "c"),
                edge("HAS_INTEREST", "a", "t"),
                edge("HAS_INTEREST", "b", "t"),
                edge("HAS_INTEREST", "c", "t"),
            ],
        );
        let oracle = Oracle::new(&graph).unwrap();
        assert_eq!(oracle.node_counts()["Person"], 3);
        assert_eq!(oracle.relationship_counts()["HAS_INTEREST"], 3);
        assert_eq!(
            oracle.schema.dominant_relationship.as_deref(),
            Some("HAS_INTEREST")
        );
        let knows = Label::from("KNOWS");
        assert_eq!(oracle.max_out_degree_vertex().1, 2);
        assert_eq!(
            oracle
                .max_out_degree_vertex_over(EdgeFilter::Label(&knows))
                .1,
            1
        );
        let a = NodeId::from("a");
        assert_eq!(oracle.khop_layers(&a, 2), (vec![2, 1], 3));
        assert_eq!(
            oracle.khop_layers_over(&a, 2, EdgeFilter::Label(&knows)),
            (vec![1, 1], 2)
        );
        assert_eq!(
            oracle.bfs_depth_over(&a, 8, EdgeFilter::Label(&knows)),
            (2, 2)
        );
    }

    #[test]
    fn the_compact_reference_answers_as_the_indexed_one() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("g.txt");
        // a small directed graph with a hub (b), a chain, a cycle and a loop
        let mut text = String::new();
        for t in ["c", "d", "e", "f"] {
            text.push_str(&format!("b {t}\n"));
        }
        text.push_str("a b\nd e\ne f\nf d\nf f\ng a\nh h\n");
        std::fs::write(&p, text).unwrap();
        let (graph, _) = crate::dataset::load_snap_edge_list(&p, None).unwrap();
        let (compact, _) = crate::compact::load_snap_compact(&p, None).unwrap();
        let full = Oracle::new(&graph).unwrap();
        let small = Oracle::compact(&compact, DatasetSchema::of(&graph));
        assert_eq!(full.node_count(), small.node_count());
        assert_eq!(full.first_vertex(), small.first_vertex());
        assert_eq!(full.sample_vertices(3), small.sample_vertices(3));
        assert_eq!(full.max_out_degree_vertex(), small.max_out_degree_vertex());
        let e = Label::from(EDGE_LABEL);
        assert_eq!(
            full.max_out_degree_vertex_over(EdgeFilter::Label(&e)),
            small.max_out_degree_vertex_over(EdgeFilter::Label(&e))
        );
        for id in ["a", "b", "d", "g", "h"] {
            let id = NodeId::from(id);
            assert_eq!(full.out_degree(&id), small.out_degree(&id), "{id:?}");
            for k in 1..5 {
                assert_eq!(
                    full.khop_layers(&id, k),
                    small.khop_layers(&id, k),
                    "{id:?} k={k}"
                );
                assert_eq!(
                    full.bfs_depth(&id, k),
                    small.bfs_depth(&id, k),
                    "{id:?} d={k}"
                );
            }
        }
        assert_eq!(small.out_degree(&NodeId::from("zz")), 0);
    }
}
