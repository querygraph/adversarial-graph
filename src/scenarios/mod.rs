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
    pub graph: &'a grust::Graph,
    pub oracle: &'a Oracle<'a>,
    pub backend: &'a Backend,
    pub smoke: bool,
    /// Edges the hot-node family (A4) has appended to the hub in this
    /// process: later families that read the hub add it to the oracle
    /// degree instead of counting accepted writes as wrong answers.
    pub hub_writes: &'a AtomicUsize,
}

pub fn all() -> &'static [&'static str] {
    &["A1", "A2", "A3", "A4", "A5", "A6", "A7", "A8", "A12"]
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
