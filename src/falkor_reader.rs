//! Harness-native FalkorDB read path. `grust-falkor` 0.13 implements only
//! writes (`get_node`, `get_edges`, and `traverse` return `Unsupported`, and
//! its native-Cypher escape hatch discards results), so the harness reads the
//! graph back itself through `GRAPH.QUERY` openCypher. Results obtained this
//! way are labelled `read_path = "harness-native-cypher"` in the report so
//! they are never mistaken for Grust's portable API.

use redis::Value;

#[derive(Clone)]
pub struct FalkorReader {
    client: redis::Client,
    graph: String,
}

fn escape(id: &str) -> String {
    id.replace('\\', "\\\\").replace('\'', "\\'")
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
        Ok(Self { client, graph: graph.to_string() })
    }

    /// First column of every result row, as strings.
    pub fn column(&self, cypher: &str) -> grust::Result<Vec<String>> {
        let mut conn = self
            .client
            .get_connection()
            .map_err(|e| grust::GrustError::Backend(format!("falkor connect: {e}")))?;
        let value: Value = redis::cmd("GRAPH.QUERY")
            .arg(&self.graph)
            .arg(cypher)
            .arg("--compact")
            .query(&mut conn)
            .map_err(|e| grust::GrustError::Backend(format!("falkor GRAPH.QUERY: {e}")))?;
        // Result shape: [header, rows, statistics]; each row is an array of
        // cells, each cell (compact) is [type, value].
        let Value::Array(parts) = value else {
            return Err(grust::GrustError::Backend("falkor: unexpected result shape".into()));
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

    pub fn out_neighbors(&self, node_label: &str, edge_label: &str, id: &str) -> grust::Result<Vec<String>> {
        self.column(&format!(
            "MATCH (a:{node_label} {{id: '{}'}})-[:{edge_label}]->(b) RETURN b.id",
            escape(id)
        ))
    }

    pub fn out_degree(&self, node_label: &str, edge_label: &str, id: &str) -> grust::Result<usize> {
        let counts = self.column(&format!(
            "MATCH (a:{node_label} {{id: '{}'}})-[r:{edge_label}]->() RETURN count(r)",
            escape(id)
        ))?;
        Ok(counts.first().and_then(|c| c.parse::<usize>().ok()).unwrap_or(0))
    }
}
