//! Apache AGE (the openCypher extension for PostgreSQL), presented to the
//! harness as a Grust `GraphStore` so it runs the same scenarios as every
//! internal backend. Every operation is Cypher sent through AGE's
//! `cypher()` table function over the PostgreSQL wire protocol; the harness
//! never touches the label tables directly. Nodes are `(:V {id})`, edges
//! `[:E]`; loads go through `UNWIND` batches; reads are ordinary Cypher.
//! Every result taken through this adapter reports
//! `read_path = "harness-native-cypher"`.
//!
//! Indexes created at bootstrap, as the harness does for FalkorDB, Neo4j
//! and Helix, so the engine and not a missing index is what gets measured:
//! a GIN index on `V.properties` (what AGE's planner uses for property
//! equality) and B-tree indexes on `E.start_id` / `E.end_id` (AGE stores
//! edges in a plain heap; without them every one-hop read is a sequential
//! scan of the edge table).
//!
//! Parameters travel as an `agtype` map in PostgreSQL text format (AGE has
//! no binary send/recv for `agtype`), and result columns are cast to `text`
//! for the same reason. A small fixed pool of connections is rotated
//! round-robin; `tokio-postgres` pipelines concurrent queries on one
//! connection, so the pool only bounds server-side parallelism.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use bytes::BytesMut;
use grust::{
    Edge, EdgeQuery, Graph, GraphAdminStore, GraphStore, GrustError, LoadReport, Node, NodeId,
    Props, PutOutcome, Traversal,
};
use tokio_postgres::types::{Format, IsNull, ToSql, Type, to_sql_checked};
use tokio_postgres::{Client, NoTls};

use crate::dataset::{EDGE_LABEL, NODE_LABEL};

const BATCH: usize = 5_000;

/// An `agtype` value sent in text format: AGE parses it with `agtype_in`.
#[derive(Debug)]
struct AgParam(String);

impl ToSql for AgParam {
    fn to_sql(
        &self,
        _ty: &Type,
        out: &mut BytesMut,
    ) -> Result<IsNull, Box<dyn std::error::Error + Sync + Send>> {
        out.extend_from_slice(self.0.as_bytes());
        Ok(IsNull::No)
    }
    fn accepts(ty: &Type) -> bool {
        ty.name() == "agtype"
    }
    fn encode_format(&self, _ty: &Type) -> Format {
        Format::Text
    }
    to_sql_checked!();
}

pub struct AgeStore {
    clients: Vec<Arc<Client>>,
    next: AtomicUsize,
    graph: String,
}

fn backend(err: tokio_postgres::Error) -> GrustError {
    // The driver's Display is just "db error"; the server's message is the
    // source.
    let detail = err
        .as_db_error()
        .map(|d| d.message().to_string())
        .or_else(|| std::error::Error::source(&err).map(|s| s.to_string()))
        .unwrap_or_default();
    GrustError::Backend(format!("age: {err}: {detail}"))
}

/// The text form of an agtype scalar back to a plain string: strings come
/// back JSON-quoted (`"123"`), integers bare (`123`).
fn unquote(text: &str) -> String {
    text.strip_prefix('"')
        .and_then(|t| t.strip_suffix('"'))
        .unwrap_or(text)
        .to_string()
}

impl AgeStore {
    /// `url` is a `tokio-postgres` connection string; `pool` connections are
    /// opened, each with the AGE library loaded and `ag_catalog` on the
    /// search path.
    pub async fn connect(url: &str, graph: &str, pool: usize) -> grust::Result<Self> {
        let mut clients = Vec::with_capacity(pool.max(1));
        for _ in 0..pool.max(1) {
            let (client, connection) =
                tokio_postgres::connect(url, NoTls).await.map_err(backend)?;
            tokio::spawn(async move {
                if let Err(e) = connection.await {
                    eprintln!("age: connection closed: {e}");
                }
            });
            client
                .batch_execute("LOAD 'age'; SET search_path = ag_catalog, \"$user\", public;")
                .await
                .map_err(backend)?;
            clients.push(Arc::new(client));
        }
        Ok(Self {
            clients,
            next: AtomicUsize::new(0),
            graph: graph.to_string(),
        })
    }

    fn client(&self) -> &Client {
        let i = self.next.fetch_add(1, Ordering::Relaxed) % self.clients.len();
        &self.clients[i]
    }

    /// Run a Cypher body with a parameter map; every returned column comes
    /// back as agtype text. `columns` is the AGE column-definition list.
    async fn cypher(
        &self,
        body: &str,
        params: serde_json::Value,
        columns: &[&str],
    ) -> grust::Result<Vec<Vec<String>>> {
        let select = columns
            .iter()
            .map(|c| format!("{c}::text"))
            .collect::<Vec<_>>()
            .join(", ");
        let defs = columns
            .iter()
            .map(|c| format!("{c} agtype"))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT {select} FROM ag_catalog.cypher('{}', $q${body}$q$, $1) AS ({defs})",
            self.graph
        );
        let rows = self
            .client()
            .query(&sql, &[&AgParam(params.to_string())])
            .await
            .map_err(backend)?;
        Ok(rows
            .iter()
            .map(|row| {
                (0..columns.len())
                    .map(|i| {
                        row.try_get::<_, String>(i)
                            .map(|s| unquote(&s))
                            .unwrap_or_default()
                    })
                    .collect()
            })
            .collect())
    }

    /// Delete one vertex, whatever its label, with every edge incident to
    /// it, through `cypher()`: one statement, one PostgreSQL transaction.
    pub async fn delete_node(&self, id: &NodeId) -> grust::Result<()> {
        self.cypher(
            "MATCH (n {id: $id}) DETACH DELETE n",
            serde_json::json!({ "id": id.as_str() }),
            &["a"],
        )
        .await
        .map(|_| ())
    }

    /// First column of a single-column Cypher result.
    async fn column(&self, body: &str, params: serde_json::Value) -> grust::Result<Vec<String>> {
        Ok(self
            .cypher(body, params, &["a"])
            .await?
            .into_iter()
            .map(|mut r| r.remove(0))
            .collect())
    }

    async fn sql(&self, sql: &str) -> grust::Result<()> {
        self.client().batch_execute(sql).await.map_err(backend)
    }

    async fn graph_exists(&self) -> grust::Result<bool> {
        let row = self
            .client()
            .query_one(
                "SELECT count(*)::bigint FROM ag_catalog.ag_graph WHERE name = $1",
                &[&self.graph],
            )
            .await
            .map_err(backend)?;
        Ok(row.get::<_, i64>(0) > 0)
    }

    async fn label_exists(&self, label: &str) -> grust::Result<bool> {
        let row = self
            .client()
            .query_one(
                "SELECT count(*)::bigint FROM ag_catalog.ag_label l \
                 JOIN ag_catalog.ag_graph g ON l.graph = g.graphid \
                 WHERE g.name = $1 AND l.name = $2",
                &[&self.graph, &label],
            )
            .await
            .map_err(backend)?;
        Ok(row.get::<_, i64>(0) > 0)
    }

    /// Graph, labels and the indexes the scenarios rely on; idempotent.
    async fn ensure_schema(&self) -> grust::Result<()> {
        let g = &self.graph;
        if !self.graph_exists().await? {
            self.sql(&format!("SELECT ag_catalog.create_graph('{g}')"))
                .await?;
        }
        if !self.label_exists(NODE_LABEL).await? {
            self.sql(&format!(
                "SELECT ag_catalog.create_vlabel('{g}', '{NODE_LABEL}')"
            ))
            .await?;
        }
        if !self.label_exists(EDGE_LABEL).await? {
            self.sql(&format!(
                "SELECT ag_catalog.create_elabel('{g}', '{EDGE_LABEL}')"
            ))
            .await?;
        }
        self.sql(&format!(
            "CREATE INDEX IF NOT EXISTS {NODE_LABEL}_properties_gin ON \"{g}\".\"{NODE_LABEL}\" USING gin (properties); \
             CREATE INDEX IF NOT EXISTS {NODE_LABEL}_id_btree ON \"{g}\".\"{NODE_LABEL}\" \
                 (ag_catalog.agtype_access_operator(VARIADIC ARRAY[properties, '\"id\"'::ag_catalog.agtype])); \
             CREATE INDEX IF NOT EXISTS {EDGE_LABEL}_start ON \"{g}\".\"{EDGE_LABEL}\" (start_id); \
             CREATE INDEX IF NOT EXISTS {EDGE_LABEL}_end ON \"{g}\".\"{EDGE_LABEL}\" (end_id);"
        ))
        .await
    }
}

#[async_trait]
impl GraphStore for AgeStore {
    async fn put_node(&self, node: &Node) -> grust::Result<PutOutcome> {
        // Label-preserving upsert. AGE has no `SET n += map`, so every
        // property is its own `SET` clause bound to its own parameter.
        let mut params = serde_json::Map::new();
        params.insert("id".into(), serde_json::json!(node.id.as_str()));
        let mut sets = Vec::with_capacity(node.props.len());
        for (i, (key, value)) in node.props.iter().enumerate() {
            let cell = match value {
                grust::Value::Null => continue,
                grust::Value::Bool(b) => serde_json::json!(b),
                grust::Value::Int(v) => serde_json::json!(v),
                grust::Value::Float(f) => serde_json::json!(f),
                grust::Value::String(s) => serde_json::json!(s),
                grust::Value::StringArray(items) => serde_json::json!(items),
                other => serde_json::json!(crate::differential::value_text(other)),
            };
            let name = format!("p{i}");
            sets.push(format!("n.`{}` = ${name}", key.replace('`', "``")));
            params.insert(name, cell);
        }
        let set_clause = if sets.is_empty() {
            String::new()
        } else {
            format!(" SET {}", sets.join(", "))
        };
        self.column(
            &format!("MERGE (n:{} {{id: $id}}){set_clause} RETURN 1", node.label.as_str()),
            serde_json::Value::Object(params),
        )
        .await?;
        Ok(PutOutcome::Upserted)
    }

    async fn put_edge(&self, edge: &Edge) -> grust::Result<PutOutcome> {
        self.column(
            &format!(
                "MATCH (a:{NODE_LABEL} {{id: $from}}), (b:{NODE_LABEL} {{id: $to}}) \
                 CREATE (a)-[:{EDGE_LABEL}]->(b) RETURN 1"
            ),
            serde_json::json!({ "from": edge.from.as_str(), "to": edge.to.as_str() }),
        )
        .await?;
        Ok(PutOutcome::Upserted)
    }

    async fn put_graph(&self, graph: &Graph) -> grust::Result<LoadReport> {
        let mut report = LoadReport::default();
        for chunk in graph.nodes.chunks(BATCH) {
            let ids: Vec<&str> = chunk.iter().map(|n| n.id.as_str()).collect();
            self.column(
                &format!("UNWIND $ids AS id CREATE (n:{NODE_LABEL} {{id: id}}) RETURN count(*)"),
                serde_json::json!({ "ids": ids }),
            )
            .await?;
            report.nodes += chunk.len();
        }
        for chunk in graph.edges.chunks(BATCH) {
            let pairs: Vec<serde_json::Value> = chunk
                .iter()
                .map(|e| serde_json::json!({ "f": e.from.as_str(), "t": e.to.as_str() }))
                .collect();
            self.column(
                &format!(
                    "UNWIND $pairs AS p \
                     MATCH (a:{NODE_LABEL} {{id: p.f}}), (b:{NODE_LABEL} {{id: p.t}}) \
                     CREATE (a)-[:{EDGE_LABEL}]->(b) RETURN count(*)"
                ),
                serde_json::json!({ "pairs": pairs }),
            )
            .await?;
            report.edges += chunk.len();
        }
        Ok(report)
    }

    async fn get_node(&self, id: &NodeId) -> grust::Result<Option<Node>> {
        // Whatever the label, with every property: `label(n)` and
        // `properties(n)` come back as agtype text, which for a map of
        // scalars is JSON.
        let rows = self
            .cypher(
                "MATCH (n {id: $id}) RETURN label(n), properties(n) LIMIT 1",
                serde_json::json!({ "id": id.as_str() }),
                &["l", "p"],
            )
            .await?;
        let Some(row) = rows.into_iter().next() else {
            return Ok(None);
        };
        let label = row.first().filter(|l| !l.is_empty()).cloned().unwrap_or_else(|| NODE_LABEL.to_string());
        let props = row
            .get(1)
            .and_then(|text| serde_json::from_str::<serde_json::Value>(text).ok())
            .map(|v| crate::typed_load::props_from_json(&v))
            .unwrap_or_default();
        Ok(Some(Node::new(label, id.as_str(), props)))
    }

    async fn get_edges(&self, q: EdgeQuery) -> grust::Result<Vec<Edge>> {
        let Some(from) = q.from else {
            return Err(GrustError::Unsupported(
                "age adapter: unanchored edge query".into(),
            ));
        };
        if q.label.as_ref().is_some_and(|l| l.as_str() == EDGE_LABEL) {
            // The SNAP shape on the `V` id index, unchanged (see the Bolt
            // adapter).
            let tos = self
                .column(
                    &format!("MATCH (a:{NODE_LABEL} {{id: $id}})-[:{EDGE_LABEL}]->(b) RETURN b.id"),
                    serde_json::json!({ "id": from.as_str() }),
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
            .map(|l| format!(":{}", l.as_str()))
            .unwrap_or_default();
        let rows = self
            .cypher(
                &format!("MATCH (a {{id: $id}})-[r{rel}]->(b) RETURN type(r), b.id"),
                serde_json::json!({ "id": from.as_str() }),
                &["t", "b"],
            )
            .await?;
        Ok(rows
            .into_iter()
            .filter_map(|row| {
                let label = row.first()?.clone();
                let to = row.get(1)?.clone();
                q.to.as_ref()
                    .is_none_or(|t| t.as_str() == to)
                    .then(|| Edge::new(label, from.as_str(), to, Props::new()))
            })
            .collect())
    }

    async fn traverse(&self, traversal: Traversal) -> grust::Result<Vec<Node>> {
        let grust::Start::Node(start) = traversal.start else {
            return Err(GrustError::Unsupported(
                "age adapter: only node-anchored traversals".into(),
            ));
        };
        if traversal.steps.len() != 1 {
            return Err(GrustError::Unsupported(
                "age adapter: one hop per traversal".into(),
            ));
        }
        let ids = self
            .column(
                &format!("MATCH (a:{NODE_LABEL} {{id: $id}})-[:{EDGE_LABEL}]->(b) RETURN b.id"),
                serde_json::json!({ "id": start.as_str() }),
            )
            .await?;
        Ok(ids
            .into_iter()
            .map(|id| Node::new(NODE_LABEL, id, Props::new()))
            .collect())
    }
}

#[async_trait]
impl GraphAdminStore for AgeStore {
    async fn bootstrap(&self) -> grust::Result<()> {
        self.sql("CREATE EXTENSION IF NOT EXISTS age").await?;
        self.ensure_schema().await
    }

    /// Drop the whole graph (its label tables go with it) and recreate the
    /// empty schema with its indexes.
    async fn clear(&self) -> grust::Result<()> {
        if self.graph_exists().await? {
            self.sql(&format!(
                "SELECT ag_catalog.drop_graph('{}', true)",
                self.graph
            ))
            .await?;
        }
        self.ensure_schema().await
    }
}
