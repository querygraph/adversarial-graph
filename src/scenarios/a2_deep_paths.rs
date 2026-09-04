//! A2 — deep paths. Hop-by-hop BFS to a large depth on a high-diameter graph
//! (roads) through the traversal IR; the reached set per depth must match
//! the oracle. Bounded-policy Cypher must refuse a variable-length pattern
//! beyond its hop limit rather than hang. Layers: L1–L2.

use std::time::{Duration, Instant};

use grust::{CypherParameters, ReadQueryPolicy, run_bounded_read_query};

use crate::report::{ScenarioResult, histogram, record, Latency};
use super::Ctx;

pub async fn run(ctx: &Ctx<'_>) -> ScenarioResult {
    let mut r = ScenarioResult::new("A2", ctx.backend.kind.name(), ctx.dataset);
    let depth = if ctx.smoke { 8 } else { 50 };
    // Start from the lowest-id vertex: deterministic and, on road graphs,
    // usually far from a hub.
    let start = ctx.graph.nodes[0].id.clone();
    let (expected_reached, expected_deepest) = ctx.oracle.bfs_depth(&start, depth);
    r.observe("start", start.as_str());
    r.observe("depth", depth);
    r.observe("expected_reached", expected_reached);
    r.observe("expected_deepest", expected_deepest);

    let mut h = histogram();
    let t = Instant::now();
    let layers = match ctx.backend.khop(&start, depth).await {
        Ok(l) => l,
        Err(e) => {
            r.gates.oom_or_crash += 1;
            r.notes.push(format!("deep traversal failed: {e}"));
            return r;
        }
    };
    record(&mut h, t.elapsed());
    let reached: usize = layers.iter().sum();
    let deepest = layers.iter().rposition(|n| *n > 0).map(|i| i + 1).unwrap_or(0);
    r.observe("observed_reached", reached);
    r.observe("observed_deepest", deepest);
    if reached != expected_reached || deepest != expected_deepest {
        r.gates.wrong_answer += 1;
        r.notes.push(format!(
            "reached/deepest {reached}/{deepest} != oracle {expected_reached}/{expected_deepest}"
        ));
    }
    r.latency = Some(Latency::from_histogram(&h));

    // Policy half: a variable-length path far beyond the hop limit must be
    // refused quickly by the bounded reference executor. Only the memory
    // backend can hand the executor a materialized graph.
    if let Some(memory) = &ctx.backend.memory {
        let graph = memory.graph();
        let policy = ReadQueryPolicy::default();
        let query = format!("MATCH (a:V {{id: '{}'}})-[:E*1..200]->(b:V) RETURN count(b)", start.as_str());
        let t = Instant::now();
        let outcome = run_bounded_read_query(&graph, &query, &CypherParameters::new(), &policy);
        let elapsed = t.elapsed();
        r.observe("policy_refusal_ms", elapsed.as_millis() as u64);
        match outcome {
            Ok(_) => {
                r.gates.policy_bypass += 1;
                r.notes.push("200-hop pattern was executed despite max_path_length=4".into());
            }
            Err(e) => {
                r.observe("policy_refusal", e.to_string());
                if elapsed > Duration::from_secs(5) {
                    r.gates.hang_or_timeout_without_refusal += 1;
                }
            }
        }
    }
    r
}
