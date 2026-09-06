//! Neo4j over Bolt (`neo4rs`), presented to the harness as a Grust
//! `GraphStore` so it runs the same scenarios as every internal backend.
//! Nodes are `(:V {id})`, edges `[:E]`; loads go through `UNWIND` batches of
//! parallel id arrays; reads are ordinary Cypher. Memory tuning lives in
//! `compose.yaml` and is recorded per run. Every result taken through this
//! adapter reports `read_path = "harness-native-cypher"`.

use async_trait::async_trait;
use grust::{
    Edge, EdgeQuery, Graph, GraphAdminStore, GraphStore, GrustError, LoadReport, Node, NodeId,
    Props, PutOutcome, Traversal,
};
use neo4rs::{Graph as Driver, query};

use crate::dataset::{EDGE_LABEL, NODE_LABEL};

const BATCH: usize = 5_000;

#[derive(Clone)]
pub struct Neo4jStore {
    driver: Driver,
}

fn backend(err: impl std::fmt::Display) -> GrustError {
    GrustError::Backend(format!("neo4j: {err}"))
}

impl Neo4jStore {
    pub async fn connect(uri: &str, user: &str, pass: &str) -> grust::Result<Self> {
        let driver = Driver::new(uri, user, pass).await.map_err(backend)?;
        Ok(Self { driver })
    }

    async fn run(&self, q: neo4rs::Query) -> grust::Result<()> {
        self.driver.run(q).await.map_err(backend)
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
        self.run(
            query(&format!("MERGE (n:{NODE_LABEL} {{id: $id}})")).param("id", node.id.as_str()),
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
        for chunk in graph.nodes.chunks(BATCH) {
            let ids: Vec<String> = chunk.iter().map(|n| n.id.as_str().to_string()).collect();
            self.run(
                query(&format!(
                    "UNWIND $ids AS id MERGE (n:{NODE_LABEL} {{id: id}})"
                ))
                .param("ids", ids),
            )
            .await?;
            report.nodes += chunk.len();
        }
        for chunk in graph.edges.chunks(BATCH) {
            let froms: Vec<String> = chunk.iter().map(|e| e.from.as_str().to_string()).collect();
            let tos: Vec<String> = chunk.iter().map(|e| e.to.as_str().to_string()).collect();
            self.run(
                query(&format!(
                    "UNWIND range(0, size($froms) - 1) AS i \
                     MATCH (a:{NODE_LABEL} {{id: $froms[i]}}), (b:{NODE_LABEL} {{id: $tos[i]}}) \
                     CREATE (a)-[:{EDGE_LABEL}]->(b)"
                ))
                .param("froms", froms)
                .param("tos", tos),
            )
            .await?;
            report.edges += chunk.len();
        }
        Ok(report)
    }

    async fn get_node(&self, id: &NodeId) -> grust::Result<Option<Node>> {
        let found = self
            .column(
                query(&format!(
                    "MATCH (n:{NODE_LABEL} {{id: $id}}) RETURN n.id AS id"
                ))
                .param("id", id.as_str()),
                "id",
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
                "neo4j adapter: unanchored edge query".into(),
            ));
        };
        let tos = self
            .column(
                query(&format!(
                    "MATCH (a:{NODE_LABEL} {{id: $id}})-[:{EDGE_LABEL}]->(b) RETURN b.id AS id"
                ))
                .param("id", from.as_str()),
                "id",
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
        self.run(query(&format!(
            "CREATE INDEX v_id IF NOT EXISTS FOR (n:{NODE_LABEL}) ON (n.id)"
        )))
        .await
    }

    async fn clear(&self) -> grust::Result<()> {
        // Batched detach-delete so a large prior graph never exhausts the
        // transaction memory cap.
        loop {
            let deleted = self
                .column(
                    query(&format!(
                        "MATCH (n:{NODE_LABEL}) WITH n LIMIT 50000 DETACH DELETE n RETURN count(*) AS c"
                    )),
                    "c",
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
