pub mod a1_fanout;
pub mod a2_deep_paths;
pub mod a3_policy_bounds;
pub mod a4_hot_node;
pub mod a7_guarded_replay;

use crate::backends::Backend;
use crate::oracle::Oracle;
use crate::report::ScenarioResult;

/// Shared inputs for one scenario run.
pub struct Ctx<'a> {
    pub dataset: &'a str,
    pub graph: &'a grust::Graph,
    pub oracle: &'a Oracle<'a>,
    pub backend: &'a Backend,
    pub smoke: bool,
}

pub fn all() -> &'static [&'static str] {
    &["A1", "A2", "A3", "A4", "A7"]
}

/// Whether a scenario is defined over labelled nodes and typed relationships
/// (the M2 families) rather than the single-label SNAP shape.
pub fn accepts_typed(id: &str) -> bool {
    matches!(id, "A5" | "A6" | "A8" | "A12")
}

pub async fn run(id: &str, ctx: &Ctx<'_>) -> ScenarioResult {
    let started = std::time::Instant::now();
    let probe = crate::probe::Probe::start(ctx.backend.kind.container());
    let mut result = match id {
        "A1" => a1_fanout::run(ctx).await,
        "A2" => a2_deep_paths::run(ctx).await,
        "A3" => a3_policy_bounds::run(ctx).await,
        "A4" => a4_hot_node::run(ctx).await,
        "A7" => a7_guarded_replay::run(ctx).await,
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
