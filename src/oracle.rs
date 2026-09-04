//! Ground truth computed from the loaded graph with Grust's in-memory
//! `GraphIndex`. Every structural scenario compares a backend's answer with
//! these numbers; a mismatch is a `wrong_answer` hard-gate failure.

use std::collections::{HashSet, VecDeque};

use grust::{Graph, GraphIndex, NodeId};

pub struct Oracle<'g> {
    pub graph: &'g Graph,
    pub index: GraphIndex,
}

impl<'g> Oracle<'g> {
    pub fn new(graph: &'g Graph) -> grust::Result<Self> {
        Ok(Self { graph, index: GraphIndex::new(graph)? })
    }

    /// Vertex with the largest out-degree (ties broken by id order, which is
    /// stable because loaders sort node ids).
    pub fn max_out_degree_vertex(&self) -> (NodeId, usize) {
        let mut best = (0usize, 0usize);
        for index in 0..self.graph.nodes.len() {
            let degree = self.index.out_degree(index);
            if degree > best.1 {
                best = (index, degree);
            }
        }
        (self.graph.nodes[best.0].id.clone(), best.1)
    }

    /// Distinct vertices reachable in exactly 1..=k out-hops, per layer, and
    /// the distinct union (excluding the start vertex itself).
    pub fn khop_layers(&self, start: &NodeId, k: usize) -> (Vec<usize>, usize) {
        let start_index = self.index.require_vertex_index(start).expect("start vertex present");
        let mut visited: HashSet<usize> = HashSet::new();
        visited.insert(start_index);
        let mut frontier = vec![start_index];
        let mut layers = Vec::with_capacity(k);
        for _ in 0..k {
            let mut next = Vec::new();
            for v in &frontier {
                for &e in self.index.outgoing_by_vertex(*v) {
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
        let start_index = self.index.require_vertex_index(start).expect("start vertex present");
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
