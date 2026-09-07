//! What the typed loaders share: a label-aware graph builder with the same
//! duplicate and truncation rules as the SNAP loader, and the schema summary
//! the report and the oracle carry.

use std::collections::{BTreeMap, HashMap, HashSet};

use grust::{Edge, Graph, Node, Props, Value};

use super::LoadStats;

/// Node and relationship labels with their counts, and the relationship
/// type with the most edges: the one A1/A2-style traversals follow when a
/// dataset has more than one.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct DatasetSchema {
    pub node_labels: BTreeMap<String, usize>,
    pub relationship_labels: BTreeMap<String, usize>,
    pub dominant_relationship: Option<String>,
}

impl DatasetSchema {
    pub fn of(graph: &Graph) -> Self {
        let mut schema = Self::default();
        for node in &graph.nodes {
            *schema
                .node_labels
                .entry(node.label.as_str().to_string())
                .or_default() += 1;
        }
        for edge in &graph.edges {
            *schema
                .relationship_labels
                .entry(edge.label.as_str().to_string())
                .or_default() += 1;
        }
        schema.dominant_relationship = schema
            .relationship_labels
            .iter()
            .max_by(|a, b| a.1.cmp(b.1).then_with(|| b.0.cmp(a.0)))
            .map(|(label, _)| label.clone());
        schema
    }

    /// One node label and one relationship type: the SNAP shape.
    pub fn is_untyped(&self) -> bool {
        self.node_labels.len() <= 1 && self.relationship_labels.len() <= 1
    }
}

/// Accumulates typed nodes and edges under the SNAP loader's rules: an edge
/// whose endpoints are unknown is dropped and counted, an exact duplicate
/// `(label, from, to)` is dropped and counted, a self-loop is kept and
/// counted, and `limit` truncates after that many edges. When truncated,
/// only nodes an accepted edge references survive, as in an edge list.
pub struct TypedGraphBuilder {
    nodes: Vec<Node>,
    node_index: HashMap<String, usize>,
    edges: Vec<Edge>,
    seen: HashSet<(String, String, String)>,
    limit: Option<usize>,
    pub lines: usize,
    pub duplicates: usize,
    pub self_loops: usize,
    pub dangling: usize,
    pub truncated_at: Option<usize>,
}

impl TypedGraphBuilder {
    pub fn new(limit: Option<usize>) -> Self {
        Self {
            nodes: Vec::new(),
            node_index: HashMap::new(),
            edges: Vec::new(),
            seen: HashSet::new(),
            limit,
            lines: 0,
            duplicates: 0,
            self_loops: 0,
            dangling: 0,
            truncated_at: None,
        }
    }

    /// Whether the edge limit has been reached; loaders stop reading then.
    pub fn full(&self) -> bool {
        self.truncated_at.is_some()
    }

    pub fn has_node(&self, id: &str) -> bool {
        self.node_index.contains_key(id)
    }

    /// Adds a node; a repeated id keeps the first definition and is counted
    /// as a duplicate.
    pub fn node(&mut self, label: &str, id: String, props: Props) {
        if self.node_index.contains_key(&id) {
            self.duplicates += 1;
            return;
        }
        self.node_index.insert(id.clone(), self.nodes.len());
        self.nodes.push(Node::new(label, id, props));
    }

    /// Adds an edge under the rules above; returns whether it was kept.
    pub fn edge(&mut self, label: &str, from: &str, to: &str, props: Props) -> bool {
        if self.full() {
            return false;
        }
        if !self.node_index.contains_key(from) || !self.node_index.contains_key(to) {
            self.dangling += 1;
            return false;
        }
        if from == to {
            self.self_loops += 1;
        }
        if !self
            .seen
            .insert((label.to_string(), from.to_string(), to.to_string()))
        {
            self.duplicates += 1;
            return false;
        }
        self.edges.push(Edge::new(label, from, to, props));
        if self.limit.is_some_and(|max| self.edges.len() >= max) {
            self.truncated_at = self.limit;
        }
        true
    }

    pub fn finish(self, file: String, format: &str) -> (Graph, LoadStats, DatasetSchema) {
        let nodes = if self.truncated_at.is_some() {
            let referenced: HashSet<&str> = self
                .edges
                .iter()
                .flat_map(|edge| [edge.from.as_str(), edge.to.as_str()])
                .collect();
            self.nodes
                .into_iter()
                .filter(|node| referenced.contains(node.id.as_str()))
                .collect()
        } else {
            self.nodes
        };
        let graph = Graph::new(nodes, self.edges);
        let schema = DatasetSchema::of(&graph);
        let stats = LoadStats {
            file,
            format: format.to_string(),
            lines: self.lines,
            nodes: graph.nodes.len(),
            edges: graph.edges.len(),
            duplicate_edges_dropped: self.duplicates,
            dangling_edges_dropped: self.dangling,
            self_loops: self.self_loops,
            truncated_at: self.truncated_at,
            node_labels: schema.node_labels.clone(),
            relationship_labels: schema.relationship_labels.clone(),
        };
        (graph, stats, schema)
    }
}

/// A CSV cell as a typed property value: empty cells are absent, integers
/// are `Int`, and a millisecond epoch in a date column is an RFC 3339
/// `DateTime`; everything else stays a string.
pub fn typed_value(column: &str, cell: &str, epoch_millis_columns: &[&str]) -> Option<Value> {
    if cell.is_empty() {
        return None;
    }
    if epoch_millis_columns.contains(&column)
        && let Some(datetime) = cell
            .parse::<i64>()
            .ok()
            .and_then(chrono::DateTime::from_timestamp_millis)
    {
        return Value::datetime(datetime.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)).ok();
    }
    if let Ok(int) = cell.parse::<i64>() {
        return Some(Value::Int(int));
    }
    Some(Value::String(cell.to_string()))
}

/// `hasCreator` → `HAS_CREATOR`, `officer_of` → `OFFICER_OF`.
pub fn relationship_label(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    let mut previous_lower = false;
    for c in name.chars() {
        if c.is_ascii_uppercase() && previous_lower {
            out.push('_');
        }
        if c == ' ' || c == '-' {
            out.push('_');
            previous_lower = false;
            continue;
        }
        previous_lower = c.is_ascii_lowercase() || c.is_ascii_digit();
        out.push(c.to_ascii_uppercase());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relationship_labels_are_upper_snake() {
        assert_eq!(relationship_label("hasCreator"), "HAS_CREATOR");
        assert_eq!(relationship_label("knows"), "KNOWS");
        assert_eq!(relationship_label("officer_of"), "OFFICER_OF");
        assert_eq!(
            relationship_label("registered address"),
            "REGISTERED_ADDRESS"
        );
        assert_eq!(relationship_label("isPartOf"), "IS_PART_OF");
    }

    #[test]
    fn values_are_typed_by_column_and_shape() {
        assert_eq!(typed_value("length", "", &[]), None);
        assert_eq!(typed_value("length", "140", &[]), Some(Value::Int(140)));
        assert_eq!(
            typed_value("name", "Kam_Air", &[]),
            Some(Value::String("Kam_Air".into()))
        );
        let date = typed_value("creationDate", "1266161530447", &["creationDate"]).unwrap();
        assert!(matches!(date, Value::DateTime(_)), "{date:?}");
        assert_eq!(
            typed_value("creationDate", "not a date", &["creationDate"]),
            Some(Value::String("not a date".into()))
        );
    }

    #[test]
    fn builder_applies_the_edge_list_rules_and_truncation() {
        let mut b = TypedGraphBuilder::new(Some(2));
        b.node("Person", "p1".into(), Props::new());
        b.node("Person", "p2".into(), Props::new());
        b.node("Person", "p3".into(), Props::new());
        b.node("Person", "p1".into(), Props::new());
        b.node("Tag", "t1".into(), Props::new());
        assert!(b.edge("KNOWS", "p1", "p2", Props::new()));
        assert!(!b.edge("KNOWS", "p1", "p2", Props::new()), "duplicate");
        assert!(!b.edge("KNOWS", "p1", "nobody", Props::new()), "dangling");
        assert!(b.edge("KNOWS", "p2", "p2", Props::new()), "self-loop kept");
        assert!(b.full());
        assert!(!b.edge("KNOWS", "p2", "p3", Props::new()), "past the limit");
        let (graph, stats, schema) = b.finish("x".into(), "test");
        assert_eq!(stats.nodes, 2, "p3 and t1 are unreferenced once truncated");
        assert_eq!(stats.edges, 2);
        assert_eq!(
            (
                stats.duplicate_edges_dropped,
                stats.dangling_edges_dropped,
                stats.self_loops
            ),
            (2, 1, 1)
        );
        assert_eq!(stats.truncated_at, Some(2));
        assert_eq!(schema.node_labels["Person"], 2);
        assert_eq!(schema.dominant_relationship.as_deref(), Some("KNOWS"));
        assert!(!schema.is_untyped() || schema.node_labels.len() == 1);
        assert_eq!(graph.edges.len(), 2);
    }
}
