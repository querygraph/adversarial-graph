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

/// A property value as an engine's JSON or JSON-like text reports it, back
/// to the harness's `Value`: what a typed vertex round-trips through a
/// `properties(n)` read.
pub fn value_from_json(value: &serde_json::Value) -> grust::Value {
    match value {
        serde_json::Value::Null => grust::Value::Null,
        serde_json::Value::Bool(b) => grust::Value::Bool(*b),
        serde_json::Value::Number(n) => match n.as_i64() {
            Some(i) => grust::Value::Int(i),
            None => grust::Value::Float(n.as_f64().unwrap_or(f64::NAN)),
        },
        serde_json::Value::String(s) => grust::Value::String(s.clone()),
        serde_json::Value::Array(items) => grust::Value::StringArray(
            items
                .iter()
                .map(|item| match item {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                })
                .collect(),
        ),
        serde_json::Value::Object(_) => grust::Value::String(value.to_string()),
    }
}

/// A `properties(n)` map as `Props`.
pub fn props_from_json(value: &serde_json::Value) -> grust::Props {
    value
        .as_object()
        .map(|map| {
            map.iter()
                .map(|(k, v)| (k.clone(), value_from_json(v)))
                .collect()
        })
        .unwrap_or_default()
}

/// The label a typed vertex carries, from the engine's label list: the
/// first one that is not the untyped `V`, else `V`.
pub fn label_from_list<'a>(labels: impl IntoIterator<Item = &'a str>) -> String {
    let mut fallback = None;
    for label in labels {
        if label == crate::dataset::NODE_LABEL {
            fallback = Some(label);
        } else {
            return label.to_string();
        }
    }
    fallback.unwrap_or(crate::dataset::NODE_LABEL).to_string()
}
