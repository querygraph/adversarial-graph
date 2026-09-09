//! Harness-native FalkorDB read path. `grust-falkor` 0.13 implements only
//! writes (`get_node`, `get_edges`, and `traverse` return `Unsupported`, and
//! its native-Cypher escape hatch discards results), so the harness reads the
//! graph back itself through `GRAPH.QUERY` openCypher. Results obtained this
//! way are labelled `read_path = "harness-native-cypher"` in the report so
//! they are never mistaken for Grust's portable API.

use std::sync::{Arc, Mutex};

use redis::Value;

use crate::differential::{Cell, ResultSet};

/// One persistent connection per reader, opened on first use and reopened
/// after an error, the way any client library keeps a session: a connection
/// per query exhausted the host's ephemeral ports after a few thousand
/// one-hop reads (`Can't assign requested address`), which is a harness
/// artifact and not a store finding. Clones share the connection;
/// [`FalkorReader::fresh`] gives a clone its own.
#[derive(Clone)]
pub struct FalkorReader {
    client: redis::Client,
    graph: String,
    conn: Arc<Mutex<Option<redis::Connection>>>,
}

fn escape(id: &str) -> String {
    id.replace('\\', "\\\\").replace('\'', "\\'")
}

/// A Grust value as a Cypher literal: ints, floats and bools as themselves,
/// strings quoted, string arrays as lists, dates and decimals as text.
fn literal(value: &grust::Value) -> Option<String> {
    Some(match value {
        grust::Value::Null => return None,
        grust::Value::Bool(b) => b.to_string(),
        grust::Value::Int(i) => i.to_string(),
        grust::Value::Float(f) => f.to_string(),
        grust::Value::String(s) => format!("'{}'", escape(s)),
        grust::Value::StringArray(items) => format!(
            "[{}]",
            items
                .iter()
                .map(|s| format!("'{}'", escape(s)))
                .collect::<Vec<_>>()
                .join(",")
        ),
        other => format!("'{}'", escape(&crate::differential::value_text(other))),
    })
}

fn props_literal(props: &grust::Props) -> String {
    let fields: Vec<String> = props
        .iter()
        .filter_map(|(k, v)| literal(v).map(|lit| format!("`{}`: {lit}", k.replace('`', "``"))))
        .collect();
    format!("{{{}}}", fields.join(", "))
}

/// A compact-mode cell (`[type, value]`) as a comparison cell. FalkorDB's
/// compact types: 1 null, 2 string, 3 integer, 4 boolean, 5 double, 6 array;
/// nodes, edges, paths and maps are reported as their text.
fn compact_cell(cell: &Value) -> Cell {
    let (kind, inner) = match cell {
        Value::Array(typed) if typed.len() == 2 => (
            match &typed[0] {
                Value::Int(k) => *k,
                _ => 0,
            },
            &typed[1],
        ),
        other => (0, other),
    };
    match (kind, inner) {
        (1, _) | (_, Value::Nil) => Cell::Null,
        (3, Value::Int(i)) => Cell::Int(*i),
        (3, other) => value_to_string(other)
            .and_then(|s| s.parse().ok())
            .map(Cell::Int)
            .unwrap_or(Cell::Null),
        (4, other) => Cell::Bool(value_to_string(other).is_some_and(|s| s == "true")),
        (5, other) => value_to_string(other)
            .and_then(|s| s.parse().ok())
            .map(Cell::Float)
            .unwrap_or(Cell::Null),
        (6, Value::Array(items)) => Cell::List(items.iter().map(compact_cell).collect()),
        (_, Value::Int(i)) => Cell::Int(*i),
        (_, other) => value_to_string(other)
            .map(Cell::Str)
            .unwrap_or_else(|| Cell::Str(format!("{other:?}"))),
    }
}

fn value_to_string(value: &Value) -> Option<String> {
    match value {
        Value::BulkString(bytes) => Some(String::from_utf8_lossy(bytes).to_string()),
        Value::SimpleString(s) | Value::VerbatimString { text: s, .. } => Some(s.clone()),
        Value::Int(i) => Some(i.to_string()),
        Value::Double(d) => Some(d.to_string()),
        Value::Array(items) if items.len() == 1 => value_to_string(&items[0]),
        _ => None,
    }
}

impl FalkorReader {
    pub fn new(redis_url: &str, graph: &str) -> grust::Result<Self> {
        let client = redis::Client::open(redis_url)
            .map_err(|e| grust::GrustError::Backend(format!("falkor client: {e}")))?;
        Ok(Self {
            client,
            graph: graph.to_string(),
            conn: Arc::new(Mutex::new(None)),
        })
    }

    /// The same client and graph with a connection of its own.
    pub fn fresh(&self) -> Self {
        Self {
            client: self.client.clone(),
            graph: self.graph.clone(),
            conn: Arc::new(Mutex::new(None)),
        }
    }

    /// Run one command on this reader's connection; a failed connection is
    /// dropped so the next call reconnects.
    fn query(&self, command: &str, cypher: &str) -> grust::Result<Value> {
        self.query_within(command, cypher, None)
    }

    /// `GRAPH.RO_QUERY … TIMEOUT <ms>`: FalkorDB stops the query itself at
    /// the deadline and answers "Query timed out", so the next query never
    /// queues behind one the harness has stopped waiting for.
    fn query_within(
        &self,
        command: &str,
        cypher: &str,
        deadline: Option<std::time::Duration>,
    ) -> grust::Result<Value> {
        let mut slot = self.conn.lock().expect("falkor connection slot");
        if slot.is_none() {
            *slot = Some(
                self.client
                    .get_connection()
                    .map_err(|e| grust::GrustError::Backend(format!("falkor connect: {e}")))?,
            );
        }
        let conn = slot.as_mut().expect("connection present");
        let mut cmd = redis::cmd(command);
        cmd.arg(&self.graph).arg(cypher).arg("--compact");
        if let Some(deadline) = deadline {
            cmd.arg("TIMEOUT").arg(deadline.as_millis() as u64);
        }
        let result: Result<Value, redis::RedisError> = cmd.query(conn);
        match result {
            Ok(value) => Ok(value),
            Err(e) => {
                *slot = None;
                Err(grust::GrustError::Backend(format!("falkor {command}: {e}")))
            }
        }
    }

    /// Delete one vertex, whatever its label, with every edge incident to
    /// it, through `GRAPH.QUERY`: one statement, one transaction.
    pub fn delete_node(&self, id: &str) -> grust::Result<()> {
        self.query(
            "GRAPH.QUERY",
            &format!("MATCH (n {{id: '{}'}}) DETACH DELETE n", escape(id)),
        )
        .map(|_| ())
    }

    /// Every row of a read-only query through `GRAPH.RO_QUERY`.
    pub fn rows(&self, cypher: &str) -> grust::Result<ResultSet> {
        let value = self.query_within(
            "GRAPH.RO_QUERY",
            cypher,
            Some(crate::differential::STORE_BUDGET),
        )?;
        let Value::Array(parts) = value else {
            return Err(grust::GrustError::Backend(
                "falkor: unexpected result shape".into(),
            ));
        };
        // Compact header: [[type, name], …]
        let columns = match parts.first() {
            Some(Value::Array(header)) => header
                .iter()
                .filter_map(|column| match column {
                    Value::Array(pair) if pair.len() == 2 => value_to_string(&pair[1]),
                    other => value_to_string(other),
                })
                .collect(),
            _ => Vec::new(),
        };
        let rows = match parts.get(1) {
            Some(Value::Array(rows)) => rows
                .iter()
                .filter_map(|row| match row {
                    Value::Array(cells) => Some(cells.iter().map(compact_cell).collect()),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };
        Ok(ResultSet { columns, rows })
    }

    /// First column of every result row, as strings.
    pub fn column(&self, cypher: &str) -> grust::Result<Vec<String>> {
        let value = self.query("GRAPH.QUERY", cypher)?;
        // Result shape: [header, rows, statistics]; each row is an array of
        // cells, each cell (compact) is [type, value].
        let Value::Array(parts) = value else {
            return Err(grust::GrustError::Backend(
                "falkor: unexpected result shape".into(),
            ));
        };
        let Some(Value::Array(rows)) = parts.get(1) else {
            return Ok(Vec::new());
        };
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let Value::Array(cells) = row else { continue };
            let Some(cell) = cells.first() else { continue };
            let inner = match cell {
                Value::Array(typed) if typed.len() == 2 => &typed[1],
                other => other,
            };
            if let Some(s) = value_to_string(inner) {
                out.push(s);
            }
        }
        Ok(out)
    }

    /// `grust-falkor` lowercases node labels through `schema_identifier`
    /// (`V` is stored as `v`) and keeps relationship types as given; the
    /// SNAP label follows it so the adapter's own writes and this reader
    /// meet on one label. Typed labels are stored as loaded, so Cypher
    /// written for the dataset (`:Person`) matches them.
    fn label(node_label: &str) -> String {
        if node_label == crate::dataset::NODE_LABEL {
            node_label.to_ascii_lowercase()
        } else {
            node_label.to_string()
        }
    }

    /// The Grust adapter only creates its id index inside `apply_schema`,
    /// which `put_graph` never calls; without it every edge write scans all
    /// nodes. Create it up front so the engine, not the missing index, is
    /// what gets measured. Idempotent: an existing index is not an error.
    pub fn ensure_index(&self, node_label: &str) -> grust::Result<()> {
        let label = Self::label(node_label);
        match self.column(&format!("CREATE INDEX FOR (n:{label}) ON (n.id)")) {
            Ok(_) => Ok(()),
            Err(e) if e.to_string().contains("already indexed") => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// Bulk load through labelled, indexed, `UNWIND`-batched Cypher. The
    /// Grust adapter's edge batch matches endpoints *without* a label
    /// (`MATCH (a {id: …})`), which FalkorDB cannot serve from its per-label
    /// index, so its loads scan every node per edge; this path is what a
    /// FalkorDB user would write, and it is recorded as
    /// `load_path = "harness-native-cypher"`. A typed graph loads one batch
    /// per label and per (type, from label, to label), with properties as
    /// literal maps and an id index per label.
    pub fn load_graph(&self, graph: &grust::Graph) -> grust::Result<grust::LoadReport> {
        let plan = crate::typed_load::LoadPlan::of(graph);
        let mut report = grust::LoadReport::default();
        for label in plan.node_labels() {
            self.ensure_index(label)?;
        }
        for (label, nodes) in plan.nodes_by_label() {
            let label = Self::label(label);
            for chunk in nodes.chunks(5_000) {
                let maps: Vec<String> = chunk.iter().map(|n| props_literal(&n.props)).collect();
                self.column(&format!(
                    "UNWIND [{}] AS r CREATE (n:{label}) SET n = r",
                    maps.join(",")
                ))?;
                report.nodes += chunk.len();
            }
        }
        for (shape, edges) in plan.edges_by_shape() {
            let (from_label, to_label) =
                (Self::label(&shape.from_label), Self::label(&shape.to_label));
            for chunk in edges.chunks(2_000) {
                let rows: Vec<String> = chunk
                    .iter()
                    .map(|e| {
                        format!(
                            "['{}','{}',{}]",
                            escape(e.from.as_str()),
                            escape(e.to.as_str()),
                            props_literal(&e.props)
                        )
                    })
                    .collect();
                self.column(&format!(
                    "UNWIND [{}] AS p MATCH (a:{from_label} {{id: p[0]}}), (b:{to_label} {{id: p[1]}}) CREATE (a)-[e:{}]->(b) SET e = p[2]",
                    rows.join(","),
                    shape.relationship
                ))?;
                report.edges += chunk.len();
            }
        }
        Ok(report)
    }

    pub fn out_neighbors(
        &self,
        node_label: &str,
        edge_label: &str,
        id: &str,
    ) -> grust::Result<Vec<String>> {
        let label = Self::label(node_label);
        self.column(&format!(
            "MATCH (a:{label} {{id: '{}'}})-[:{edge_label}]->(b) RETURN b.id",
            escape(id)
        ))
    }

    pub fn out_degree(&self, node_label: &str, edge_label: &str, id: &str) -> grust::Result<usize> {
        let label = Self::label(node_label);
        let counts = self.column(&format!(
            "MATCH (a:{label} {{id: '{}'}})-[r:{edge_label}]->() RETURN count(r)",
            escape(id)
        ))?;
        Ok(counts
            .first()
            .and_then(|c| c.parse::<usize>().ok())
            .unwrap_or(0))
    }
}
