//! A8 — differential Cypher. A pinned set of read queries over a typed
//! graph (`scenarios/v1/a8/queries.json`) runs through grust-cypher's
//! reference executor on the in-process graph as the oracle and through
//! every store that accepts Cypher: Grust pushdown on Turso and PostgreSQL
//! (with the resident index where the plan is proven), the engine's own
//! openCypher on FalkorDB and Neo4j. Full result sets are compared, ordered
//! when the query orders them and as multisets otherwise; a disagreement
//! is a `wrong_answer` gate, a typed refusal is recorded as refused, never
//! as a pass, and the route each store took is recorded per query.

use std::time::{Duration, Instant};

use std::sync::Arc;

use super::Ctx;
use crate::differential::{oracle, queries_for, schema_of};
use crate::report::{Latency, ScenarioResult, histogram, record};

const QUERY_BUDGET: Duration = Duration::from_secs(120);

#[derive(serde::Serialize)]
struct QueryRecord {
    id: String,
    kind: String,
    route: Option<crate::differential::Route>,
    outcome: &'static str,
    ms: Option<f64>,
    rows: Option<usize>,
    detail: Option<String>,
    /// The oracle's own time for the query, for scale, never a measurement.
    oracle_ms: Option<f64>,
}

pub async fn run(ctx: &Ctx<'_>) -> ScenarioResult {
    let mut r = ScenarioResult::new("A8", ctx.backend.kind.name(), ctx.dataset);
    let Some(schema) = schema_of(ctx.format) else {
        r.unsupported("A8 needs a typed dataset (LDBC SNB or ICIJ Offshore Leaks)");
        return r;
    };
    let specs = queries_for(schema);
    if specs.is_empty() {
        r.unsupported("no reference queries are configured for this schema");
        return r;
    }
    // The oracle's typed index over the in-process graph, built once,
    // outside every query's timing.
    let graph = Arc::new(ctx.graph.clone());
    let index = match grust::TypedGraphIndex::new(Arc::clone(&graph)).map(Arc::new) {
        Ok(index) => index,
        Err(e) => {
            r.gates.oom_or_crash += 1;
            r.notes.push(format!("oracle index: {e}"));
            return r;
        }
    };
    let mut records = Vec::with_capacity(specs.len());
    let mut h = histogram();
    let (
        mut matched,
        mut mismatched,
        mut refused,
        mut errors,
        mut timeouts,
        mut reference_unsupported,
    ) = (0, 0, 0, 0, 0, 0);
    for (position, spec) in specs.iter().enumerate() {
        eprintln!(
            "   A8 {}/{} {} ({})",
            position + 1,
            specs.len(),
            spec.id,
            spec.kind
        );
        let oracle_started = Instant::now();
        // The oracle gets the same budget as the store: the reference executor
        // is not a measurement, but a shape it cannot answer in the budget is
        // not comparable either, and is recorded as such rather than waited on.
        let oracle_answer = {
            let graph = Arc::clone(&graph);
            let index = Arc::clone(&index);
            let cypher = spec.cypher.clone();
            tokio::time::timeout(
                QUERY_BUDGET,
                tokio::task::spawn_blocking(move || oracle(&graph, &index, &cypher)),
            )
            .await
        };
        let expected = match oracle_answer {
            Ok(Ok(Ok((table, _)))) => table.normalized(spec.ordered),
            Ok(Ok(Err(e))) => {
                reference_unsupported += 1;
                eprintln!(
                    "      reference-unsupported: {}",
                    e.to_string().chars().take(120).collect::<String>()
                );
                records.push(QueryRecord {
                    id: spec.id.clone(),
                    kind: spec.kind.clone(),
                    route: None,
                    outcome: "reference-unsupported",
                    ms: None,
                    rows: None,
                    detail: Some(e.to_string()),
                    oracle_ms: None,
                });
                continue;
            }
            Ok(Err(join)) => {
                reference_unsupported += 1;
                records.push(QueryRecord {
                    id: spec.id.clone(),
                    kind: spec.kind.clone(),
                    route: None,
                    outcome: "reference-unsupported",
                    ms: None,
                    rows: None,
                    detail: Some(format!("oracle task failed: {join}")),
                    oracle_ms: None,
                });
                continue;
            }
            Err(_) => {
                reference_unsupported += 1;
                let detail = format!(
                    "the reference executor exceeded the {} s budget; the shape is not comparable on this slice",
                    QUERY_BUDGET.as_secs()
                );
                eprintln!("      reference-unsupported: {detail}");
                records.push(QueryRecord {
                    id: spec.id.clone(),
                    kind: spec.kind.clone(),
                    route: None,
                    outcome: "reference-unsupported",
                    ms: None,
                    rows: None,
                    detail: Some(detail),
                    oracle_ms: Some(QUERY_BUDGET.as_secs_f64() * 1e3),
                });
                continue;
            }
        };
        let oracle_ms = oracle_started.elapsed().as_secs_f64() * 1e3;
        let route = ctx.backend.cypher_route(&spec.cypher);
        let started = Instant::now();
        let answer = tokio::time::timeout(QUERY_BUDGET, ctx.backend.cypher(&spec.cypher)).await;
        let elapsed = started.elapsed();
        let ms = Some(elapsed.as_secs_f64() * 1e3);
        let (outcome, rows, detail) = match answer {
            Ok(Ok(actual)) => {
                let actual = actual
                    .aligned_to(&expected.columns)
                    .normalized(spec.ordered);
                match expected.diff(&actual) {
                    None => {
                        matched += 1;
                        record(&mut h, elapsed);
                        ("match", Some(actual.rows.len()), None)
                    }
                    Some(difference) => {
                        mismatched += 1;
                        r.gates.wrong_answer += 1;
                        r.notes.push(format!("{}: {difference}", spec.id));
                        ("mismatch", Some(actual.rows.len()), Some(difference))
                    }
                }
            }
            Ok(Err(e)) if crate::backends::Backend::is_unsupported(&e) => {
                refused += 1;
                ("refused", None, Some(e.to_string()))
            }
            Ok(Err(e)) => {
                errors += 1;
                ("error", None, Some(e.to_string()))
            }
            Err(_) => {
                timeouts += 1;
                (
                    "timeout",
                    None,
                    Some(format!("exceeded {} s", QUERY_BUDGET.as_secs())),
                )
            }
        };
        eprintln!(
            "      {outcome} oracle={oracle_ms:.0}ms backend={:.0}ms route={route:?}{}",
            ms.unwrap_or(0.0),
            detail
                .as_deref()
                .map(|d| format!(" {}", d.chars().take(100).collect::<String>()))
                .unwrap_or_default()
        );
        records.push(QueryRecord {
            id: spec.id.clone(),
            kind: spec.kind.clone(),
            route: Some(route),
            outcome,
            ms,
            rows,
            detail,
            oracle_ms: Some(oracle_ms),
        });
    }
    let attempted = specs.len() - reference_unsupported;
    if attempted > 0 && refused == attempted {
        r.unsupported("backend does not accept Cypher");
    } else if refused > 0 {
        // A cell with refused queries is not comparable as a whole: the
        // matched ones stay in the records, the outcome says what was
        // missing rather than reading as a pass over the remainder.
        let sample = records
            .iter()
            .find(|q| q.outcome == "refused")
            .and_then(|q| q.detail.clone())
            .unwrap_or_default();
        r.unsupported(&format!(
            "{refused} of {attempted} queries refused by the store: {}",
            sample.chars().take(160).collect::<String>()
        ));
    }
    r.observe("queries", &records);
    r.observe("query_count", specs.len());
    r.observe("matched", matched);
    r.observe("mismatched", mismatched);
    r.observe("refused", refused);
    r.observe("errors", errors);
    r.observe("timeouts", timeouts);
    r.observe("reference_unsupported", reference_unsupported);
    account_query_outcomes(&mut r, errors, timeouts, reference_unsupported);
    if matched > 0 {
        r.latency = Some(Latency::from_histogram(&h));
    }
    r
}

fn account_query_outcomes(
    r: &mut ScenarioResult,
    errors: usize,
    timeouts: usize,
    reference_unsupported: usize,
) {
    r.gates.oom_or_crash += errors as u64;
    r.gates.hang_or_timeout_without_refusal += timeouts as u64;
    if reference_unsupported > 0 {
        r.unsupported(&format!(
            "the reference could not validate {reference_unsupported} required queries; this scenario's comparison coverage is incomplete"
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::Outcome;

    #[test]
    fn incomplete_or_failed_comparisons_never_pass() {
        for (errors, timeouts, missing_reference, expected) in [
            (3, 0, 0, Outcome::Fail),
            (0, 3, 0, Outcome::Fail),
            (0, 0, 3, Outcome::Unsupported),
            (0, 0, 1, Outcome::Unsupported),
            (1, 0, 1, Outcome::Fail),
            (0, 0, 0, Outcome::Pass),
        ] {
            let mut r = ScenarioResult::new("A8", "fixture", "fixture");
            account_query_outcomes(&mut r, errors, timeouts, missing_reference);
            r.finish();
            assert_eq!(r.outcome, expected);
            assert_eq!(r.gates.total(), (errors + timeouts) as u64);
        }
    }

    #[test]
    fn refusal_and_wrong_answer_keep_both_evidence_and_failure_headline() {
        let mut r = ScenarioResult::new("A8", "fixture", "fixture");
        r.gates.wrong_answer = 1;
        r.unsupported("one query refused");
        account_query_outcomes(&mut r, 0, 0, 0);
        r.finish();
        assert_eq!(r.outcome, Outcome::Fail);
        assert_eq!(r.gates.wrong_answer, 1);
        assert!(r.notes.iter().any(|note| note == "one query refused"));
    }
}
