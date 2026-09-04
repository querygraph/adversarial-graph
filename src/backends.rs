//! Systems under test. Each backend is reached only through the published
//! `grust-graph` 0.13 API so the harness never depends on a Grust checkout.

use std::path::PathBuf;
use std::sync::Arc;

use grust::{Edge, EdgeQuery, Graph, GraphAdminStore, GraphStore, NodeId, Traversal};
use grust::{TursoConfig, TursoGraphStore, TursoJournalMode};

use crate::dataset::EDGE_LABEL;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendKind {
    Memory,
    TursoWal,
    TursoMvcc,
}

impl BackendKind {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "memory" => Some(Self::Memory),
            "turso" | "turso-wal" => Some(Self::TursoWal),
            "turso-mvcc" => Some(Self::TursoMvcc),
            _ => None,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Memory => "memory",
            Self::TursoWal => "turso-wal",
            Self::TursoMvcc => "turso-mvcc",
        }
    }
    pub fn all() -> &'static [BackendKind] {
        &[Self::Memory, Self::TursoWal, Self::TursoMvcc]
    }
}

/// A live handle to a backend plus what the harness needs to reopen it.
pub struct Backend {
    pub kind: BackendKind,
    pub store: Arc<dyn GraphStore>,
    pub memory: Option<grust::MemoryGraphStore>,
    pub turso: Option<Arc<TursoGraphStore>>,
    pub turso_path: Option<PathBuf>,
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
                })
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
            _ => {
                let path = self.turso_path.as_ref().expect("turso path");
                Ok(Arc::new(Self::connect_turso(self.kind, path).await?))
            }
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
                let nodes = self.store.traverse(Traversal::from_node(v.clone()).out(EDGE_LABEL)).await?;
                for n in nodes {
                    if visited.insert(n.id.clone()) {
                        next.push(n.id);
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

    pub async fn out_edges(&self, from: &NodeId) -> grust::Result<Vec<Edge>> {
        self.store
            .get_edges(EdgeQuery { from: Some(from.clone()), to: None, label: Some(EDGE_LABEL.into()) })
            .await
    }

    /// Count of edges leaving `from` after reopening the durable store from
    /// disk (Turso) or re-reading the shared memory store.
    pub async fn out_degree_after_reopen(&self, from: &NodeId) -> grust::Result<usize> {
        match self.kind {
            BackendKind::Memory => Ok(self.out_edges(from).await?.len()),
            _ => {
                let path = self.turso_path.as_ref().expect("turso path");
                let store = Self::connect_turso(self.kind, path).await?;
                Ok(store
                    .get_edges(EdgeQuery { from: Some(from.clone()), to: None, label: Some(EDGE_LABEL.into()) })
                    .await?
                    .len())
            }
        }
    }
}
