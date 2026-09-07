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
        }
    }
    pub fn observe(&mut self, key: &str, value: impl Serialize) {
        self.observations.insert(
            key.to_string(),
            serde_json::to_value(value).unwrap_or(serde_json::Value::Null),
        );
    }
    pub fn finish(&mut self) {
        if self.outcome == Outcome::NotTested {
            self.outcome = if self.gates.total() == 0 {
                Outcome::Pass
            } else {
                Outcome::Fail
            };
        }
    }
    pub fn unsupported(&mut self, why: &str) {
        self.outcome = Outcome::Unsupported;
        self.notes.push(why.to_string());
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
    pub fn push(&mut self, result: ScenarioResult) {
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
