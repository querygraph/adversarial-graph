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
        self.query(
            format!("MERGE (n:{NODE_LABEL} {{id: $id}})"),
            json!({ "id": node.id.as_str() }),
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
        for chunk in graph.nodes.chunks(BATCH) {
            let ids: Vec<&str> = chunk.iter().map(|n| n.id.as_str()).collect();
            self.query(
                format!("UNWIND $ids AS id MERGE (n:{NODE_LABEL} {{id: id}})"),
                json!({ "ids": ids }),
            )
            .await?;
            report.nodes += chunk.len();
        }
        for chunk in graph.edges.chunks(BATCH) {
            let froms: Vec<&str> = chunk.iter().map(|e| e.from.as_str()).collect();
            let tos: Vec<&str> = chunk.iter().map(|e| e.to.as_str()).collect();
            self.query(
                format!(
                    "UNWIND range(0, size($froms) - 1) AS i \
                     MATCH (a:{NODE_LABEL} {{id: $froms[i]}}), (b:{NODE_LABEL} {{id: $tos[i]}}) \
                     CREATE (a)-[:{EDGE_LABEL}]->(b)"
                ),
                json!({ "froms": froms, "tos": tos }),
            )
            .await?;
            report.edges += chunk.len();
        }
        Ok(report)
    }

    async fn get_node(&self, id: &NodeId) -> grust::Result<Option<Node>> {
        let found = self
            .column(
                format!("MATCH (n:{NODE_LABEL} {{id: $id}}) RETURN n.id"),
                json!({ "id": id.as_str() }),
            )
            .await?;
        Ok(found
            .into_iter()
            .next()
            .map(|id| Node::new(NODE_LABEL, id, Props::new())))
    }

    async fn get_edges(&self, q: EdgeQuery) -> grust::Result<Vec<Edge>> {
        let Some(from) = q.from else {
            return Err(GrustError::Unsupported(
                "neo4j-http adapter: unanchored edge query".into(),
            ));
        };
        let tos = self
            .column(
                format!("MATCH (a:{NODE_LABEL} {{id: $id}})-[:{EDGE_LABEL}]->(b) RETURN b.id"),
                json!({ "id": from.as_str() }),
            )
            .await?;
        Ok(tos
            .into_iter()
            .map(|to| Edge::new(EDGE_LABEL, from.as_str(), to, Props::new()))
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
        loop {
            let deleted = self
                .column(
                    format!(
                        "MATCH (n:{NODE_LABEL}) WITH n LIMIT 50000 DETACH DELETE n RETURN count(*)"
                    ),
                    json!({}),
                )
                .await?;
            let n: i64 = deleted.first().and_then(|c| c.parse().ok()).unwrap_or(0);
            if n == 0 {
                break;
            }
        }
        Ok(())
    }
}
