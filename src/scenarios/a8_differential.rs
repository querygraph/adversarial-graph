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

const QUERY_BUDGET: Duration = crate::differential::STORE_BUDGET;
/// The probe a store gets after the harness stopped waiting for a query:
/// a trivial read, waited for up to the reference budget (900 s by
/// default; the wait is not a measurement). A store that does not answer
/// it is still executing the query it was handed; issuing the next one
/// would queue it behind that work, and its time would be the hang's, not
/// its own -- so the remaining queries are recorded as not attempted
/// instead. A store that answers late has finished the hung query on its
/// own and the next one starts on a quiet store.
const QUIESCENCE_PROBE: &str = "RETURN 1 AS ok";
/// How long a timed-out reference task is given to return before the cell
/// ends as unable to prove quiescence.
const REAP_GRACE: Duration = Duration::from_secs(10);
// The in-process executor's own deadline sits below the store budget, so
// a shape it cannot finish is refused by its policy inside the budget and
// recorded as such; only a store that is still running when the budget
// ends is a hang.
const _: () = assert!(
    crate::differential::IN_PROCESS_BUDGET.as_secs() < QUERY_BUDGET.as_secs(),
    "the in-process executor must refuse before the store budget ends"
);

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
    /// How the answer key was produced: the in-process executor, the
    /// proven resident count, or the harness's native oracle.
    oracle_route: Option<crate::differential::Route>,
}

pub async fn run(ctx: &Ctx<'_>) -> ScenarioResult {
    let reference_budget = crate::differential::reference_budget();
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
    // outside every query's timing, over the loaded graph itself: the copy
    // A8 used to take here was a second graph's worth of memory per cell.
    let graph = Arc::clone(ctx.shared_typed_graph());
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
        mut not_attempted,
    ) = (0, 0, 0, 0, 0, 0, 0);
    // The query the harness last stopped waiting for, until the store has
    // answered a probe since.
    let mut hung: Option<String> = None;
    for (position, spec) in specs.iter().enumerate() {
        eprintln!(
            "   A8 {}/{} {} ({})",
            position + 1,
            specs.len(),
            spec.id,
            spec.kind
        );
        if let Some(hung_id) = hung.take() {
            let probe_started = Instant::now();
            let probe =
                tokio::time::timeout(reference_budget, ctx.backend.cypher(QUIESCENCE_PROBE)).await;
            let probe_ms = probe_started.elapsed().as_secs_f64() * 1e3;
            match probe {
                // An answer, or an error, is a store that is listening again.
                Ok(_) => eprintln!("      store quiescent again after {probe_ms:.0}ms"),
                Err(_) => {
                    let detail = format!(
                        "not attempted: the store was still executing {hung_id} {} s after the harness stopped waiting for it",
                        reference_budget.as_secs()
                    );
                    eprintln!("      {detail}");
                    for rest in &specs[position..] {
                        not_attempted += 1;
                        records.push(QueryRecord {
                            id: rest.id.clone(),
                            kind: rest.kind.clone(),
                            route: None,
                            outcome: "not-attempted",
                            ms: None,
                            rows: None,
                            detail: Some(detail.clone()),
                            oracle_ms: None,
                            oracle_route: None,
                        });
                    }
                    r.notes.push(format!(
                        "{not_attempted} queries not attempted after {hung_id}: the store did not answer a probe within the budget"
                    ));
                    break;
                }
            }
        }
        let oracle_started = Instant::now();
        // The oracle gets its own budget (`differential::reference_budget`,
        // larger than the store's): the reference executor is not a
        // measurement, but a shape it cannot answer in that budget is not
        // comparable either, and is recorded as such rather than waited on.
        // The reference executor runs under the same cooperative deadline
        // (`in_process_policy_with(reference_budget()).max_execution_time`),
        // so a timed-out task stops itself shortly after the outer timeout
        // fires. It is nevertheless reaped here before the next query: a
        // blocking task keeps its thread, the graph and the CPU until it
        // returns, and a cell may not start its next observation with the
        // last one still running. A task that will not return within the
        // grace ends the cell -- quiescence cannot be proven.
        let oracle_answer = {
            let graph = Arc::clone(&graph);
            let index = Arc::clone(&index);
            let cypher = spec.cypher.clone();
            let mut task = tokio::task::spawn_blocking(move || oracle(&graph, &index, &cypher));
            match tokio::time::timeout(reference_budget, &mut task).await {
                Ok(joined) => Ok(joined),
                Err(elapsed) => match tokio::time::timeout(REAP_GRACE, &mut task).await {
                    Ok(_) => Err(elapsed),
                    Err(_) => {
                        r.gates.hang_or_timeout_without_refusal += 1;
                        r.notes.push(format!(
                                "the reference executor did not stop within {}s of its {}s budget on {}; the cell ends because quiescence cannot be proven",
                                REAP_GRACE.as_secs(), reference_budget.as_secs(), spec.id
                            ));
                        account_query_outcomes(
                            &mut r,
                            errors,
                            timeouts + 1,
                            reference_unsupported,
                            not_attempted,
                        );
                        r.observe("queries", records.len());
                        r.finish();
                        return r;
                    }
                },
            }
        };
        let mut oracle_route = None;
        let expected = match oracle_answer {
            Ok(Ok(Ok((table, route)))) => {
                oracle_route = Some(route);
                table.normalized(spec.ordered)
            }
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
                    oracle_route: None,
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
                    oracle_route: None,
                });
                continue;
            }
            Err(_) => {
                reference_unsupported += 1;
                let detail = format!(
                    "the reference executor exceeded the {} s budget; the shape is not comparable on this slice",
                    reference_budget.as_secs()
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
                    oracle_ms: Some(reference_budget.as_secs_f64() * 1e3),
                    oracle_route: None,
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
            Ok(Err(e))
                if crate::backends::Backend::is_unsupported(&e)
                    || crate::differential::is_policy_refusal(&e) =>
            {
                refused += 1;
                ("refused", None, Some(e.to_string()))
            }
            // The store stopped the query at the deadline it was handed:
            // a timeout, and the store is already free for the next one.
            Ok(Err(e)) if crate::differential::is_store_deadline(&e) => {
                timeouts += 1;
                ("timeout", None, Some(e.to_string()))
            }
            Ok(Err(e)) => {
                errors += 1;
                ("error", None, Some(e.to_string()))
            }
            Err(_) => {
                timeouts += 1;
                hung = Some(spec.id.clone());
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
            oracle_route,
        });
    }
    let attempted = specs.len() - reference_unsupported - not_attempted;
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
    r.observe("reference_budget_s", reference_budget.as_secs());
    r.observe(
        "reference_intermediate_gb",
        crate::differential::reference_intermediate_bytes() >> 30,
    );
    r.observe("store_budget_s", QUERY_BUDGET.as_secs());
    r.observe("matched", matched);
    r.observe("mismatched", mismatched);
    r.observe("refused", refused);
    r.observe("errors", errors);
    r.observe("timeouts", timeouts);
    r.observe("reference_unsupported", reference_unsupported);
    r.observe("not_attempted", not_attempted);
    account_query_outcomes(
        &mut r,
        errors,
        timeouts,
        reference_unsupported,
        not_attempted,
    );
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
    not_attempted: usize,
) {
    r.gates.oom_or_crash += errors as u64;
    // Each hang is one gate; the queries never issued behind it are none.
    r.gates.hang_or_timeout_without_refusal += timeouts as u64;
    if reference_unsupported > 0 {
        r.unsupported(&format!(
            "the reference could not validate {reference_unsupported} required queries; this scenario's comparison coverage is incomplete"
        ));
    }
    if not_attempted > 0 {
        r.unsupported(&format!(
            "{not_attempted} queries were not attempted behind a query the store was still executing; this scenario's comparison coverage is incomplete"
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::Outcome;

    #[test]
    fn incomplete_or_failed_comparisons_never_pass() {
        for (errors, timeouts, missing_reference, not_attempted, expected) in [
            (3, 0, 0, 0, Outcome::Fail),
            (0, 3, 0, 0, Outcome::Fail),
            (0, 0, 3, 0, Outcome::Unsupported),
            (0, 0, 1, 0, Outcome::Unsupported),
            (1, 0, 1, 0, Outcome::Fail),
            // One hang, twenty-seven queries never issued behind it: one
            // gate, not twenty-eight, and the cell is not a pass.
            (0, 1, 0, 27, Outcome::Fail),
            (0, 0, 0, 2, Outcome::Unsupported),
            (0, 0, 0, 0, Outcome::Pass),
        ] {
            let mut r = ScenarioResult::new("A8", "fixture", "fixture");
            account_query_outcomes(&mut r, errors, timeouts, missing_reference, not_attempted);
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
        account_query_outcomes(&mut r, 0, 0, 0, 0);
        r.finish();
        assert_eq!(r.outcome, Outcome::Fail);
        assert_eq!(r.gates.wrong_answer, 1);
        assert!(r.notes.iter().any(|note| note == "one query refused"));
    }
}
