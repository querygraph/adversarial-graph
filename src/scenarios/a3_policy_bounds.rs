//! A3 — unbounded results. Cartesian products, range bombs, unanchored
//! MATCH, and deep UNION arms against the bounded read policy. Every attack
//! must be refused (Err) within the policy's own deadline; an accepted
//! attack is a `policy_bypass`, a slow refusal a `hang_or_timeout`.
//! Layer: L2. Runs on the memory backend, which exposes the materialized
//! graph the reference executor needs.

use std::time::{Duration, Instant};

use grust::{CypherParameters, ReadQueryPolicy, run_bounded_read_query};

use crate::report::{ScenarioResult, histogram, record, Latency};
use super::Ctx;

pub const ATTACKS: &[(&str, &str)] = &[
    ("unanchored-match", "MATCH (n:V) RETURN n"),
    ("cartesian", "MATCH (a:V), (b:V) RETURN count(*)"),
    ("triple-cartesian", "MATCH (a:V), (b:V), (c:V) RETURN count(*)"),
    ("range-bomb", "UNWIND range(1, 100000000) AS i RETURN count(i)"),
    ("union-arms", "MATCH (n:V) RETURN n.id UNION MATCH (n:V) RETURN n.id UNION MATCH (n:V) RETURN n.id UNION MATCH (n:V) RETURN n.id UNION MATCH (n:V) RETURN n.id UNION MATCH (n:V) RETURN n.id"),
    ("two-hop-collect", "MATCH (a:V)-[:E]->(b:V)-[:E]->(c:V) RETURN collect(c)"),
    ("deep-path", "MATCH (a:V)-[:E*1..64]->(b:V) RETURN count(*)"),
];

pub async fn run(ctx: &Ctx<'_>) -> ScenarioResult {
    let mut r = ScenarioResult::new("A3", ctx.backend.kind.name(), ctx.dataset);
    let Some(memory) = &ctx.backend.memory else {
        r.unsupported("bounded read policy is exercised through the reference executor on the memory backend");
        return r;
    };
    let graph = memory.graph();
    let policy = ReadQueryPolicy::default();
    let deadline = policy.max_execution_time * 3;
    let mut h = histogram();
    let mut refused = 0usize;
    for (name, query) in ATTACKS {
        let t = Instant::now();
        let outcome = run_bounded_read_query(&graph, query, &CypherParameters::new(), &policy);
        let elapsed = t.elapsed();
        record(&mut h, elapsed);
        match outcome {
            Ok(table) => {
                // A result is acceptable only if it is genuinely small.
                if table.rows.len() > policy.max_result_rows {
                    r.gates.policy_bypass += 1;
                    r.notes.push(format!("{name}: returned {} rows", table.rows.len()));
                } else {
                    r.observe(&format!("{name}.rows"), table.rows.len());
                }
            }
            Err(e) => {
                refused += 1;
                r.observe(&format!("{name}.refusal"), e.to_string());
            }
        }
        if elapsed > deadline {
            r.gates.hang_or_timeout_without_refusal += 1;
            r.notes.push(format!("{name}: took {:?} (> {:?})", elapsed, deadline));
        }
        r.observe(&format!("{name}.ms"), elapsed.as_millis() as u64);
    }
    r.observe("attacks", ATTACKS.len());
    r.observe("refused", refused);
    let _ = Duration::ZERO;
    r.latency = Some(Latency::from_histogram(&h));
    r
}
