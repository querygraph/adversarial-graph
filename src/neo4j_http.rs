//! Neo4j over its HTTP Query API (`POST /db/{db}/query/v2`), the transport
//! counterpart of the Bolt store in `neo4j.rs`. Same Cypher, same labels,
//! same batching; only the wire differs, so the pair isolates driver and
//! protocol cost. Reports through this adapter carry
//! `read_path = "harness-native-cypher"`.

use async_trait::async_trait;
use grust::{
    Edge, EdgeQuery, Graph, GraphAdminStore, GraphStore, GrustError, LoadReport, Node, NodeId,
    Props, PutOutcome, Traversal,
};
use serde_json::{Value, json};

use crate::dataset::{EDGE_LABEL, NODE_LABEL};
use crate::differential::{Cell, ResultSet};

const BATCH: usize = 5_000;

#[derive(Clone)]
pub struct Neo4jHttpStore {
    client: reqwest::Client,
    url: String,
    user: String,
    pass: String,
}

fn backend(err: impl std::fmt::Display) -> GrustError {
    GrustError::Backend(format!("neo4j-http: {err}"))
}

/// Grust properties as JSON for the Query API: ints, floats, bools and
/// strings as themselves, string arrays as lists, dates as their text.
fn json_props(props: &Props) -> Value {
    let mut map = serde_json::Map::new();
    for (key, value) in props.iter() {
        let cell = match value {
            grust::Value::Null => continue,
            grust::Value::Bool(b) => json!(b),
            grust::Value::Int(i) => json!(i),
            grust::Value::Float(f) => json!(f),
            grust::Value::String(s) => json!(s),
            grust::Value::StringArray(items) => json!(items),
            other => json!(crate::differential::value_text(other)),
        };
        map.insert(key.clone(), cell);
    }
    Value::Object(map)
}

impl Neo4jHttpStore {
    pub fn connect(base_url: &str, user: &str, pass: &str) -> grust::Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(600))
            .build()
            .map_err(backend)?;
        let db = std::env::var("AG_NEO4J_DB").unwrap_or_else(|_| "neo4j".into());
        Ok(Self {
            client,
            url: format!("{}/db/{db}/query/v2", base_url.trim_end_matches('/')),
            user: user.to_string(),
            pass: pass.to_string(),
        })
    }

    /// Every row of a read query, cells normalized for comparison.
    /// Delete one vertex, whatever its label, with every edge incident to
    /// it, through the Query API: one statement, one transaction.
    pub async fn delete_node(&self, id: &NodeId) -> grust::Result<()> {
        self.query(
            "MATCH (n {id: $id}) DETACH DELETE n".to_string(),
            json!({ "id": id.as_str() }),
        )
        .await
        .map(|_| ())
    }

    pub async fn rows(&self, cypher: &str) -> grust::Result<ResultSet> {
        let body = json!({ "statement": cypher, "parameters": {} });
        let response = self
            .client
            .post(&self.url)
            .basic_auth(&self.user, Some(&self.pass))
            .json(&body)
            .send()
            .await
            .map_err(backend)?;
        let status = response.status();
        let text = response.text().await.map_err(backend)?;
        let parsed: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        if let Some(errors) = parsed.get("errors").and_then(Value::as_array)
            && let Some(first) = errors.first()
        {
            return Err(backend(format!(
                "{} {}",
                first.get("code").and_then(Value::as_str).unwrap_or("?"),
                first.get("message").and_then(Value::as_str).unwrap_or("")
            )));
        }
        if !status.is_success() {
            return Err(backend(format!(
                "HTTP {status}: {}",
                text.chars().take(200).collect::<String>()
            )));
        }
        let columns = parsed
            .pointer("/data/fields")
            .and_then(Value::as_array)
            .map(|f| {
                f.iter()
                    .filter_map(|c| c.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let rows = parsed
            .pointer("/data/values")
            .and_then(Value::as_array)
            .map(|rows| {
                rows.iter()
                    .filter_map(|r| r.as_array())
                    .map(|r| r.iter().map(Cell::from_json).collect())
                    .collect()
            })
            .unwrap_or_default();
        Ok(ResultSet { columns, rows })
    }

    /// Run one statement; returns the result rows (`data.values`).
    async fn query(&self, statement: String, parameters: Value) -> grust::Result<Vec<Vec<Value>>> {
        let body = json!({ "statement": statement, "parameters": parameters });
        let response = self
            .client
            .post(&self.url)
            .basic_auth(&self.user, Some(&self.pass))
            .json(&body)
            .send()
            .await
            .map_err(backend)?;
        let status = response.status();
        let text = response.text().await.map_err(backend)?;
        let parsed: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        if let Some(errors) = parsed.get("errors").and_then(Value::as_array)
            && let Some(first) = errors.first()
        {
            return Err(backend(format!(
                "{} {}",
                first.get("code").and_then(Value::as_str).unwrap_or("?"),
                first.get("message").and_then(Value::as_str).unwrap_or("")
            )));
        }
        if !status.is_success() {
            return Err(backend(format!(
                "HTTP {status}: {}",
                text.chars().take(200).collect::<String>()
            )));
        }
        Ok(parsed
            .pointer("/data/values")
            .and_then(Value::as_array)
            .map(|rows| rows.iter().filter_map(|r| r.as_array().cloned()).collect())
            .unwrap_or_default())
    }

    async fn column(&self, statement: String, parameters: Value) -> grust::Result<Vec<String>> {
        Ok(self
            .query(statement, parameters)
            .await?
            .into_iter()
            .filter_map(|row| match row.into_iter().next()? {
                Value::String(s) => Some(s),
                Value::Number(n) => Some(n.to_string()),
                _ => None,
            })
            .collect())
    }
}

#[async_trait]
impl GraphStore for Neo4jHttpStore {
    async fn put_node(&self, node: &Node) -> grust::Result<PutOutcome> {
        // Label-preserving upsert with the properties as one map (see the
        // Bolt adapter).
        self.query(
            format!(
                "MERGE (n:`{}` {{id: $id}}) SET n += $props",
                node.label.as_str().replace('`', "``")
            ),
            json!({ "id": node.id.as_str(), "props": json_props(&node.props) }),
        )
        .await?;
        Ok(PutOutcome::Upserted)
    }

    async fn put_edge(&self, edge: &Edge) -> grust::Result<PutOutcome> {
        self.query(
            format!(
                "MATCH (a:{NODE_LABEL} {{id: $from}}), (b:{NODE_LABEL} {{id: $to}}) CREATE (a)-[:{EDGE_LABEL}]->(b)"
            ),
            json!({ "from": edge.from.as_str(), "to": edge.to.as_str() }),
        )
        .await?;
        Ok(PutOutcome::Upserted)
    }

    async fn put_graph(&self, graph: &Graph) -> grust::Result<LoadReport> {
        let mut report = LoadReport::default();
        let plan = crate::typed_load::LoadPlan::of(graph);
        for label in plan.node_labels() {
            self.query(
                format!(
                    "CREATE INDEX {} IF NOT EXISTS FOR (n:{label}) ON (n.id)",
                    crate::typed_load::index_name(label)
                ),
                json!({}),
            )
            .await?;
        }
        self.query("CALL db.awaitIndexes(600)".to_string(), json!({}))
            .await?;
        for (label, nodes) in plan.nodes_by_label() {
            for chunk in nodes.chunks(BATCH) {
                let rows: Vec<Value> = chunk.iter().map(|n| json_props(&n.props)).collect();
                self.query(
                    format!("UNWIND $rows AS r CREATE (n:{label}) SET n = r"),
                    json!({ "rows": rows }),
                )
                .await?;
                report.nodes += chunk.len();
            }
        }
        for (key, edges) in plan.edges_by_shape() {
            for chunk in edges.chunks(BATCH) {
                let rows: Vec<Value> = chunk
                    .iter()
                    .map(|e| json!({ "from": e.from.as_str(), "to": e.to.as_str(), "props": json_props(&e.props) }))
                    .collect();
                self.query(
                    format!(
                        "UNWIND $rows AS r MATCH (a:{} {{id: r.from}}), (b:{} {{id: r.to}}) \
                         CREATE (a)-[e:{}]->(b) SET e = r.props",
                        key.from_label, key.to_label, key.relationship
                    ),
                    json!({ "rows": rows }),
                )
                .await?;
                report.edges += chunk.len();
            }
        }
        Ok(report)
    }

    async fn get_node(&self, id: &NodeId) -> grust::Result<Option<Node>> {
        // Whatever the label, with every property (see the Bolt adapter).
        let rows = self
            .query(
                "MATCH (n {id: $id}) RETURN labels(n), properties(n) LIMIT 1".to_string(),
                json!({ "id": id.as_str() }),
            )
            .await?;
        let Some(row) = rows.into_iter().next() else {
            return Ok(None);
        };
        let labels: Vec<String> = row
            .first()
            .and_then(|v| v.as_array())
            .map(|items| items.iter().filter_map(|l| l.as_str().map(String::from)).collect())
            .unwrap_or_default();
        let label = crate::typed_load::label_from_list(labels.iter().map(String::as_str));
        let props = row
            .get(1)
            .map(crate::typed_load::props_from_json)
            .unwrap_or_default();
        Ok(Some(Node::new(label, id.as_str(), props)))
    }

    async fn get_edges(&self, q: EdgeQuery) -> grust::Result<Vec<Edge>> {
        let Some(from) = q.from else {
            return Err(GrustError::Unsupported(
                "neo4j-http adapter: unanchored edge query".into(),
            ));
        };
        if q.label.as_ref().is_some_and(|l| l.as_str() == EDGE_LABEL) {
            // The SNAP shape on the `:V` id index, unchanged (see the Bolt
            // adapter).
            let tos = self
                .column(
                    format!("MATCH (a:{NODE_LABEL} {{id: $id}})-[:{EDGE_LABEL}]->(b) RETURN b.id"),
                    json!({ "id": from.as_str() }),
                )
                .await?;
            return Ok(tos
                .into_iter()
                .map(|to| Edge::new(EDGE_LABEL, from.as_str(), to, Props::new()))
                .collect());
        }
        let rel = q
            .label
            .as_ref()
            .map(|l| format!(":`{}`", l.as_str().replace('`', "``")))
            .unwrap_or_default();
        let rows = self
            .query(
                format!("MATCH (a {{id: $id}})-[r{rel}]->(b) RETURN type(r), b.id"),
                json!({ "id": from.as_str() }),
            )
            .await?;
        Ok(rows
            .into_iter()
            .filter_map(|row| {
                let label = row.first()?.as_str()?.to_string();
                let to = row.get(1)?.as_str()?.to_string();
                q.to.as_ref()
                    .is_none_or(|t| t.as_str() == to)
                    .then(|| Edge::new(label, from.as_str(), to, Props::new()))
            })
            .collect())
    }

    async fn traverse(&self, traversal: Traversal) -> grust::Result<Vec<Node>> {
        let grust::Start::Node(start) = traversal.start else {
            return Err(GrustError::Unsupported(
                "neo4j-http adapter: only node-anchored traversals".into(),
            ));
        };
        if traversal.steps.len() != 1 {
            return Err(GrustError::Unsupported(
                "neo4j-http adapter: one hop per traversal".into(),
            ));
        }
        let ids = self
            .column(
                format!("MATCH (a:{NODE_LABEL} {{id: $id}})-[:{EDGE_LABEL}]->(b) RETURN b.id"),
                json!({ "id": start.as_str() }),
            )
            .await?;
        Ok(ids
            .into_iter()
            .map(|id| Node::new(NODE_LABEL, id, Props::new()))
            .collect())
    }
}

#[async_trait]
impl GraphAdminStore for Neo4jHttpStore {
    async fn bootstrap(&self) -> grust::Result<()> {
        self.query(
            format!("CREATE INDEX v_id IF NOT EXISTS FOR (n:{NODE_LABEL}) ON (n.id)"),
            json!({}),
        )
        .await?;
        Ok(())
    }

    async fn clear(&self) -> grust::Result<()> {
        // Relationships first, then nodes, whatever their labels, in
        // bounded batches (see `neo4j.rs`): a 50k-node DETACH DELETE from
        // a hub-heavy graph exceeded the 1 GiB transaction memory cap.
        for stmt in [
            "MATCH ()-[r]->() WITH r LIMIT 100000 DELETE r RETURN count(*)",
            "MATCH (n) WITH n LIMIT 100000 DETACH DELETE n RETURN count(*)",
        ] {
            loop {
                let deleted = self.column(stmt.to_string(), json!({})).await?;
                let n: i64 = deleted.first().and_then(|c| c.parse().ok()).unwrap_or(0);
                if n == 0 {
                    break;
                }
            }
        }
        Ok(())
    }
}
