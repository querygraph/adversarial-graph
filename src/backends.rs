//! Systems under test. Each backend is reached through the published
//! `grust-graph` 0.13 API, or through a Grust internal adapter pinned to the
//! same release tag, so the harness never depends on a Grust checkout. Where
//! a system offers both an HTTP API and a Rust SDK, both are separate
//! backends so the transport cost is measured rather than assumed.

use std::path::PathBuf;
use std::sync::Arc;

#[allow(unused_imports)]
use grust::GraphAdminStore as _;
use grust::{Edge, EdgeQuery, Graph, GraphAdminStore, GraphStore, NodeId, Traversal};
use grust::{TursoConfig, TursoGraphStore, TursoJournalMode};

use crate::dataset::EDGE_LABEL;
#[cfg(feature = "falkor")]
use crate::dataset::NODE_LABEL;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendKind {
    Memory,
    TursoWal,
    TursoMvcc,
    #[cfg(feature = "postgres")]
    Postgres,
    #[cfg(feature = "surreal")]
    SurrealHttp,
    #[cfg(feature = "surreal")]
    SurrealSdk,
    #[cfg(feature = "falkor")]
    Falkor,
    #[cfg(feature = "lancedb")]
    LanceDb,
    #[cfg(feature = "ladybug")]
    Ladybug,
    #[cfg(feature = "helix")]
    HelixHttp,
    #[cfg(feature = "helix")]
    HelixSdk,
    #[cfg(feature = "neo4j")]
    Neo4j,
    #[cfg(feature = "neo4j")]
    Neo4jHttp,
    /// Memgraph over Bolt through the same harness-side store as Neo4j.
    #[cfg(feature = "neo4j")]
    Memgraph,
    /// Apache AGE: openCypher through PostgreSQL's `cypher()` function.
    #[cfg(feature = "age")]
    Age,
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
            "surreal-http" => Some(Self::SurrealHttp),
            #[cfg(feature = "surreal")]
            "surreal" | "surreal-sdk" => Some(Self::SurrealSdk),
            #[cfg(feature = "falkor")]
            "falkor" => Some(Self::Falkor),
            #[cfg(feature = "lancedb")]
            "lancedb" => Some(Self::LanceDb),
            #[cfg(feature = "ladybug")]
            "ladybug" => Some(Self::Ladybug),
            #[cfg(feature = "helix")]
            "helix-http" => Some(Self::HelixHttp),
            #[cfg(feature = "helix")]
            "helix" | "helix-sdk" => Some(Self::HelixSdk),
            #[cfg(feature = "neo4j")]
            "neo4j" | "neo4j-bolt" => Some(Self::Neo4j),
            #[cfg(feature = "neo4j")]
            "neo4j-http" => Some(Self::Neo4jHttp),
            #[cfg(feature = "neo4j")]
            "memgraph" => Some(Self::Memgraph),
            #[cfg(feature = "age")]
            "age" | "apache-age" => Some(Self::Age),
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
            Self::SurrealHttp => "surreal-http",
            #[cfg(feature = "surreal")]
            Self::SurrealSdk => "surreal-sdk",
            #[cfg(feature = "falkor")]
            Self::Falkor => "falkor",
            #[cfg(feature = "lancedb")]
            Self::LanceDb => "lancedb",
            #[cfg(feature = "ladybug")]
            Self::Ladybug => "ladybug",
            #[cfg(feature = "helix")]
            Self::HelixHttp => "helix-http",
            #[cfg(feature = "helix")]
            Self::HelixSdk => "helix-sdk",
            #[cfg(feature = "neo4j")]
            Self::Neo4j => "neo4j",
            #[cfg(feature = "neo4j")]
            Self::Neo4jHttp => "neo4j-http",
            #[cfg(feature = "neo4j")]
            Self::Memgraph => "memgraph",
            #[cfg(feature = "age")]
            Self::Age => "age",
        }
    }
    /// How the harness reaches the system: recorded per run so HTTP and
    /// SDK rows of the same engine can be compared.
    pub fn transport(self) -> &'static str {
        match self {
            Self::Memory => "embedded",
            Self::TursoWal | Self::TursoMvcc => "embedded",
            #[cfg(feature = "postgres")]
            Self::Postgres => "pg-wire",
            #[cfg(feature = "surreal")]
            Self::SurrealHttp => "http-sql",
            #[cfg(feature = "surreal")]
            Self::SurrealSdk => "rust-sdk-ws",
            #[cfg(feature = "falkor")]
            Self::Falkor => "resp",
            #[cfg(feature = "lancedb")]
            Self::LanceDb => "embedded",
            #[cfg(feature = "ladybug")]
            Self::Ladybug => "embedded",
            #[cfg(feature = "helix")]
            Self::HelixHttp => "http-json",
            #[cfg(feature = "helix")]
            Self::HelixSdk => "rust-sdk-http",
            #[cfg(feature = "neo4j")]
            Self::Neo4j => "bolt",
            #[cfg(feature = "neo4j")]
            Self::Neo4jHttp => "http-query-api",
            #[cfg(feature = "neo4j")]
            Self::Memgraph => "bolt",
            #[cfg(feature = "age")]
            Self::Age => "pg-wire",
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
            Self::SurrealHttp,
            #[cfg(feature = "surreal")]
            Self::SurrealSdk,
            #[cfg(feature = "falkor")]
            Self::Falkor,
            #[cfg(feature = "lancedb")]
            Self::LanceDb,
            #[cfg(feature = "ladybug")]
            Self::Ladybug,
            #[cfg(feature = "helix")]
            Self::HelixHttp,
            #[cfg(feature = "helix")]
            Self::HelixSdk,
            #[cfg(feature = "neo4j")]
            Self::Neo4j,
            #[cfg(feature = "neo4j")]
            Self::Neo4jHttp,
            #[cfg(feature = "neo4j")]
            Self::Memgraph,
            #[cfg(feature = "age")]
            Self::Age,
        ]
    }
    /// Docker container serving this backend, if any (for resource probes).
    /// The server-side configuration a run was taken under, when the harness
    /// varies it: FalkorDB's `RESULTSET_SIZE` (the image default of 10,000
    /// silently truncates results; `-1` is the tuned profile). Recorded on
    /// every row so runs under different profiles never overwrite each other
    /// in `RESULTS.md`.
    pub fn profile(self) -> Option<String> {
        match self {
            #[cfg(feature = "falkor")]
            Self::Falkor => Some(format!(
                "resultset_size={}",
                env_or("FALKOR_RESULTSET_SIZE", "10000")
            )),
            #[cfg(feature = "ladybug")]
            Self::Ladybug => Some(format!(
                "buffer_pool_bytes={},concurrent_writes={}",
                ladybug_buffer_pool_bytes(),
                ladybug_concurrent_writes()
            )),
            _ => None,
        }
    }
    pub fn container(self) -> Option<&'static str> {
        match self {
            #[cfg(feature = "postgres")]
            Self::Postgres => Some("adversarial-graph-postgres-1"),
            #[cfg(feature = "surreal")]
            Self::SurrealHttp | Self::SurrealSdk => Some("adversarial-graph-surreal-1"),
            #[cfg(feature = "falkor")]
            Self::Falkor => Some("adversarial-graph-falkor-1"),
            #[cfg(feature = "helix")]
            Self::HelixHttp | Self::HelixSdk => Some("adversarial-graph-helix-1"),
            #[cfg(feature = "neo4j")]
            Self::Neo4j | Self::Neo4jHttp => Some("adversarial-graph-neo4j-1"),
            #[cfg(feature = "neo4j")]
            Self::Memgraph => Some("adversarial-graph-memgraph-1"),
            #[cfg(feature = "age")]
            Self::Age => Some("adversarial-graph-age-1"),
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

/// A tag as an identifier every store accepts: lowercase, `[a-z0-9_]`
/// only, so `ldbc-snb-sf0.1` is `ldbc_snb_sf0_1`.
fn slug(tag: &str) -> String {
    tag.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(feature = "postgres")]
async fn connect_postgres(tag: &str) -> grust::Result<grust::PostgresGraphStore> {
    grust::PostgresGraphStore::connect(grust::PostgresGraphConfig {
        connection_string: env_or(
            "AG_POSTGRES_URL",
            "host=127.0.0.1 port=15432 user=postgres password=postgres dbname=graph",
        ),
        schema: "public".to_string(),
        table_prefix: format!("ag_{}", slug(tag)),
        batch_size: 500,
    })
    .await
}

#[cfg(feature = "surreal")]
fn surreal_config(tag: &str, transport: &str) -> grust::SurrealConfig {
    grust::SurrealConfig {
        url: env_or("AG_SURREAL_URL", "http://127.0.0.1:18000/sql"),
        user: "root".to_string(),
        pass: "root".to_string(),
        namespace: "ag".to_string(),
        database: format!("ag_{}_{transport}", slug(tag)),
        batch_size: 500,
        labels: vec![NODE_LABEL.to_string()],
        relationships: vec![EDGE_LABEL.to_string()],
    }
}

/// The HTTP store posts SurrealQL to `/sql`; the SDK store speaks the native
/// WebSocket protocol through the `surrealdb` crate. Same server, same
/// adapter logic, different transport.
#[cfg(feature = "surreal")]
async fn connect_surreal(kind: BackendKind, tag: &str) -> grust::Result<Arc<dyn AdminStore>> {
    Ok(match kind {
        BackendKind::SurrealHttp => Arc::new(grust::SurrealHttpGraphStore::connect(
            surreal_config(tag, "http"),
        )?),
        _ => Arc::new(grust::SurrealSdkGraphStore::connect(surreal_config(tag, "sdk")).await?),
    })
}

#[cfg(feature = "falkor")]
fn connect_falkor(tag: &str) -> grust::FalkorGraphStore {
    grust::FalkorGraphStore::new(grust::FalkorConfig {
        redis_url: env_or("AG_FALKOR_URL", "redis://127.0.0.1:16379"),
        graph: format!("ag_{}", slug(tag)),
        batch_size: 1_000,
        pool_size: 16,
        id_property: "id".to_string(),
        labels_property: "labels".to_string(),
    })
}

/// HelixDB through Grust's internal adapter: the HTTP store posts dynamic
/// queries to `/v1/query`; the SDK store sends the same requests through the
/// `helix-db` client crate.
#[cfg(feature = "helix")]
fn connect_helix(kind: BackendKind) -> grust::Result<Arc<dyn AdminStore>> {
    let base = helix_base_url();
    Ok(match kind {
        BackendKind::HelixHttp => Arc::new(grust_helix::HelixHttpGraphStore::connect(
            grust_helix::HelixHttpConfig {
                query_url: format!("{}/v1/query", base.trim_end_matches('/')),
                batch_size: 500,
                labels: vec![NODE_LABEL.to_string()],
            },
        )?),
        _ => Arc::new(grust_helix::HelixSdkGraphStore::connect(
            grust_helix::HelixSdkConfig {
                base_url: base,
                batch_size: 500,
                labels: vec![NODE_LABEL.to_string()],
            },
        )?),
    })
}

#[cfg(feature = "helix")]
fn helix_base_url() -> String {
    env_or("AG_HELIX_URL", "http://127.0.0.1:18082")
}

/// Helix answers `NWhere id = …` by scanning every node unless a runtime
/// equality index exists on the property, and `grust-helix` writes each edge
/// as two such filters, so without the index a 500-edge batch on a 145k-node
/// slice outruns the gateway's 30 s request timeout and the load fails with
/// 408. Create the index at bootstrap, as the harness does for FalkorDB and
/// Neo4j, so the engine and not the missing index is what gets measured.
#[cfg(feature = "helix")]
async fn helix_create_id_index() -> grust::Result<()> {
    use grust::GrustError::Backend;
    let request = serde_json::json!({
        "request_type": "write",
        "query": {
            "queries": [{"Query": {
                "name": "id_index",
                "steps": [{"CreateIndex": {
                    "spec": {"NodeEquality": {"label": NODE_LABEL, "property": "id", "unique": false}},
                    "if_not_exists": true
                }}],
                "condition": null
            }}],
            "returns": []
        },
        "parameters": {},
        "parameter_types": {}
    });
    let url = format!("{}/v1/query", helix_base_url().trim_end_matches('/'));
    let response = reqwest::Client::new()
        .post(&url)
        .json(&request)
        .send()
        .await
        .map_err(|e| Backend(format!("Helix index request failed: {e}")))?;
    let status = response.status();
    if status.is_success() {
        return Ok(());
    }
    let body = response.text().await.unwrap_or_default();
    Err(Backend(format!(
        "Helix index creation failed with status {status}: {body}"
    )))
}

/// LadybugDB embedded through Grust's internal adapter (the `lbug` crate),
/// on-disk under the run's work directory, untyped mode so the harness
/// labels create their tables on first write.
#[cfg(feature = "ladybug")]
fn connect_ladybug(
    work_dir: &std::path::Path,
    tag: &str,
) -> grust::Result<grust_ladybug::LadybugGraphStore> {
    let dir = work_dir.join(format!("ladybug-{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    grust_ladybug::LadybugGraphStore::new(grust_ladybug::LadybugConfig {
        path: grust_ladybug::LadybugPath::Directory(dir),
        table_prefix: "ag".to_string(),
        dynamic_schema: true,
        query_timeout_ms: None,
        buffer_pool_bytes: Some(ladybug_buffer_pool_bytes()),
        concurrent_writes: ladybug_concurrent_writes(),
    })
}

/// The engine's multi-writer mode is off by default; `AG_LADYBUG_CONCURRENT_WRITES=1`
/// selects the tuned profile where the adapter lets writers run concurrently.
#[cfg(feature = "ladybug")]
fn ladybug_concurrent_writes() -> bool {
    env_or("AG_LADYBUG_CONCURRENT_WRITES", "0") == "1"
}

/// The engine sizes its buffer pool from host RAM by default (about 6 GB
/// resident here for a 200k-edge slice); the harness caps it like any other
/// store's memory budget and records the cap in the row's profile.
#[cfg(feature = "ladybug")]
fn ladybug_buffer_pool_bytes() -> u64 {
    env_or("AG_LADYBUG_BUFFER_POOL_BYTES", "4294967296")
        .parse()
        .expect("AG_LADYBUG_BUFFER_POOL_BYTES must be a byte count")
}

#[cfg(feature = "neo4j")]
fn connect_neo4j_http() -> grust::Result<crate::neo4j_http::Neo4jHttpStore> {
    crate::neo4j_http::Neo4jHttpStore::connect(
        &env_or("AG_NEO4J_HTTP_URL", "http://127.0.0.1:17474"),
        &env_or("AG_NEO4J_USER", "neo4j"),
        &env_or("AG_NEO4J_PASS", "adversarial"),
    )
}

/// Neo4j over Bolt, or Memgraph through the same store with its dialect.
#[cfg(feature = "neo4j")]
async fn connect_neo4j_bolt(kind: BackendKind) -> grust::Result<crate::neo4j::Neo4jStore> {
    if kind == BackendKind::Memgraph {
        return crate::neo4j::Neo4jStore::connect_dialect(
            &env_or("AG_MEMGRAPH_URI", "bolt://127.0.0.1:17688"),
            &env_or("AG_MEMGRAPH_USER", ""),
            &env_or("AG_MEMGRAPH_PASS", ""),
            crate::neo4j::BoltDialect::Memgraph,
        )
        .await;
    }
    crate::neo4j::Neo4jStore::connect(
        &env_or("AG_NEO4J_URI", "bolt://127.0.0.1:17687"),
        &env_or("AG_NEO4J_USER", "neo4j"),
        &env_or("AG_NEO4J_PASS", "adversarial"),
    )
    .await
}

#[cfg(feature = "age")]
async fn connect_age() -> grust::Result<Arc<dyn AdminStore>> {
    let pool: usize = env_or("AG_AGE_POOL", "16")
        .parse()
        .expect("AG_AGE_POOL must be a connection count");
    Ok(Arc::new(
        crate::age::AgeStore::connect(
            &env_or(
                "AG_AGE_URL",
                "host=127.0.0.1 port=55434 user=postgres password=postgres dbname=graph",
            ),
            &env_or("AG_AGE_GRAPH", "adversarial"),
            pool,
        )
        .await?,
    ))
}

#[cfg(feature = "lancedb")]
async fn connect_lancedb(
    work_dir: &std::path::Path,
    tag: &str,
) -> grust::Result<grust::LanceDbGraphStore> {
    grust::LanceDbGraphStore::connect(grust::LanceDbConfig {
        uri: work_dir
            .join(format!("lancedb-{tag}"))
            .display()
            .to_string(),
        table_prefix: "ag".to_string(),
        batch_size: 500,
    })
    .await
}

/// Object-safe view of a store the harness can bootstrap and clear.
pub trait AdminStore: GraphAdminStore + Send + Sync {}
impl<T: GraphAdminStore + Send + Sync> AdminStore for T {}

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
    #[cfg(feature = "postgres")]
    pub postgres: Option<Arc<grust::PostgresGraphStore>>,
    #[cfg(feature = "neo4j")]
    pub neo4j: Option<crate::neo4j::Neo4jStore>,
    #[cfg(feature = "neo4j")]
    pub neo4j_http: Option<crate::neo4j_http::Neo4jHttpStore>,
}

impl Backend {
    fn plain(kind: BackendKind, store: Arc<dyn GraphStore>, tag: &str) -> Self {
        Self {
            kind,
            store,
            memory: None,
            turso: None,
            turso_path: None,
            tag: tag.to_string(),
            #[cfg(feature = "falkor")]
            falkor: None,
            #[cfg(feature = "postgres")]
            postgres: None,
            #[cfg(feature = "neo4j")]
            neo4j: None,
            #[cfg(feature = "neo4j")]
            neo4j_http: None,
        }
    }

    /// Bootstrap and clear a network or embedded store, then wrap it.
    async fn prepared(
        kind: BackendKind,
        store: Arc<dyn AdminStore>,
        tag: &str,
    ) -> grust::Result<Self> {
        store.bootstrap().await?;
        store.clear().await?;
        let dyn_store: Arc<dyn GraphStore> = store;
        Ok(Self::plain(kind, dyn_store, tag))
    }

    pub async fn open(
        kind: BackendKind,
        work_dir: &std::path::Path,
        tag: &str,
    ) -> grust::Result<Self> {
        let mkdir = || {
            std::fs::create_dir_all(work_dir).map_err(|e| grust::GrustError::Backend(e.to_string()))
        };
        match kind {
            BackendKind::Memory => {
                let store = grust::MemoryGraphStore::new();
                let mut b = Self::plain(kind, Arc::new(store.clone()), tag);
                b.memory = Some(store);
                Ok(b)
            }
            #[cfg(feature = "postgres")]
            BackendKind::Postgres => {
                let store = Arc::new(connect_postgres(tag).await?);
                let mut b = Self::prepared(kind, store.clone(), tag).await?;
                b.postgres = Some(store);
                Ok(b)
            }
            #[cfg(feature = "surreal")]
            BackendKind::SurrealHttp | BackendKind::SurrealSdk => {
                Self::prepared(kind, connect_surreal(kind, tag).await?, tag).await
            }
            #[cfg(feature = "falkor")]
            BackendKind::Falkor => {
                let mut b = Self::prepared(kind, Arc::new(connect_falkor(tag)), tag).await?;
                let reader = crate::falkor_reader::FalkorReader::new(
                    &env_or("AG_FALKOR_URL", "redis://127.0.0.1:16379"),
                    &format!("ag_{}", slug(tag)),
                )?;
                reader.ensure_index(NODE_LABEL)?;
                b.falkor = Some(reader);
                Ok(b)
            }
            #[cfg(feature = "helix")]
            BackendKind::HelixHttp | BackendKind::HelixSdk => {
                let b = Self::prepared(kind, connect_helix(kind)?, tag).await?;
                helix_create_id_index().await?;
                Ok(b)
            }
            #[cfg(feature = "ladybug")]
            BackendKind::Ladybug => {
                mkdir()?;
                Self::prepared(kind, Arc::new(connect_ladybug(work_dir, tag)?), tag).await
            }
            #[cfg(feature = "neo4j")]
            BackendKind::Neo4jHttp => {
                let store = connect_neo4j_http()?;
                let mut b = Self::prepared(kind, Arc::new(store.clone()), tag).await?;
                b.neo4j_http = Some(store);
                Ok(b)
            }
            #[cfg(feature = "neo4j")]
            BackendKind::Neo4j | BackendKind::Memgraph => {
                let store = connect_neo4j_bolt(kind).await?;
                let mut b = Self::prepared(kind, Arc::new(store.clone()), tag).await?;
                b.neo4j = Some(store);
                Ok(b)
            }
            #[cfg(feature = "age")]
            BackendKind::Age => Self::prepared(kind, connect_age().await?, tag).await,
            #[cfg(feature = "lancedb")]
            BackendKind::LanceDb => {
                mkdir()?;
                Self::prepared(kind, Arc::new(connect_lancedb(work_dir, tag).await?), tag).await
            }
            BackendKind::TursoWal | BackendKind::TursoMvcc => {
                mkdir()?;
                let path = work_dir.join(format!("{}-{}.db", kind.name(), tag));
                for suffix in ["", "-wal", "-shm", "-log"] {
                    let _ = std::fs::remove_file(format!("{}{}", path.display(), suffix));
                }
                let store = Arc::new(Self::connect_turso(kind, &path).await?);
                store.bootstrap().await?;
                let mut b = Self::plain(kind, store.clone(), tag);
                b.turso = Some(store);
                b.turso_path = Some(path);
                Ok(b)
            }
        }
    }

    pub async fn connect_turso(
        kind: BackendKind,
        path: &std::path::Path,
    ) -> grust::Result<TursoGraphStore> {
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
    /// handle per writer). Memory and embedded stores are shared; client
    /// drivers that pool connections are shared too.
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
            BackendKind::SurrealHttp | BackendKind::SurrealSdk => {
                let s: Arc<dyn GraphStore> = connect_surreal(self.kind, &self.tag).await?;
                Ok(s)
            }
            #[cfg(feature = "falkor")]
            BackendKind::Falkor => Ok(Arc::new(connect_falkor(&self.tag))),
            #[cfg(feature = "lancedb")]
            BackendKind::LanceDb => Ok(self.store.clone()),
            #[cfg(feature = "ladybug")]
            BackendKind::Ladybug => Ok(self.store.clone()), // one embedded database per process
            #[cfg(feature = "helix")]
            BackendKind::HelixHttp | BackendKind::HelixSdk => {
                let s: Arc<dyn GraphStore> = connect_helix(self.kind)?;
                Ok(s)
            }
            #[cfg(feature = "neo4j")]
            BackendKind::Neo4j | BackendKind::Neo4jHttp | BackendKind::Memgraph => {
                Ok(self.store.clone())
            } // the clients pool connections
            #[cfg(feature = "age")]
            BackendKind::Age => Ok(self.store.clone()), // round-robin connection pool
        }
    }

    pub async fn load(&self, graph: &Graph) -> grust::Result<grust::LoadReport> {
        #[cfg(feature = "falkor")]
        if let Some(reader) = &self.falkor {
            let reader = reader.clone();
            let graph = graph.clone();
            return tokio::task::spawn_blocking(move || reader.load_graph(&graph))
                .await
                .map_err(|e| grust::GrustError::Backend(e.to_string()))?;
        }
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
                reader.out_neighbors(NODE_LABEL, EDGE_LABEL, &id)
            })
            .await
            .map_err(|e| grust::GrustError::Backend(e.to_string()))?
            .map(|ids| ids.into_iter().map(NodeId::new).collect());
        }
        self.store
            .traverse_ids(Traversal::from_node(v.clone()).out(EDGE_LABEL))
            .await
    }

    /// Out-degree of one vertex through the same read path as `neighbors`.
    pub async fn out_degree(&self, from: &NodeId) -> grust::Result<usize> {
        #[cfg(feature = "falkor")]
        if let Some(reader) = &self.falkor {
            let reader = reader.clone();
            let id = from.as_str().to_string();
            return tokio::task::spawn_blocking(move || {
                reader.out_degree(NODE_LABEL, EDGE_LABEL, &id)
            })
            .await
            .map_err(|e| grust::GrustError::Backend(e.to_string()))?;
        }
        Ok(self.out_edges(from).await?.len())
    }

    /// Run a read-only Cypher query through the store's own query path and
    /// return every row: Grust's reference executor or resident index for
    /// Memory, Grust pushdown for Turso and PostgreSQL, the engine's own
    /// openCypher for FalkorDB and Neo4j. Stores without a Cypher path
    /// return `Unsupported`.
    pub async fn cypher(&self, cypher: &str) -> grust::Result<crate::differential::ResultSet> {
        use crate::differential::{ResultSet, Route, resident_proven};
        let run_indexed = |index: Arc<grust::TypedGraphIndex>, cypher: String| async move {
            tokio::task::spawn_blocking(move || {
                grust_cypher::read::run_read_query_indexed(
                    &index,
                    &cypher,
                    &grust_cypher::CypherParameters::new(),
                )
            })
            .await
            .map_err(|e| grust::GrustError::Backend(e.to_string()))?
            .map(ResultSet::from_table)
        };
        if let Some(memory) = &self.memory {
            if resident_proven(cypher) {
                return run_indexed(memory.indexed_snapshot()?, cypher.to_string()).await;
            }
            let graph = memory.graph();
            let cypher = cypher.to_string();
            return tokio::task::spawn_blocking(move || {
                grust_cypher::read::run_read_query(
                    &graph,
                    &cypher,
                    &grust_cypher::CypherParameters::new(),
                )
            })
            .await
            .map_err(|e| grust::GrustError::Backend(e.to_string()))?
            .map(ResultSet::from_table);
        }
        let params = grust_cypher::CypherParameters::new();
        if let Some(turso) = &self.turso {
            if self.cypher_route(cypher) == Route::ResidentIndexRustCount {
                return run_indexed(turso.indexed_snapshot().await?, cypher.to_string()).await;
            }
            return Ok(ResultSet::from_table(
                turso.run_read_query(cypher, &params).await?,
            ));
        }
        #[cfg(feature = "postgres")]
        if let Some(postgres) = &self.postgres {
            if self.cypher_route(cypher) == Route::ResidentIndexRustCount {
                return run_indexed(postgres.indexed_snapshot().await?, cypher.to_string()).await;
            }
            return Ok(ResultSet::from_table(
                postgres.run_read_query(cypher, &params).await?,
            ));
        }
        #[cfg(feature = "falkor")]
        if let Some(reader) = &self.falkor {
            let reader = reader.clone();
            let cypher = cypher.to_string();
            return tokio::task::spawn_blocking(move || reader.rows(&cypher))
                .await
                .map_err(|e| grust::GrustError::Backend(e.to_string()))?;
        }
        #[cfg(feature = "neo4j")]
        if let Some(store) = &self.neo4j {
            return store.rows(cypher).await;
        }
        #[cfg(feature = "neo4j")]
        if let Some(store) = &self.neo4j_http {
            return store.rows(cypher).await;
        }
        Err(grust::GrustError::Unsupported(format!(
            "{} has no Cypher read path in this harness",
            self.kind.name()
        )))
    }

    /// The route `cypher` takes on this store, as the LSQB harness names it.
    pub fn cypher_route(&self, cypher: &str) -> crate::differential::Route {
        use crate::differential::{Route, resident_proven, sql_route};
        if self.memory.is_some() {
            return if resident_proven(cypher) {
                Route::ResidentIndexRustCount
            } else {
                Route::InProcessReference
            };
        }
        if self.turso.is_some() {
            // The resident plan comes before the store's own SQL, as in the
            // LSQB harness since 5c34fc2 (260 s vs 66 ms for q1 at SF0.1).
            if resident_proven(cypher) {
                return Route::ResidentIndexRustCount;
            }
            return sql_route(cypher, &grust::TursoReadDialect::new("ag"));
        }
        #[cfg(feature = "postgres")]
        if self.postgres.is_some() {
            if resident_proven(cypher) {
                return Route::ResidentIndexRustCount;
            }
            let config = grust::PostgresGraphConfig {
                schema: "public".to_string(),
                table_prefix: format!("ag_{}", slug(&self.tag)),
                ..grust::PostgresGraphConfig::default()
            };
            return sql_route(
                cypher,
                &grust_postgres_core::PostgresReadDialect::new(&config),
            );
        }
        Route::NativeCypher
    }

    /// Which read path `neighbors`/`out_degree` use, recorded in reports.
    pub fn read_path(&self) -> &'static str {
        #[cfg(feature = "falkor")]
        if self.falkor.is_some() {
            return "harness-native-cypher";
        }
        #[cfg(feature = "neo4j")]
        if matches!(
            self.kind,
            BackendKind::Neo4j | BackendKind::Neo4jHttp | BackendKind::Memgraph
        ) {
            return "harness-native-cypher";
        }
        #[cfg(feature = "age")]
        if matches!(self.kind, BackendKind::Age) {
            return "harness-native-cypher";
        }
        "grust-portable-api"
    }

    pub async fn out_edges(&self, from: &NodeId) -> grust::Result<Vec<Edge>> {
        self.store
            .get_edges(EdgeQuery {
                from: Some(from.clone()),
                to: None,
                label: Some(EDGE_LABEL.into()),
            })
            .await
    }

    /// Count of edges leaving `from` after reopening the durable store from
    /// disk (Turso) or re-reading the shared memory store.
    pub async fn out_degree_after_reopen(&self, from: &NodeId) -> grust::Result<usize> {
        if self.kind.is_turso() {
            let path = self.turso_path.as_ref().expect("turso path");
            let store = Self::connect_turso(self.kind, path).await?;
            return Ok(store
                .get_edges(EdgeQuery {
                    from: Some(from.clone()),
                    to: None,
                    label: Some(EDGE_LABEL.into()),
                })
                .await?
                .len());
        }
        #[cfg(feature = "falkor")]
        if self.falkor.is_some() {
            return self.out_degree(from).await;
        }
        // Network backends: a fresh handle is a fresh connection to the same
        // durable state; embedded memory/Lance/Ladybug stores re-read in place.
        let handle = self.extra_handle().await?;
        Ok(handle
            .get_edges(EdgeQuery {
                from: Some(from.clone()),
                to: None,
                label: Some(EDGE_LABEL.into()),
            })
            .await?
            .len())
    }

    /// Whether an error means the backend does not implement the operation
    /// (reported as `unsupported`, never as a crash).
    pub fn is_unsupported(err: &grust::GrustError) -> bool {
        matches!(err, grust::GrustError::Unsupported(_))
    }
}
