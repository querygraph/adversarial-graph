//! Report contract: hard gates that must all be zero, plus quality and
//! performance sections that are never folded into a score.

use std::collections::BTreeMap;
use std::time::Duration;

use hdrhistogram::Histogram;
use serde::Serialize;

#[derive(Debug, Default, Clone, Serialize)]
pub struct HardGates {
    pub wrong_answer: u64,
    pub lost_write: u64,
    pub duplicate_durable_mutation: u64,
    pub isolation_anomaly: u64,
    pub policy_bypass: u64,
    pub hang_or_timeout_without_refusal: u64,
    pub oom_or_crash: u64,
    pub unauthorized_disclosure: u64,
    pub non_deterministic_receipt: u64,
}

impl HardGates {
    pub fn total(&self) -> u64 {
        self.wrong_answer
            + self.lost_write
            + self.duplicate_durable_mutation
            + self.isolation_anomaly
            + self.policy_bypass
            + self.hang_or_timeout_without_refusal
            + self.oom_or_crash
            + self.unauthorized_disclosure
            + self.non_deterministic_receipt
    }
    pub fn merge(&mut self, other: &HardGates) {
        self.wrong_answer += other.wrong_answer;
        self.lost_write += other.lost_write;
        self.duplicate_durable_mutation += other.duplicate_durable_mutation;
        self.isolation_anomaly += other.isolation_anomaly;
        self.policy_bypass += other.policy_bypass;
        self.hang_or_timeout_without_refusal += other.hang_or_timeout_without_refusal;
        self.oom_or_crash += other.oom_or_crash;
        self.unauthorized_disclosure += other.unauthorized_disclosure;
        self.non_deterministic_receipt += other.non_deterministic_receipt;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome {
    Pass,
    Fail,
    Unsupported,
    NotTested,
}

#[derive(Debug, Clone, Serialize)]
pub struct Latency {
    pub samples: u64,
    pub p50_us: u64,
    pub p90_us: u64,
    pub p99_us: u64,
    pub p999_us: u64,
    pub max_us: u64,
    pub mean_us: f64,
}

impl Latency {
    pub fn from_histogram(h: &Histogram<u64>) -> Self {
        Self {
            samples: h.len(),
            p50_us: h.value_at_quantile(0.5),
            p90_us: h.value_at_quantile(0.9),
            p99_us: h.value_at_quantile(0.99),
            p999_us: h.value_at_quantile(0.999),
            max_us: h.max(),
            mean_us: h.mean(),
        }
    }
}

pub fn histogram() -> Histogram<u64> {
    Histogram::<u64>::new_with_bounds(1, 3_600_000_000, 3).expect("histogram bounds")
}

pub fn record(h: &mut Histogram<u64>, d: Duration) {
    let us = d.as_micros().clamp(1, 3_600_000_000) as u64;
    let _ = h.record(us);
}

#[derive(Debug, Clone, Serialize)]
pub struct ScenarioResult {
    pub scenario: String,
    pub backend: String,
    pub dataset: String,
    pub outcome: Outcome,
    pub gates: HardGates,
    pub observations: BTreeMap<String, serde_json::Value>,
    pub latency: Option<Latency>,
    pub wall_ms: u128,
    pub notes: Vec<String>,
    #[serde(skip)]
    pub setup_failed: bool,
}

impl ScenarioResult {
    pub fn new(scenario: &str, backend: &str, dataset: &str) -> Self {
        Self {
            scenario: scenario.to_string(),
            backend: backend.to_string(),
            dataset: dataset.to_string(),
            outcome: Outcome::NotTested,
            gates: HardGates::default(),
            observations: BTreeMap::new(),
            latency: None,
            wall_ms: 0,
            notes: Vec::new(),
            setup_failed: false,
        }
    }
    pub fn observe(&mut self, key: &str, value: impl Serialize) {
        self.observations.insert(
            key.to_string(),
            serde_json::to_value(value).unwrap_or(serde_json::Value::Null),
        );
    }
    /// The cell could not be attempted: the dataset did not load or the
    /// oracle could not be built. That is a harness or host outcome, not a
    /// store finding, so it carries no gate -- but it must stay visible as
    /// `not-tested`, never become a pass, and count against completeness.
    pub fn setup_failed(&mut self, why: &str) {
        self.outcome = Outcome::NotTested;
        self.setup_failed = true;
        self.notes.push(why.to_string());
    }

    pub fn finish(&mut self) {
        if self.setup_failed {
            return;
        }
        // A refusal in one operation cannot hide a failure in another.
        // Keep the refusal's notes, but make the cell headline reflect gates.
        if self.gates.total() > 0 {
            self.outcome = Outcome::Fail;
        } else if self.outcome == Outcome::NotTested {
            self.outcome = Outcome::Pass;
        }
    }
    pub fn unsupported(&mut self, why: &str) {
        self.outcome = Outcome::Unsupported;
        self.notes.push(why.to_string());
    }

    /// A cell the harness declined to spend the budget on, from what this
    /// host had already measured: `not-tested`, the projection in the note,
    /// and `finish` leaves it so.
    pub fn not_attempted(&mut self, why: &str) {
        self.outcome = Outcome::NotTested;
        self.setup_failed = true;
        self.notes.push(why.to_string());
    }
}

/// `AG_HOST_PROFILE`: the host class a run was taken on, when it is not the
/// reference host. RESULTS.md keys a cell by dataset, backend, scenario,
/// slice and profile, not by host, so without it a row from a smaller box
/// would supersede the reference host's row for the same cell.
pub fn host_profile() -> Option<String> {
    std::env::var("AG_HOST_PROFILE")
        .ok()
        .filter(|h| !h.is_empty())
}

/// `AG_TURSO_SYNC`: the `PRAGMA synchronous` the Turso stores ran under, when
/// it is not Turso's default `full`. A `normal` or `off` row does not fsync
/// every commit, so it is tagged `turso_sync=<mode>` and forms its own cells
/// instead of superseding the durable rows.
pub fn turso_sync() -> Option<String> {
    std::env::var("AG_TURSO_SYNC")
        .ok()
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty() && s != "full")
}

/// Every profile tag a row takes from the run's environment: the host class
/// and, on Turso rows, a non-default synchronous mode.
pub fn tag_run(result: &mut ScenarioResult) {
    tag_host(result, host_profile().as_deref());
    if result.backend.starts_with("turso") {
        if let Some(mode) = turso_sync() {
            tag_profile(result, format!("turso_sync={mode}"));
        }
    }
}

/// Adds `host=<class>` to the row's profile (once), keeping what is there.
pub fn tag_host(result: &mut ScenarioResult, host: Option<&str>) {
    let Some(host) = host else { return };
    tag_profile(result, format!("host={host}"));
}

/// Adds `tag` to the row's comma-separated profile once, keeping what is there.
fn tag_profile(result: &mut ScenarioResult, tag: String) {
    let merged = match result.observations.get("profile").and_then(|v| v.as_str()) {
        Some(p) if p.split(',').any(|c| c == tag) => return,
        Some(p) => format!("{p},{tag}"),
        None => tag,
    };
    result.observe("profile", merged);
}

#[cfg(test)]
mod host_profile_tests {
    use super::*;

    #[test]
    fn a_host_class_joins_the_profile_once_and_keeps_what_was_there() {
        let mut plain = ScenarioResult::new("A1", "neo4j", "com-Orkut");
        tag_host(&mut plain, None);
        assert!(!plain.observations.contains_key("profile"));
        tag_host(&mut plain, Some("2xlarge-4c-32g"));
        assert_eq!(plain.observations["profile"], "host=2xlarge-4c-32g");
        tag_host(&mut plain, Some("2xlarge-4c-32g"));
        assert_eq!(plain.observations["profile"], "host=2xlarge-4c-32g");
        let mut profiled = ScenarioResult::new("LOAD", "falkor", "wiki-Talk");
        profiled.observe("profile", "resultset_size=10000");
        tag_host(&mut profiled, Some("2xlarge-4c-32g"));
        assert_eq!(
            profiled.observations["profile"],
            "resultset_size=10000,host=2xlarge-4c-32g"
        );
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub schema: &'static str,
    pub benchmark: &'static str,
    pub generated_at: String,
    pub harness_version: &'static str,
    /// Git revision of the harness that produced this report (`-dirty` when
    /// the tree had uncommitted changes), stamped at build time.
    pub harness_revision: &'static str,
    /// The paths that made the build tree differ from `harness_revision`
    /// (tracked changes, or untracked files under the build's inputs);
    /// empty for a clean build.
    pub harness_dirty_paths: Vec<String>,
    /// `grust-graph` facade version from Cargo.lock.
    pub grust_version: &'static str,
    /// Where `grust-core` (and with it every adapter sharing its `GraphStore`)
    /// resolved from: a registry version or a pinned git revision.
    pub grust_source: &'static str,
    pub host: BTreeMap<String, String>,
    pub datasets: Vec<serde_json::Value>,
    pub results: Vec<ScenarioResult>,
    pub gates: HardGates,
    pub summary: BTreeMap<String, serde_json::Value>,
}

impl Report {
    pub fn new() -> Self {
        let mut host = BTreeMap::new();
        host.insert("os".into(), std::env::consts::OS.into());
        host.insert("arch".into(), std::env::consts::ARCH.into());
        host.insert(
            "cpus".into(),
            std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(0)
                .to_string(),
        );
        // Memory and swap, so a row says whether the host could have paged
        // instead of failing: swap turns an over-RAM run into a silent thrash
        // that no resident-set guard sees.
        for (key, value) in host_memory() {
            host.insert(key.into(), value);
        }
        Self {
            schema: "adversarial-graph/report/v1",
            benchmark: "GRAPH-ADVERSARIAL-v1",
            generated_at: chrono::Utc::now().to_rfc3339(),
            harness_version: env!("CARGO_PKG_VERSION"),
            harness_revision: env!("AG_GIT_REV"),
            harness_dirty_paths: env!("AG_GIT_DIRTY_PATHS")
                .split(',')
                .filter(|p| !p.is_empty())
                .map(str::to_string)
                .collect(),
            grust_version: env!("AG_GRUST_GRAPH_VERSION"),
            grust_source: env!("AG_GRUST_CORE_SOURCE"),
            host,
            datasets: Vec::new(),
            results: Vec::new(),
            gates: HardGates::default(),
            summary: BTreeMap::new(),
        }
    }
    pub fn push(&mut self, mut result: ScenarioResult) {
        tag_run(&mut result);
        self.gates.merge(&result.gates);
        self.results.push(result);
    }
    pub fn finalize(&mut self) {
        let mut by_outcome: BTreeMap<String, u64> = BTreeMap::new();
        for r in &self.results {
            *by_outcome
                .entry(format!("{:?}", r.outcome).to_lowercase())
                .or_default() += 1;
        }
        self.summary
            .insert("outcomes".into(), serde_json::to_value(by_outcome).unwrap());
        self.summary
            .insert("hard_gate_total".into(), self.gates.total().into());
    }
}

/// `mem_total_bytes` and `swap_total_bytes` from `/proc/meminfo` on Linux
/// and from `sysctl` on macOS; absent where neither answers.
fn host_memory() -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    if let Ok(text) = std::fs::read_to_string("/proc/meminfo") {
        for line in text.lines() {
            let mut parts = line.split_whitespace();
            match (parts.next(), parts.next()) {
                (Some("MemTotal:"), Some(kb)) => {
                    if let Ok(kb) = kb.parse::<u64>() {
                        out.push(("mem_total_bytes", (kb * 1024).to_string()));
                    }
                }
                (Some("SwapTotal:"), Some(kb)) => {
                    if let Ok(kb) = kb.parse::<u64>() {
                        out.push(("swap_total_bytes", (kb * 1024).to_string()));
                    }
                }
                _ => {}
            }
        }
        return out;
    }
    if cfg!(target_os = "macos") {
        let sysctl = |key: &str| {
            std::process::Command::new("sysctl")
                .args(["-n", key])
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .map(|s| s.trim().to_string())
        };
        if let Some(bytes) = sysctl("hw.memsize") {
            out.push(("mem_total_bytes", bytes));
        }
        // `vm.swapusage` prints "total = 9216.00M  used = …"; macOS swap
        // grows on demand, so the total is the current file size.
        if let Some(line) = sysctl("vm.swapusage")
            && let Some(total) = line.split_whitespace().nth(2)
            && let Some(mb) = total.strip_suffix('M').and_then(|m| m.parse::<f64>().ok())
        {
            out.push(("swap_total_bytes", ((mb * 1048576.0) as u64).to_string()));
        }
    }
    out
}
