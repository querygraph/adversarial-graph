pub mod a12_cold_start;
pub mod a1_fanout;
pub mod a2_deep_paths;
pub mod a3_policy_bounds;
pub mod a4_hot_node;
pub mod a5_recursive_deletes;
pub mod a6_isolation;
pub mod a7_guarded_replay;
pub mod a8_differential;

use std::sync::atomic::AtomicUsize;

use crate::backends::Backend;
use crate::oracle::Oracle;
use crate::report::ScenarioResult;

/// Shared inputs for one scenario run.
pub struct Ctx<'a> {
    pub dataset: &'a str,
    /// The loader's format (`snap-edge-list`, `ldbc-snb-csvbasic`, …).
    pub format: &'a str,
    /// The parsed graph, absent under the compact reference (SNAP tiers
    /// above the host's `Graph` budget); the typed families require it.
    pub graph: Option<&'a grust::Graph>,
    /// The compact reference when `graph` is absent.
    pub compact: Option<&'a crate::compact::CompactGraph>,
    pub oracle: &'a Oracle<'a>,
    pub backend: &'a Backend,
    pub smoke: bool,
    /// Edges the hot-node family (A4) has appended to the hub in this
    /// process: later families that read the hub add it to the oracle
    /// degree instead of counting accepted writes as wrong answers.
    pub hub_writes: &'a AtomicUsize,
}

impl Ctx<'_> {
    /// The parsed graph for a typed family, which only runs on typed
    /// datasets and those always materialize (`dataset::load_dataset`).
    pub fn typed_graph(&self) -> &grust::Graph {
        self.graph
            .expect("typed families run over materialized typed datasets")
    }

    /// A `Graph` for the reference executor's policy checks: the memory
    /// store's whole graph, or under the compact reference a prefix of at
    /// most `max_edges` edges with the vertices they touch (the policy
    /// refusals are decided on the query, not the data). The second
    /// element names which, for the row.
    pub fn policy_graph(
        &self,
        memory: &grust::MemoryGraphStore,
        max_edges: usize,
    ) -> (grust::Graph, String) {
        match self.compact {
            Some(compact) => {
                let g = compact.prefix_subgraph(max_edges);
                let name = format!(
                    "prefix-subgraph({} nodes, {} edges)",
                    g.nodes.len(),
                    g.edges.len()
                );
                (g, name)
            }
            None => (memory.graph(), "memory-store".to_string()),
        }
    }
}

/// Every scenario, in run order. A8 (the read-only differential against
/// the reference over the loaded graph) runs before the mutating typed
/// families A5 and A6: on 2026-09-09 the first typed tiers showed every
/// store agreeing with each other and disagreeing with the reference by
/// A5's deletes and A6's upserts (14 "wrong answers" per cell), while A8
/// alone on a pristine store matched on every query.
pub fn all() -> &'static [&'static str] {
    &["A1", "A2", "A3", "A4", "A8", "A5", "A6", "A7", "A12"]
}

#[cfg(test)]
mod order_tests {
    #[test]
    fn the_differential_runs_before_the_mutating_families() {
        let order = super::all();
        let at = |id: &str| order.iter().position(|s| *s == id).unwrap();
        assert!(at("A8") < at("A5"));
        assert!(at("A8") < at("A6"));
    }
}

/// Whether a scenario is defined over the dataset's shape: the M1 families
/// over the single-label, single-type SNAP view (and the adapters index
/// that shape), the M2 families over labelled nodes and typed
/// relationships. A scenario that does not apply is skipped and named in
/// the dataset's report block, never run as a pass.
pub fn applies(id: &str, typed: bool) -> bool {
    match id {
        "A5" | "A6" | "A8" => typed,
        _ => !typed,
    }
}

pub async fn run(id: &str, ctx: &Ctx<'_>) -> ScenarioResult {
    let started = std::time::Instant::now();
    let probe = crate::probe::Probe::start(ctx.backend.kind.container());
    let mut result = match id {
        "A1" => a1_fanout::run(ctx).await,
        "A2" => a2_deep_paths::run(ctx).await,
        "A3" => a3_policy_bounds::run(ctx).await,
        "A4" => a4_hot_node::run(ctx).await,
        "A5" => a5_recursive_deletes::run(ctx).await,
        "A6" => a6_isolation::run(ctx).await,
        "A7" => a7_guarded_replay::run(ctx).await,
        "A12" => a12_cold_start::run(ctx).await,
        "A8" => a8_differential::run(ctx).await,
        other => {
            let mut r = ScenarioResult::new(other, ctx.backend.kind.name(), ctx.dataset);
            r.unsupported("unknown scenario id");
            r
        }
    };
    result.wall_ms = started.elapsed().as_millis();
    result.observe("read_path", ctx.backend.read_path());
    if let Some(profile) = ctx.backend.kind.profile() {
        result.observe("profile", profile);
    }
    probe.finish(&mut result);
    result.finish();
    result
}
