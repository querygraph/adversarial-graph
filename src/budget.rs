//! Load budgets and the load-rate prediction that spares a host a
//! foregone cap. A store that cannot load a tier inside the budget is a
//! finding, recorded in the LOAD row with the gate; a store whose own
//! measured rate on this host says the tier cannot load inside the budget
//! is not sent to spend it, and the row says what was projected from what.

use std::path::Path;
use std::time::Duration;

/// `AG_LOAD_BUDGET_S`: the load phase's own budget, separate from the
/// families' time. Unset means unbounded, as before the budget existed.
pub fn load_budget() -> Option<Duration> {
    std::env::var("AG_LOAD_BUDGET_S")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|s| *s > 0)
        .map(Duration::from_secs)
}

/// `AG_LOAD_BOX_S`: the standard load budget a longer `AG_LOAD_BUDGET_S`
/// is measured against. A load that finishes inside the box is a boxed run
/// whatever budget it was given; one that needed more, or ran out of the
/// longer budget, is keyed under a profile that names that budget.
pub fn load_box() -> Option<Duration> {
    std::env::var("AG_LOAD_BOX_S")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|s| *s > 0)
        .map(Duration::from_secs)
}

/// The profile component for a load under a budget longer than the box, or
/// none when the load fit the box (or no longer budget was given).
pub fn load_budget_profile(
    budget: Option<Duration>,
    boxed: Option<Duration>,
    took: Duration,
    ran_out: bool,
) -> Option<String> {
    let (budget, boxed) = (budget?, boxed?);
    (budget > boxed && (ran_out || took > boxed))
        .then(|| format!("load_budget_s={}", budget.as_secs()))
}

/// `AG_PREDICT_LOAD=0` sends a store to the tier regardless of the
/// projection: the deliberate measurement of a load past the budget.
pub fn predict_enabled() -> bool {
    std::env::var("AG_PREDICT_LOAD").ok().as_deref() != Some("0")
}

/// What this host has already measured for a store: its edges per second
/// on the largest tier it loaded here, and where that number came from.
#[derive(Debug, Clone, PartialEq)]
pub struct MeasuredRate {
    pub edges_per_s: f64,
    pub edges: u64,
    pub dataset: String,
    pub run: String,
}

impl MeasuredRate {
    /// The projected load time for `edges` at this rate.
    pub fn projected(&self, edges: u64) -> Duration {
        Duration::from_secs_f64(edges as f64 / self.edges_per_s.max(1e-9))
    }
}

/// The store's measured rate from the passing LOAD rows under `out_dir`
/// (this host's run directory): the row with the most edges, so a small
/// tier's warm-up rate never stands for a large one. `None` when the host
/// has no passing load for the store.
pub fn measured_rate(out_dir: &Path, backend: &str) -> Option<MeasuredRate> {
    let mut best: Option<MeasuredRate> = None;
    let Ok(entries) = std::fs::read_dir(out_dir) else {
        return None;
    };
    for entry in entries.flatten() {
        let run = entry.file_name().to_string_lossy().to_string();
        let Ok(text) = std::fs::read_to_string(entry.path().join("report.json")) else {
            continue;
        };
        let Ok(report) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };
        // A rate measured under another Grust source says nothing about this
        // build: an adapter change can make a store ten times faster or
        // slower, and a stale rate would refuse a load that now fits.
        if report["grust_source"].as_str() != Some(env!("AG_GRUST_CORE_SOURCE")) {
            continue;
        }
        let Some(results) = report["results"].as_array() else {
            continue;
        };
        for r in results {
            if r["backend"].as_str() != Some(backend)
                || r["scenario"].as_str() != Some("LOAD")
                || r["outcome"].as_str() != Some("pass")
            {
                continue;
            }
            let o = &r["observations"];
            let (Some(rate), Some(edges)) = (o["edges_per_s"].as_f64(), o["edges"].as_u64()) else {
                continue;
            };
            if rate <= 0.0 || edges == 0 {
                continue;
            }
            let candidate = MeasuredRate {
                edges_per_s: rate,
                edges,
                dataset: r["dataset"].as_str().unwrap_or("").to_string(),
                run: run.clone(),
            };
            let better = match &best {
                None => true,
                Some(b) => edges > b.edges || (edges == b.edges && run > b.run),
            };
            if better {
                best = Some(candidate);
            }
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_run(
        dir: &Path,
        run: &str,
        backend: &str,
        dataset: &str,
        edges: u64,
        rate: f64,
        outcome: &str,
    ) {
        let d = dir.join(run);
        std::fs::create_dir_all(&d).unwrap();
        let report = serde_json::json!({
            "grust_source": env!("AG_GRUST_CORE_SOURCE"),
            "results": [{
                "scenario": "LOAD", "backend": backend, "dataset": dataset, "outcome": outcome,
                "gates": {}, "observations": {"edges": edges, "edges_per_s": rate}
            }]
        });
        std::fs::write(d.join("report.json"), report.to_string()).unwrap();
    }

    #[test]
    fn the_rate_comes_from_the_largest_passing_load_of_that_store_on_this_host() {
        let dir = tempfile::tempdir().unwrap();
        write_run(
            dir.path(),
            "20260909T010000Z",
            "neo4j",
            "wiki-Talk",
            5_000_000,
            40_000.0,
            "pass",
        );
        write_run(
            dir.path(),
            "20260909T020000Z",
            "neo4j",
            "cit-Patents",
            16_500_000,
            8_000.0,
            "pass",
        );
        write_run(
            dir.path(),
            "20260909T030000Z",
            "neo4j",
            "com-Orkut",
            117_000_000,
            100_000.0,
            "fail",
        );
        write_run(
            dir.path(),
            "20260909T040000Z",
            "memgraph",
            "cit-Patents",
            16_500_000,
            9_000.0,
            "pass",
        );
        let rate = measured_rate(dir.path(), "neo4j").unwrap();
        assert_eq!(rate.dataset, "cit-Patents");
        assert_eq!(rate.edges_per_s, 8_000.0);
        // 117 M edges at 8,000/s is about 4.1 h: past a 2 h budget.
        let projected = rate.projected(117_185_083);
        assert!(projected > Duration::from_secs(7200), "{projected:?}");
        assert!(
            projected < Duration::from_secs(4 * 3600 + 600),
            "{projected:?}"
        );
        assert!(measured_rate(dir.path(), "falkor").is_none());
        assert!(measured_rate(Path::new("/nonexistent"), "neo4j").is_none());
    }

    #[test]
    fn a_rate_measured_under_another_grust_source_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path().join("20260910T000000Z");
        std::fs::create_dir_all(&d).unwrap();
        let report = serde_json::json!({
            "grust_source": "git+https://github.com/querygraph/grust?rev=an-older-revision",
            "results": [{
                "scenario": "LOAD", "backend": "turso-mvcc", "dataset": "cit-Patents", "outcome": "pass",
                "gates": {}, "observations": {"edges": 16_518_948u64, "edges_per_s": 1_505.0}
            }]
        });
        std::fs::write(d.join("report.json"), report.to_string()).unwrap();
        assert!(measured_rate(dir.path(), "turso-mvcc").is_none());
    }

    #[test]
    fn a_longer_budget_names_a_profile_only_when_the_load_needed_it() {
        let s = Duration::from_secs;
        let (longer, boxed) = (Some(s(14_400)), Some(s(7_200)));
        // Fit the box: a boxed run, whatever budget it was given.
        assert_eq!(load_budget_profile(longer, boxed, s(6_000), false), None);
        // Needed the extra time, or ran out of it: keyed under the budget.
        let named = Some("load_budget_s=14400".to_string());
        assert_eq!(load_budget_profile(longer, boxed, s(9_000), false), named);
        assert_eq!(load_budget_profile(longer, boxed, s(14_400), true), named);
        // No longer budget, or no box: nothing to name.
        assert_eq!(load_budget_profile(boxed, boxed, s(7_300), true), None);
        assert_eq!(load_budget_profile(longer, None, s(9_000), false), None);
        assert_eq!(load_budget_profile(None, boxed, s(9_000), false), None);
    }
}
