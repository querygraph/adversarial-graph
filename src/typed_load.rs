//! What a harness-native loader needs from a typed graph: nodes grouped by
//! label and edges grouped by (relationship type, from label, to label), so
//! each group is one `UNWIND` batch whose labels are literal in the Cypher
//! text (labels cannot be parameters). A SNAP graph is one group of each.

use std::collections::{BTreeMap, HashMap};

use grust::{Edge, Graph, Node};

/// The labels of an edge's endpoints and its relationship type.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct EdgeShape {
    pub relationship: String,
    pub from_label: String,
    pub to_label: String,
}

pub struct LoadPlan<'g> {
    nodes: BTreeMap<String, Vec<&'g Node>>,
    edges: BTreeMap<EdgeShape, Vec<&'g Edge>>,
}

impl<'g> LoadPlan<'g> {
    pub fn of(graph: &'g Graph) -> Self {
        let label_of: HashMap<&str, &str> = graph
            .nodes
            .iter()
            .map(|n| (n.id.as_str(), n.label.as_str()))
            .collect();
        let mut nodes: BTreeMap<String, Vec<&Node>> = BTreeMap::new();
        for node in &graph.nodes {
            nodes
                .entry(node.label.as_str().to_string())
                .or_default()
                .push(node);
        }
        let mut edges: BTreeMap<EdgeShape, Vec<&Edge>> = BTreeMap::new();
        for edge in &graph.edges {
            let (Some(from), Some(to)) = (
                label_of.get(edge.from.as_str()),
                label_of.get(edge.to.as_str()),
            ) else {
                continue;
            };
            let shape = EdgeShape {
                relationship: edge.label.as_str().to_string(),
                from_label: (*from).to_string(),
                to_label: (*to).to_string(),
            };
            edges.entry(shape).or_default().push(edge);
        }
        Self { nodes, edges }
    }

    pub fn node_labels(&self) -> impl Iterator<Item = &str> {
        self.nodes.keys().map(String::as_str)
    }

    pub fn nodes_by_label(&self) -> impl Iterator<Item = (&str, &[&'g Node])> {
        self.nodes
            .iter()
            .map(|(label, nodes)| (label.as_str(), nodes.as_slice()))
    }

    pub fn edges_by_shape(&self) -> impl Iterator<Item = (&EdgeShape, &[&'g Edge])> {
        self.edges
            .iter()
            .map(|(shape, edges)| (shape, edges.as_slice()))
    }
}

/// A stable, label-derived index name: `ag_id_person`.
pub fn index_name(label: &str) -> String {
    let mut name = String::from("ag_id_");
    name.extend(label.chars().map(|c| {
        if c.is_ascii_alphanumeric() {
            c.to_ascii_lowercase()
        } else {
            '_'
        }
    }));
    name
}

#[cfg(test)]
mod tests {
    use super::*;
    use grust::Props;

    #[test]
    fn groups_nodes_by_label_and_edges_by_shape() {
        let graph = Graph::new(
            vec![
                Node::new("Person", "p1", Props::new()),
                Node::new("Person", "p2", Props::new()),
                Node::new("Tag", "t1", Props::new()),
            ],
            vec![
                Edge::new("KNOWS", "p1", "p2", Props::new()),
                Edge::new("HAS_INTEREST", "p1", "t1", Props::new()),
                Edge::new("HAS_INTEREST", "p2", "t1", Props::new()),
            ],
        );
        let plan = LoadPlan::of(&graph);
        assert_eq!(plan.node_labels().collect::<Vec<_>>(), ["Person", "Tag"]);
        let shapes: Vec<(String, usize)> = plan
            .edges_by_shape()
            .map(|(s, e)| {
                (
                    format!("{}:{}->{}", s.relationship, s.from_label, s.to_label),
                    e.len(),
                )
            })
            .collect();
        assert_eq!(
            shapes,
            [
                ("HAS_INTEREST:Person->Tag".to_string(), 2),
                ("KNOWS:Person->Person".to_string(), 1)
            ]
        );
        assert_eq!(index_name("TagClass"), "ag_id_tagclass");
    }
}
