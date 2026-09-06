//! Ground truth computed from the loaded graph with Grust's in-memory
//! `GraphIndex`. Every structural scenario compares a backend's answer with
//! these numbers; a mismatch is a `wrong_answer` hard-gate failure.

use std::collections::{BTreeMap, HashSet, VecDeque};

use grust::{Graph, GraphIndex, Label, NodeId};

use crate::dataset::DatasetSchema;

pub struct Oracle<'g> {
    pub graph: &'g Graph,
    pub index: GraphIndex,
    /// Labels and relationship types with their counts, as the loader
    /// produced them; one of each for a SNAP edge list.
    pub schema: DatasetSchema,
}

/// Which edges a traversal follows: every edge, or one relationship type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeFilter<'a> {
    Any,
    Label(&'a Label),
}

impl<'g> Oracle<'g> {
    #[allow(dead_code)]
    pub fn new(graph: &'g Graph) -> grust::Result<Self> {
        Ok(Self {
            graph,
            index: GraphIndex::new(graph)?,
            schema: DatasetSchema::of(graph),
        })
    }

    pub fn with_schema(graph: &'g Graph, schema: DatasetSchema) -> grust::Result<Self> {
        Ok(Self {
            graph,
            index: GraphIndex::new(graph)?,
            schema,
        })
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
        match filter {
            EdgeFilter::Any => true,
            EdgeFilter::Label(label) => &self.graph.edges[edge].label == label,
        }
    }

    /// Vertex with the largest out-degree (ties broken by id order, which is
    /// stable because loaders sort node ids).
    pub fn max_out_degree_vertex(&self) -> (NodeId, usize) {
        self.max_out_degree_vertex_over(EdgeFilter::Any)
    }

    /// The same, counting only the edges `filter` admits; an optional node
    /// label restricts the candidates.
    pub fn max_out_degree_vertex_over(&self, filter: EdgeFilter<'_>) -> (NodeId, usize) {
        let mut best = (0usize, 0usize);
        for index in 0..self.graph.nodes.len() {
            let degree = match filter {
                EdgeFilter::Any => self.index.out_degree(index),
                EdgeFilter::Label(_) => self
                    .index
                    .outgoing_by_vertex(index)
                    .iter()
                    .filter(|&&e| self.follows(e, filter))
                    .count(),
            };
            if degree > best.1 {
                best = (index, degree);
            }
        }
        (self.graph.nodes[best.0].id.clone(), best.1)
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
        let start_index = self
            .index
            .require_vertex_index(start)
            .expect("start vertex present");
        let mut visited: HashSet<usize> = HashSet::new();
        visited.insert(start_index);
        let mut frontier = vec![start_index];
        let mut layers = Vec::with_capacity(k);
        for _ in 0..k {
            let mut next = Vec::new();
            for v in &frontier {
                for &e in self.index.outgoing_by_vertex(*v) {
                    if !self.follows(e, filter) {
                        continue;
                    }
                    let (_, to) = self.index.edge_endpoints(e);
                    if visited.insert(to) {
                        next.push(to);
                    }
                }
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
        let start_index = self
            .index
            .require_vertex_index(start)
            .expect("start vertex present");
        let mut visited = vec![false; self.graph.nodes.len()];
        visited[start_index] = true;
        let mut queue = VecDeque::from([(start_index, 0usize)]);
        let mut reached = 0usize;
        let mut deepest = 0usize;
        while let Some((v, d)) = queue.pop_front() {
            if d >= max_depth {
                continue;
            }
            for &e in self.index.outgoing_by_vertex(v) {
                if !self.follows(e, filter) {
                    continue;
                }
                let (_, to) = self.index.edge_endpoints(e);
                if !visited[to] {
                    visited[to] = true;
                    reached += 1;
                    deepest = deepest.max(d + 1);
                    queue.push_back((to, d + 1));
                }
            }
        }
        (reached, deepest)
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
}
