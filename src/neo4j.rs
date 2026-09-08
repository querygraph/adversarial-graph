//! Neo4j over Bolt (`neo4rs`), presented to the harness as a Grust
//! `GraphStore` so it runs the same scenarios as every internal backend.
//! A SNAP graph loads as `(:V {id})` and `[:E]`; a typed graph loads its
//! labels, relationship types and properties, one `UNWIND` batch per label
//! or per (type, from label, to label) with an id index per label. Reads
//! are ordinary Cypher. Memory tuning lives in
//! `compose.yaml` and is recorded per run. Every result taken through this
//! adapter reports `read_path = "harness-native-cypher"`.

use async_trait::async_trait;
use grust::{
    Edge, EdgeQuery, Graph, GraphAdminStore, GraphStore, GrustError, LoadReport, Node, NodeId,
    Props, PutOutcome, Traversal,
};
use neo4rs::{BoltList, BoltMap, BoltString, BoltType, Graph as Driver, query};

use crate::dataset::{EDGE_LABEL, NODE_LABEL};
use crate::differential::{Cell, ResultSet};

const BATCH: usize = 5_000;

/// Which Bolt server is on the other end. Both speak openCypher over Bolt;
/// only the index DDL differs, so one store serves both engines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoltDialect {
    Neo4j,
    Memgraph,
}

#[derive(Clone)]
pub struct Neo4jStore {
    driver: Driver,
    dialect: BoltDialect,
}

fn backend(err: impl std::fmt::Display) -> GrustError {
    GrustError::Backend(format!("neo4j: {err}"))
}

/// Grust properties as a Bolt map: ints, floats, bools and strings as
/// themselves, string arrays as lists, dates and decimals as their text.
fn bolt_props(props: &Props) -> BoltType {
    let mut map = BoltMap::new();
    for (key, value) in props.iter() {
        let cell = match value {
            grust::Value::Null => continue,
            grust::Value::Bool(b) => BoltType::Boolean(neo4rs::BoltBoolean::new(*b)),
            grust::Value::Int(i) => BoltType::Integer(neo4rs::BoltInteger::new(*i)),
            grust::Value::Float(f) => BoltType::Float(neo4rs::BoltFloat::new(*f)),
            grust::Value::String(s) => BoltType::String(BoltString::from(s.as_str())),
            grust::Value::StringArray(items) => BoltType::List(BoltList::from(
                items
                    .iter()
                    .map(|s| BoltType::String(BoltString::from(s.as_str())))
                    .collect::<Vec<_>>(),
            )),
            other => BoltType::String(BoltString::from(
                crate::differential::value_text(other).as_str(),
            )),
        };
        map.put(BoltString::from(key.as_str()), cell);
    }
    BoltType::Map(map)
}

impl Neo4jStore {
    pub async fn connect(uri: &str, user: &str, pass: &str) -> grust::Result<Self> {
        Self::connect_dialect(uri, user, pass, BoltDialect::Neo4j).await
    }

    pub async fn connect_dialect(
        uri: &str,
        user: &str,
        pass: &str,
        dialect: BoltDialect,
    ) -> grust::Result<Self> {
        // The driver defaults to the `neo4j` database; Memgraph only serves
        // `memgraph`, so the dialect picks the session database.
        let db = match dialect {
            BoltDialect::Neo4j => "neo4j",
            BoltDialect::Memgraph => "memgraph",
        };
        let config = neo4rs::ConfigBuilder::default()
            .uri(uri)
            .user(user)
            .password(pass)
            .db(db)
            .build()
            .map_err(backend)?;
        let driver = Driver::connect(config).await.map_err(backend)?;
        Ok(Self { driver, dialect })
    }

    async fn run(&self, q: neo4rs::Query) -> grust::Result<()> {
        self.driver.run(q).await.map_err(backend)
    }

    /// Delete one vertex, whatever its label, with every edge incident to
    /// it: the store's own `DETACH DELETE`, one statement, one transaction.
    pub async fn delete_node(&self, id: &NodeId) -> grust::Result<()> {
        self.run(query("MATCH (n {id: $id}) DETACH DELETE n").param("id", id.as_str()))
            .await
    }

    /// Every row of a read query, cells normalized for comparison.
    pub async fn rows(&self, cypher: &str) -> grust::Result<ResultSet> {
        let mut stream = self.driver.execute(query(cypher)).await.map_err(backend)?;
        let mut result = ResultSet::default();
        while let Some(row) = stream.next().await.map_err(backend)? {
            // The driver hands a row back as a map; the scenario realigns
            // cells to the oracle's column order by name.
            let fields: std::collections::BTreeMap<String, serde_json::Value> =
                row.to().map_err(backend)?;
            if result.columns.is_empty() {
                result.columns = fields.keys().cloned().collect();
            }
            result
                .rows
                .push(fields.values().map(Cell::from_json).collect());
        }
        Ok(result)
    }

    /// First column of every row as strings.
    async fn column(&self, q: neo4rs::Query, column: &str) -> grust::Result<Vec<String>> {
        let mut rows = self.driver.execute(q).await.map_err(backend)?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await.map_err(backend)? {
            if let Ok(v) = row.get::<String>(column) {
                out.push(v);
            } else if let Ok(v) = row.get::<i64>(column) {
                out.push(v.to_string());
            }
        }
        Ok(out)
    }
}

#[async_trait]
impl GraphStore for Neo4jStore {
    async fn put_node(&self, node: &Node) -> grust::Result<PutOutcome> {
        // Label-preserving upsert: a typed vertex keeps its label and its
        // properties travel as one map, so a read-modify-write on a
        // `Person` lands on the `Person`, not on a fresh `:V`.
        self.run(
            query(&format!(
                "MERGE (n:`{}` {{id: $id}}) SET n += $props",
                node.label.as_str().replace('`', "``")
            ))
            .param("id", node.id.as_str())
            .param("props", bolt_props(&node.props)),
        )
        .await?;
        Ok(PutOutcome::Upserted)
    }

    async fn put_edge(&self, edge: &Edge) -> grust::Result<PutOutcome> {
        self.run(
            query(&format!(
                "MATCH (a:{NODE_LABEL} {{id: $from}}), (b:{NODE_LABEL} {{id: $to}}) \
                 CREATE (a)-[:{EDGE_LABEL}]->(b)"
            ))
            .param("from", edge.from.as_str())
            .param("to", edge.to.as_str()),
        )
        .await?;
        Ok(PutOutcome::Upserted)
    }

    async fn put_graph(&self, graph: &Graph) -> grust::Result<LoadReport> {
        let mut report = LoadReport::default();
        let plan = crate::typed_load::LoadPlan::of(graph);
        for label in plan.node_labels() {
            let ddl = match self.dialect {
                BoltDialect::Neo4j => format!(
                    "CREATE INDEX {} IF NOT EXISTS FOR (n:{label}) ON (n.id)",
                    crate::typed_load::index_name(label)
                ),
                BoltDialect::Memgraph => format!("CREATE INDEX ON :{label}(id)"),
            };
            self.run(query(&ddl)).await?;
        }
        if self.dialect == BoltDialect::Neo4j {
            // Neo4j populates an index in the background; the edge batches
            // must not start before it is online.
            self.run(query("CALL db.awaitIndexes(600)")).await?;
        }
        for (label, nodes) in plan.nodes_by_label() {
            for chunk in nodes.chunks(BATCH) {
                let rows: Vec<BoltType> = chunk.iter().map(|n| bolt_props(&n.props)).collect();
                self.run(
                    query(&format!("UNWIND $rows AS r CREATE (n:{label}) SET n = r"))
                        .param("rows", BoltType::List(BoltList::from(rows))),
                )
                .await?;
                report.nodes += chunk.len();
            }
        }
        for (key, edges) in plan.edges_by_shape() {
            for chunk in edges.chunks(BATCH) {
                let rows: Vec<BoltType> = chunk
                    .iter()
                    .map(|e| {
                        let mut map = BoltMap::new();
                        map.put(
                            BoltString::from("from"),
                            BoltType::String(BoltString::from(e.from.as_str())),
                        );
                        map.put(
                            BoltString::from("to"),
                            BoltType::String(BoltString::from(e.to.as_str())),
                        );
                        map.put(BoltString::from("props"), bolt_props(&e.props));
                        BoltType::Map(map)
                    })
                    .collect();
                self.run(
                    query(&format!(
                        "UNWIND $rows AS r MATCH (a:{} {{id: r.from}}), (b:{} {{id: r.to}}) \
                         CREATE (a)-[e:{}]->(b) SET e = r.props",
                        key.from_label, key.to_label, key.relationship
                    ))
                    .param("rows", BoltType::List(BoltList::from(rows))),
                )
                .await?;
                report.edges += chunk.len();
            }
        }
        Ok(report)
    }

    async fn get_node(&self, id: &NodeId) -> grust::Result<Option<Node>> {
        // Whatever the label: the typed loads put `Person`, `Message`, …
        // and the SNAP loads `V`; the vertex comes back with its label and
        // every property.
        let mut stream = self
            .driver
            .execute(
                query("MATCH (n {id: $id}) RETURN labels(n) AS labels, properties(n) AS props LIMIT 1")
                    .param("id", id.as_str()),
            )
            .await
            .map_err(backend)?;
        let Some(row) = stream.next().await.map_err(backend)? else {
            return Ok(None);
        };
        let fields: std::collections::BTreeMap<String, serde_json::Value> =
            row.to().map_err(backend)?;
        let labels: Vec<String> = fields
            .get("labels")
            .and_then(|v| v.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|l| l.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let label = crate::typed_load::label_from_list(labels.iter().map(String::as_str));
        let props = fields
            .get("props")
            .map(crate::typed_load::props_from_json)
            .unwrap_or_default();
        Ok(Some(Node::new(label, id.as_str(), props)))
    }

    async fn get_edges(&self, q: EdgeQuery) -> grust::Result<Vec<Edge>> {
        let Some(from) = q.from else {
            return Err(GrustError::Unsupported(
                "neo4j adapter: unanchored edge query".into(),
            ));
        };
        if q.label.as_ref().is_some_and(|l| l.as_str() == EDGE_LABEL) {
            // The SNAP shape, on the `:V` id index: unchanged, so the
            // hot-node and stream families keep their access path.
            let tos = self
                .column(
                    query(&format!(
                        "MATCH (a:{NODE_LABEL} {{id: $id}})-[:{EDGE_LABEL}]->(b) RETURN b.id AS id"
                    ))
                    .param("id", from.as_str()),
                    "id",
                )
                .await?;
            return Ok(tos
                .into_iter()
                .map(|to| Edge::new(EDGE_LABEL, from.as_str(), to, Props::new()))
                .collect());
        }
        // Typed graphs: any label on the anchor, the asked-for relationship
        // type or every type, with the type reported per edge.
        let rel = q
            .label
            .as_ref()
            .map(|l| format!(":`{}`", l.as_str().replace('`', "``")))
            .unwrap_or_default();
        let mut stream = self
            .driver
            .execute(
                query(&format!(
                    "MATCH (a {{id: $id}})-[r{rel}]->(b) RETURN type(r) AS label, b.id AS to"
                ))
                .param("id", from.as_str()),
            )
            .await
            .map_err(backend)?;
        let mut edges = Vec::new();
        while let Some(row) = stream.next().await.map_err(backend)? {
            let label: String = row.get("label").map_err(backend)?;
            let to: String = row.get("to").map_err(backend)?;
            if q.to.as_ref().is_none_or(|t| t.as_str() == to) {
                edges.push(Edge::new(label, from.as_str(), to, Props::new()));
            }
        }
        Ok(edges)
    }

    async fn traverse(&self, traversal: Traversal) -> grust::Result<Vec<Node>> {
        let grust::Start::Node(start) = traversal.start else {
            return Err(GrustError::Unsupported(
                "neo4j adapter: only node-anchored traversals".into(),
            ));
        };
        if traversal.steps.len() != 1 {
            return Err(GrustError::Unsupported(
                "neo4j adapter: one hop per traversal".into(),
            ));
        }
        let ids = self
            .column(
                query(&format!(
                    "MATCH (a:{NODE_LABEL} {{id: $id}})-[:{EDGE_LABEL}]->(b) RETURN b.id AS id"
                ))
                .param("id", start.as_str()),
                "id",
            )
            .await?;
        Ok(ids
            .into_iter()
            .map(|id| Node::new(NODE_LABEL, id, Props::new()))
            .collect())
    }
}

#[async_trait]
impl GraphAdminStore for Neo4jStore {
    async fn bootstrap(&self) -> grust::Result<()> {
        let ddl = match self.dialect {
            BoltDialect::Neo4j => {
                format!("CREATE INDEX v_id IF NOT EXISTS FOR (n:{NODE_LABEL}) ON (n.id)")
            }
            // Memgraph's label-property index DDL; creating it twice is a no-op.
            BoltDialect::Memgraph => format!("CREATE INDEX ON :{NODE_LABEL}(id)"),
        };
        self.run(query(&ddl)).await
    }

    async fn clear(&self) -> grust::Result<()> {
        // Every relationship first, then every node, whatever their labels
        // and types, in bounded batches: a typed graph left by an earlier
        // run never leaks into this one, and each transaction's footprint
        // stays proportional to the batch. A 50k-node DETACH DELETE from a
        // hub-heavy graph (web-Google, at the cit-Patents cell) exceeded
        // Neo4j's 1 GiB transaction memory cap.
        for stmt in [
            "MATCH ()-[r]->() WITH r LIMIT 100000 DELETE r RETURN count(*) AS c",
            "MATCH (n) WITH n LIMIT 100000 DETACH DELETE n RETURN count(*) AS c",
        ] {
            loop {
                let deleted = self.column(query(stmt), "c").await?;
                let n: i64 = deleted.first().and_then(|c| c.parse().ok()).unwrap_or(0);
                if n == 0 {
                    break;
                }
            }
        }
        Ok(())
    }
}
