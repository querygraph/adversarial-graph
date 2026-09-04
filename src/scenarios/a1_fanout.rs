//! A1 — super-node fan-out. k-hop neighbourhood layers from the maximum
//! out-degree vertex must match the GraphIndex oracle exactly; latency per
//! hop is recorded. Layers: L0–L2 (store + traversal IR).

use std::time::Instant;

use crate::report::{ScenarioResult, histogram, record, Latency};
use super::Ctx;

pub async fn run(ctx: &Ctx<'_>) -> ScenarioResult {
    let mut r = ScenarioResult::new("A1", ctx.backend.kind.name(), ctx.dataset);
    let (hub, degree) = ctx.oracle.max_out_degree_vertex();
    let k = if ctx.smoke || degree > 100_000 { 1 } else { 2 };
    let (expected_layers, expected_total) = ctx.oracle.khop_layers(&hub, k);
    r.observe("hub", hub.as_str());
    r.observe("hub_out_degree", degree);
    r.observe("k", k);
    r.observe("expected_layers", &expected_layers);
    r.observe("expected_distinct", expected_total);

    let mut h = histogram();
    let mut rounds = if ctx.smoke { 1 } else { 3 };
    if degree > 500_000 {
        rounds = 1;
    }
    let mut last = Vec::new();
    for _ in 0..rounds {
        let t = Instant::now();
        match ctx.backend.khop(&hub, k).await {
            Ok(layers) => {
                record(&mut h, t.elapsed());
                last = layers;
            }
            Err(e) => {
                if crate::backends::Backend::is_unsupported(&e) {
                r.unsupported(&format!("backend cannot traverse: {e}"));
                return r;
            }
            r.gates.oom_or_crash += 1;
                r.notes.push(format!("khop failed: {e}"));
                return r;
            }
        }
    }
    r.observe("observed_layers", &last);
    if last != expected_layers {
        r.gates.wrong_answer += 1;
        r.notes.push(format!("layers {last:?} != oracle {expected_layers:?}"));
    }
    r.latency = Some(Latency::from_histogram(&h));
    r
}
