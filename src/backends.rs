//! Systems under test. Each backend is reached only through the published
//! `grust-graph` 0.13 API so the harness never depends on a Grust checkout.

use std::path::PathBuf;
use std::sync::Arc;

use grust::{Edge, EdgeQuery, Graph, GraphAdminStore, GraphStore, NodeId, Traversal};
#[allow(unused_imports)]
use grust::GraphAdminStore as _;
use grust::{TursoConfig, TursoGraphStore, TursoJournalMode};

use crate::dataset::EDGE_LABEL;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendKind {
    Memory,
    TursoWal,
    TursoMvcc,
    #[cfg(feature = "postgres")]
    Postgres,
    #[cfg(feature = "surreal")]
    Surreal,
    #[cfg(feature = "falkor")]
    Falkor,
    #[cfg(feature = "lancedb")]
    LanceDb,
    #[cfg(feature = "neo4j")]
    Neo4j,
}

impl BackendKind {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "memory" => Some(Self::Memory),
            "turso" | "turso-wal" => Some(Self::TursoWal),
            "turso-mvcc" => Some(Self::TursoMvcc),
            #[cfg(feature = "postgres")]
            "postgres" => Some(Self::Postgres),
            #[cfg(feature = "surreal")]
            "surreal" => Some(Self::Surreal),
            #[cfg(feature = "falkor")]
            "falkor" => Some(Self::Falkor),
            #[cfg(feature = "lancedb")]
            "lancedb" => Some(Self::LanceDb),
            #[cfg(feature = "neo4j")]
            "neo4j" => Some(Self::Neo4j),
            _ => None,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Memory => "memory",
            Self::TursoWal => "turso-wal",
            Self::TursoMvcc => "turso-mvcc",
            #[cfg(feature = "postgres")]
            Self::Postgres => "postgres",
            #[cfg(feature = "surreal")]
            Self::Surreal => "surreal",
            #[cfg(feature = "falkor")]
            Self::Falkor => "falkor",
            #[cfg(feature = "lancedb")]
            Self::LanceDb => "lancedb",
            #[cfg(feature = "neo4j")]
            Self::Neo4j => "neo4j",
        }
    }
    pub fn all() -> Vec<BackendKind> {
        vec![
            Self::Memory,
            Self::TursoWal,
            Self::TursoMvcc,
            #[cfg(feature = "postgres")]
            Self::Postgres,
            #[cfg(feature = "surreal")]
            Self::Surreal,
            #[cfg(feature = "falkor")]
            Self::Falkor,
            #[cfg(feature = "lancedb")]
            Self::LanceDb,
            #[cfg(feature = "neo4j")]
            Self::Neo4j,
        ]
    }
    /// Docker container serving this backend, if any (for resource probes).
    pub fn container(self) -> Option<&'static str> {
        match self {
            #[cfg(feature = "postgres")]
            Self::Postgres => Some("adversarial-graph-postgres-1"),
            #[cfg(feature = "surreal")]
            Self::Surreal => Some("adversarial-graph-surreal-1"),
            #[cfg(feature = "falkor")]
            Self::Falkor => Some("adversarial-graph-falkor-1"),
            #[cfg(feature = "neo4j")]
            Self::Neo4j => Some("adversarial-graph-neo4j-1"),
            _ => None,
        }
    }
    pub fn is_turso(self) -> bool {
        matches!(self, Self::TursoWal | Self::TursoMvcc)
    }
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

#[cfg(feature = "postgres")]
async fn connect_postgres(tag: &str) -> grust::Result<grust::PostgresGraphStore> {
    grust::PostgresGraphStore::connect(grust::PostgresGraphConfig {
        connection_string: env_or(
            "AG_POSTGRES_URL",
            "host=127.0.0.1 port=15432 user=postgres password=postgres dbname=graph",
        ),
        schema: "public".to_string(),
        table_prefix: format!("ag_{}", tag.replace('-', "_").to_ascii_lowercase()),
        batch_size: 500,
    })
    .await
}

#[cfg(feature = "surreal")]
async fn connect_surreal(tag: &str) -> grust::Result<grust::SurrealSdkGraphStore> {
    // The SDK store speaks the native WebSocket protocol through the
    // `surrealdb` crate; the HTTP `/sql` store stays available for
    // differential comparison but is not the benchmark path.
    grust::SurrealSdkGraphStore::connect(grust::SurrealConfig {
        url: env_or("AG_SURREAL_URL", "http://127.0.0.1:18000/sql"),
        user: "root".to_string(),
        pass: "root".to_string(),
        namespace: "ag".to_string(),
        database: format!("ag_{}", tag.replace('-', "_").to_ascii_lowercase()),
        batch_size: 500,
        labels: vec![crate::dataset::NODE_LABEL.to_string()],
        relationships: vec![EDGE_LABEL.to_string()],
    })
    .await
}

#[cfg(feature = "falkor")]
fn connect_falkor(tag: &str) -> grust::FalkorGraphStore {
    grust::FalkorGraphStore::new(grust::FalkorConfig {
        redis_url: env_or("AG_FALKOR_URL", "redis://127.0.0.1:16379"),
        graph: format!("ag_{}", tag.replace('-', "_").to_ascii_lowercase()),
        batch_size: 1_000,
        pool_size: 16,
        id_property: "id".to_string(),
        labels_property: "labels".to_string(),
    })
}

#[cfg(feature = "neo4j")]
async fn connect_neo4j() -> grust::Result<crate::neo4j::Neo4jStore> {
    crate::neo4j::Neo4jStore::connect(
        &env_or("AG_NEO4J_URI", "bolt://127.0.0.1:17687"),
        &env_or("AG_NEO4J_USER", "neo4j"),
        &env_or("AG_NEO4J_PASS", "adversarial"),
    )
    .await
}

#[cfg(feature = "lancedb")]
async fn connect_lancedb(work_dir: &std::path::Path, tag: &str) -> grust::Result<grust::LanceDbGraphStore> {
    grust::LanceDbGraphStore::connect(grust::LanceDbConfig {
        uri: work_dir.join(format!("lancedb-{tag}")).display().to_string(),
        table_prefix: "ag".to_string(),
        batch_size: 500,
    })
    .await
}

/// A live handle to a backend plus what the harness needs to reopen it.
pub struct Backend {
    pub kind: BackendKind,
    pub store: Arc<dyn GraphStore>,
    pub memory: Option<grust::MemoryGraphStore>,
    pub turso: Option<Arc<TursoGraphStore>>,
    pub turso_path: Option<PathBuf>,
    pub tag: String,
    #[cfg(feature = "falkor")]
    pub falkor: Option<crate::falkor_reader::FalkorReader>,
}

impl Backend {
    pub async fn open(kind: BackendKind, work_dir: &std::path::Path, tag: &str) -> grust::Result<Self> {
        match kind {
            BackendKind::Memory => {
                let store = grust::MemoryGraphStore::new();
                Ok(Self {
                    kind,
                    store: Arc::new(store.clone()),
                    memory: Some(store),
                    turso: None,
                    turso_path: None,
                    tag: tag.to_string(),
                    #[cfg(feature = "falkor")]
                    falkor: None,
                })
            }
            #[cfg(feature = "postgres")]
            BackendKind::Postgres => {
                let store = Arc::new(connect_postgres(tag).await?);
                store.bootstrap().await?;
                store.clear().await?;
                Ok(Self { kind, store, memory: None, turso: None, turso_path: None, tag: tag.to_string(), #[cfg(feature = "falkor")] falkor: None })
            }
            #[cfg(feature = "surreal")]
            BackendKind::Surreal => {
                let store = Arc::new(connect_surreal(tag).await?);
                store.bootstrap().await?;
                store.clear().await?;
                Ok(Self { kind, store, memory: None, turso: None, turso_path: None, tag: tag.to_string(), #[cfg(feature = "falkor")] falkor: None })
            }
            #[cfg(feature = "falkor")]
            BackendKind::Falkor => {
                let store = Arc::new(connect_falkor(tag));
                store.bootstrap().await?;
                store.clear().await?;
                let reader = crate::falkor_reader::FalkorReader::new(
                    &env_or("AG_FALKOR_URL", "redis://127.0.0.1:16379"),
                    &format!("ag_{}", tag.replace('-', "_").to_ascii_lowercase()),
                )?;
                Ok(Self { kind, store, memory: None, turso: None, turso_path: None, tag: tag.to_string(), falkor: Some(reader) })
            }
            #[cfg(feature = "neo4j")]
            BackendKind::Neo4j => {
                let store = Arc::new(connect_neo4j().await?);
                store.bootstrap().await?;
                store.clear().await?;
                Ok(Self { kind, store, memory: None, turso: None, turso_path: None, tag: tag.to_string(), #[cfg(feature = "falkor")] falkor: None })
            }
            #[cfg(feature = "lancedb")]
            BackendKind::LanceDb => {
                std::fs::create_dir_all(work_dir).map_err(|e| grust::GrustError::Backend(e.to_string()))?;
                let store = Arc::new(connect_lancedb(work_dir, tag).await?);
                store.bootstrap().await?;
                store.clear().await?;
                Ok(Self { kind, store, memory: None, turso: None, turso_path: None, tag: tag.to_string(), #[cfg(feature = "falkor")] falkor: None })
            }
            BackendKind::TursoWal | BackendKind::TursoMvcc => {
                std::fs::create_dir_all(work_dir).map_err(|e| grust::GrustError::Backend(e.to_string()))?;
                let path = work_dir.join(format!("{}-{}.db", kind.name(), tag));
                for suffix in ["", "-wal", "-shm", "-log"] {
                    let _ = std::fs::remove_file(format!("{}{}", path.display(), suffix));
                }
                let store = Arc::new(Self::connect_turso(kind, &path).await?);
                store.bootstrap().await?;
                Ok(Self {
                    kind,
                    store: store.clone(),
                    memory: None,
                    turso: Some(store),
                    turso_path: Some(path),
                    tag: tag.to_string(),
                    #[cfg(feature = "falkor")]
                    falkor: None,
                })
            }
        }
    }

    pub async fn connect_turso(kind: BackendKind, path: &std::path::Path) -> grust::Result<TursoGraphStore> {
        TursoGraphStore::connect(TursoConfig {
            path: path.display().to_string(),
            table_prefix: "ag".to_string(),
            batch_size: 500,
            journal_mode: if kind == BackendKind::TursoMvcc {
                TursoJournalMode::Mvcc
            } else {
                TursoJournalMode::Wal
            },
        })
        .await
    }

    /// Open an additional connection to the same durable database (Turso
    /// serializes each handle on one connection, so concurrency needs one
    /// handle per writer). Memory stores are cheaply cloneable and shared.
    pub async fn extra_handle(&self) -> grust::Result<Arc<dyn GraphStore>> {
        match self.kind {
            BackendKind::Memory => Ok(self.store.clone()),
            BackendKind::TursoWal | BackendKind::TursoMvcc => {
                let path = self.turso_path.as_ref().expect("turso path");
                Ok(Arc::new(Self::connect_turso(self.kind, path).await?))
            }
            #[cfg(feature = "postgres")]
            BackendKind::Postgres => Ok(Arc::new(connect_postgres(&self.tag).await?)),
            #[cfg(feature = "surreal")]
            BackendKind::Surreal => Ok(Arc::new(connect_surreal(&self.tag).await?)),
            #[cfg(feature = "falkor")]
            BackendKind::Falkor => Ok(Arc::new(connect_falkor(&self.tag))),
            #[cfg(feature = "lancedb")]
            BackendKind::LanceDb => Ok(self.store.clone()),
            #[cfg(feature = "neo4j")]
            BackendKind::Neo4j => Ok(self.store.clone()), // the driver pools Bolt connections
        }
    }

    pub async fn load(&self, graph: &Graph) -> grust::Result<grust::LoadReport> {
        self.store.put_graph(graph).await
    }

    /// Distinct vertices reached by exactly `k` out-hops from `start`
    /// (union of layers 1..=k, excluding `start`), computed with the
    /// portable traversal IR one hop at a time so the answer does not depend
    /// on a backend's multi-step traversal semantics.
    pub async fn khop(&self, start: &NodeId, k: usize) -> grust::Result<Vec<usize>> {
        let mut visited: std::collections::HashSet<NodeId> = std::collections::HashSet::new();
        visited.insert(start.clone());
        let mut frontier = vec![start.clone()];
        let mut layers = Vec::with_capacity(k);
        for _ in 0..k {
            let mut next = Vec::new();
            for v in &frontier {
                let ids: Vec<NodeId> = self.neighbors(v).await?;
                for id in ids {
                    if visited.insert(id.clone()) {
                        next.push(id);
                    }
                }
            }
            layers.push(next.len());
            frontier = next;
            if frontier.is_empty() {
                break;
            }
        }
        Ok(layers)
    }

    /// Out-neighbour ids of one vertex: the portable traversal IR, or the
    /// harness-native Cypher path for stores whose Grust adapter cannot read.
    pub async fn neighbors(&self, v: &NodeId) -> grust::Result<Vec<NodeId>> {
        #[cfg(feature = "falkor")]
        if let Some(reader) = &self.falkor {
            let reader = reader.clone();
            let id = v.as_str().to_string();
            return tokio::task::spawn_blocking(move || {
                reader.out_neighbors(crate::dataset::NODE_LABEL, EDGE_LABEL, &id)
            })
            .await
            .map_err(|e| grust::GrustError::Backend(e.to_string()))?
            .map(|ids| ids.into_iter().map(NodeId::new).collect());
        }
        let nodes = self.store.traverse(Traversal::from_node(v.clone()).out(EDGE_LABEL)).await?;
        Ok(nodes.into_iter().map(|n| n.id).collect())
    }

    /// Out-degree of one vertex through the same read path as `neighbors`.
    pub async fn out_degree(&self, from: &NodeId) -> grust::Result<usize> {
        #[cfg(feature = "falkor")]
        if let Some(reader) = &self.falkor {
            let reader = reader.clone();
            let id = from.as_str().to_string();
            return tokio::task::spawn_blocking(move || {
                reader.out_degree(crate::dataset::NODE_LABEL, EDGE_LABEL, &id)
            })
            .await
            .map_err(|e| grust::GrustError::Backend(e.to_string()))?;
        }
        Ok(self.out_edges(from).await?.len())
    }

    /// Which read path `neighbors`/`out_degree` use, recorded in reports.
    pub fn read_path(&self) -> &'static str {
        #[cfg(feature = "falkor")]
        if self.falkor.is_some() {
            return "harness-native-cypher";
        }
        #[cfg(feature = "neo4j")]
        if self.kind == BackendKind::Neo4j {
            return "harness-native-cypher";
        }
        "grust-portable-api"
    }

    pub async fn out_edges(&self, from: &NodeId) -> grust::Result<Vec<Edge>> {
        self.store
            .get_edges(EdgeQuery { from: Some(from.clone()), to: None, label: Some(EDGE_LABEL.into()) })
            .await
    }

    /// Count of edges leaving `from` after reopening the durable store from
    /// disk (Turso) or re-reading the shared memory store.
    pub async fn out_degree_after_reopen(&self, from: &NodeId) -> grust::Result<usize> {
        if self.kind.is_turso() {
            let path = self.turso_path.as_ref().expect("turso path");
            let store = Self::connect_turso(self.kind, path).await?;
            return Ok(store
                .get_edges(EdgeQuery { from: Some(from.clone()), to: None, label: Some(EDGE_LABEL.into()) })
                .await?
                .len());
        }
        #[cfg(feature = "falkor")]
        if self.falkor.is_some() {
            return self.out_degree(from).await;
        }
        // Network backends: a fresh handle is a fresh connection to the same
        // durable state; embedded memory/Lance stores re-read in place.
        let handle = self.extra_handle().await?;
        Ok(handle
            .get_edges(EdgeQuery { from: Some(from.clone()), to: None, label: Some(EDGE_LABEL.into()) })
            .await?
            .len())
    }

    /// Whether an error means the backend does not implement the operation
    /// (reported as `unsupported`, never as a crash).
    pub fn is_unsupported(err: &grust::GrustError) -> bool {
        matches!(err, grust::GrustError::Unsupported(_))
    }
}
