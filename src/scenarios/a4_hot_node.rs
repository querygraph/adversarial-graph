//! A4 — hot-node write contention. N writers each attach M new edges to the
//! same hub through their own store handle, concurrently. Every write must
//! be either durably applied or rejected with a typed conflict; the final
//! out-degree observed after reopening the store must equal the initial
//! degree plus the accepted count (`lost_write` / `duplicate_durable_mutation`
//! otherwise). Errors that are neither success nor conflict are counted as
//! `non_conflict_errors` (an observation) and, if any write is lost, as a
//! gate failure. Layers: L0, L1, L3.

use std::sync::Arc;
use std::time::Instant;

use grust::{Edge, GrustError, Props};
use tokio::sync::Barrier;

use crate::dataset::EDGE_LABEL;
use crate::report::{ScenarioResult, histogram, record, Latency};
use super::Ctx;

fn is_conflict(err: &GrustError) -> bool {
    let m = err.to_string().to_ascii_lowercase();
    m.contains("conflict") || m.contains("busy") || m.contains("locked") || m.contains("snapshot")
}

pub async fn run(ctx: &Ctx<'_>) -> ScenarioResult {
    let mut r = ScenarioResult::new("A4", ctx.backend.kind.name(), ctx.dataset);
    let writers = if ctx.smoke { 4 } else { 16 };
    let per_writer = if ctx.smoke { 25 } else { 200 };
    let (hub, _) = ctx.oracle.max_out_degree_vertex();
    let initial_degree = match ctx.backend.out_edges(&hub).await {
        Ok(edges) => edges.len(),
        Err(e) if crate::backends::Backend::is_unsupported(&e) => {
            r.unsupported(&format!("backend cannot read edges back: {e}"));
            return r;
        }
        Err(e) => {
            r.gates.oom_or_crash += 1;
            r.notes.push(format!("could not read hub degree: {e}"));
            return r;
        }
    };
    r.observe("hub", hub.as_str());
    r.observe("initial_out_degree", initial_degree);
    r.observe("writers", writers);
    r.observe("edges_per_writer", per_writer);

    let barrier = Arc::new(Barrier::new(writers));
    let mut handles = Vec::with_capacity(writers);
    for w in 0..writers {
        let store = match ctx.backend.extra_handle().await {
            Ok(s) => s,
            Err(e) => {
                r.gates.oom_or_crash += 1;
                r.notes.push(format!("could not open writer handle: {e}"));
                return r;
            }
        };
        let hub = hub.clone();
        let barrier = barrier.clone();
        handles.push(tokio::spawn(async move {
            let mut h = histogram();
            let mut accepted = 0usize;
            let mut conflicts = 0usize;
            let mut other = Vec::new();
            barrier.wait().await;
            for i in 0..per_writer {
                let target = format!("hot-{w}-{i}");
                let node = grust::Node::new(crate::dataset::NODE_LABEL, target.clone(), Props::new());
                let edge = Edge::new(EDGE_LABEL, hub.as_str(), target, Props::new());
                let t = Instant::now();
                let outcome = match store.put_node(&node).await {
                    Ok(_) => store.put_edge(&edge).await.map(|_| ()),
                    Err(e) => Err(e),
                };
                record(&mut h, t.elapsed());
                match outcome {
                    Ok(()) => accepted += 1,
                    Err(e) if is_conflict(&e) => conflicts += 1,
                    Err(e) => other.push(e.to_string()),
                }
            }
            (accepted, conflicts, other, h)
        }));
    }
    let mut accepted = 0usize;
    let mut conflicts = 0usize;
    let mut other: Vec<String> = Vec::new();
    let mut merged = histogram();
    for handle in handles {
        match handle.await {
            Ok((a, c, o, h)) => {
                accepted += a;
                conflicts += c;
                other.extend(o);
                let _ = merged.add(&h);
            }
            Err(e) => {
                r.gates.oom_or_crash += 1;
                r.notes.push(format!("writer task panicked: {e}"));
            }
        }
    }
    r.observe("accepted", accepted);
    r.observe("conflicts", conflicts);
    r.observe("non_conflict_errors", other.len());
    if let Some(sample) = other.first() {
        r.observe("non_conflict_error_sample", sample);
    }
    match ctx.backend.out_degree_after_reopen(&hub).await {
        Err(e) if crate::backends::Backend::is_unsupported(&e) => {
            r.unsupported(&format!("backend cannot read edges back: {e}"));
            return r;
        }
        Ok(final_degree) => {
            r.observe("final_out_degree", final_degree);
            let expected = initial_degree + accepted;
            if final_degree < expected {
                r.gates.lost_write += (expected - final_degree) as u64;
                r.notes.push(format!("{} accepted writes are missing after reopen", expected - final_degree));
            } else if final_degree > expected {
                r.gates.duplicate_durable_mutation += (final_degree - expected) as u64;
                r.notes.push(format!("{} more edges than accepted writes", final_degree - expected));
            }
        }
        Err(e) => {
            r.gates.oom_or_crash += 1;
            r.notes.push(format!("reopen failed: {e}"));
        }
    }
    r.latency = Some(Latency::from_histogram(&merged));
    r
}
