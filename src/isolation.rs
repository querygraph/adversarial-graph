//! Isolation checking for A6: an Elle-style reading of a recorded history.
//!
//! Two workloads run against a shared set of keys. In the *register*
//! workload every client reads a version counter and writes it back
//! incremented, stamped with its own id; in the *list-append* workload
//! every client reads a list and writes it back with one element of its
//! own appended. Each write is either conditional on the value read (a
//! compare-and-set, where the store offers one) or a plain overwrite. The
//! checker sees only the history: what each client observed, what it
//! wrote, and what the store held at the end. It derives the anomaly
//! classes below without knowing which store produced them.
//!
//! - `lost_update`: two accepted writes derived from the same version, so
//!   one overwrote the other and its increment is gone.
//! - `lost_append`: an accepted append whose element is absent from the
//!   final list.
//! - `intermediate_read`: a read that observed an element no accepted
//!   write kept, i.e. a value that was never durable in the final order.
//! - `divergent_order`: a read whose observed list is not a prefix of the
//!   final list although every element survived: the store exposed two
//!   incompatible orders of the same appends (a write cycle in the
//!   dependency graph).
//! - `non_monotonic_read`: one client observed a version, then a smaller
//!   one on the same key.
//!
//! A conditional write rejected with a typed conflict is not an anomaly:
//! that is the store doing its job.

use std::collections::{BTreeMap, HashSet};

/// Outcome of one write as the client saw it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum WriteOutcome {
    Accepted,
    Conflict,
    Error,
}

/// One register read-then-write.
#[derive(Clone, Debug, serde::Serialize)]
pub struct RegisterOp {
    pub client: usize,
    pub key: String,
    /// The version observed by the read half; `None` when the read failed.
    pub read_version: Option<i64>,
    pub outcome: WriteOutcome,
}

/// One list-append read-then-write.
#[derive(Clone, Debug, serde::Serialize)]
pub struct AppendOp {
    pub client: usize,
    pub key: String,
    /// The list observed by the read half; `None` when the read failed.
    pub observed: Option<Vec<String>>,
    /// The element this client appended.
    pub element: String,
    pub outcome: WriteOutcome,
}

/// Anomalies found in one history, by class.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct Anomalies {
    pub lost_update: u64,
    pub lost_append: u64,
    pub intermediate_read: u64,
    pub divergent_order: u64,
    pub non_monotonic_read: u64,
    /// Human-readable examples, a few per class, for the report notes.
    pub examples: Vec<String>,
}

impl Anomalies {
    pub fn total(&self) -> u64 {
        self.lost_update
            + self.lost_append
            + self.intermediate_read
            + self.divergent_order
            + self.non_monotonic_read
    }

    fn example(&mut self, text: String) {
        const KEEP: usize = 8;
        if self.examples.len() < KEEP {
            self.examples.push(text);
        }
    }
}

/// Check the register workload against the final version of every key.
///
/// `final_versions` maps key to the version the store held after every
/// client finished; a key missing from it was unreadable at the end.
pub fn check_register(ops: &[RegisterOp], final_versions: &BTreeMap<String, i64>) -> Anomalies {
    let mut out = Anomalies::default();

    // Lost updates: accepted writes that started from the same version.
    let mut by_key_version: BTreeMap<(&str, i64), Vec<usize>> = BTreeMap::new();
    for op in ops.iter().filter(|op| op.outcome == WriteOutcome::Accepted) {
        if let Some(v) = op.read_version {
            by_key_version
                .entry((op.key.as_str(), v))
                .or_default()
                .push(op.client);
        }
    }
    for ((key, version), clients) in &by_key_version {
        if clients.len() > 1 {
            let lost = (clients.len() - 1) as u64;
            out.lost_update += lost;
            out.example(format!(
                "{key}: {} accepted writes from version {version} (clients {clients:?}); {lost} lost",
                clients.len()
            ));
        }
    }

    // The final version must account for every accepted write; a shortfall
    // that the pairing above did not explain is still a lost update.
    let mut accepted_per_key: BTreeMap<&str, i64> = BTreeMap::new();
    for op in ops.iter().filter(|op| op.outcome == WriteOutcome::Accepted) {
        *accepted_per_key.entry(op.key.as_str()).or_default() += 1;
    }
    for (key, accepted) in accepted_per_key {
        if let Some(&final_version) = final_versions.get(key) {
            let explained: i64 = by_key_version
                .iter()
                .filter(|((k, _), clients)| *k == key && clients.len() > 1)
                .map(|(_, clients)| clients.len() as i64 - 1)
                .sum();
            let shortfall = accepted - explained - final_version;
            if shortfall > 0 {
                out.lost_update += shortfall as u64;
                out.example(format!(
                    "{key}: final version {final_version} after {accepted} accepted writes; {shortfall} unexplained"
                ));
            }
        }
    }

    // Monotonic reads per client and key.
    let mut last_seen: BTreeMap<(usize, &str), i64> = BTreeMap::new();
    for op in ops {
        let Some(v) = op.read_version else { continue };
        match last_seen.get(&(op.client, op.key.as_str())) {
            Some(&prev) if v < prev => {
                out.non_monotonic_read += 1;
                out.example(format!(
                    "{}: client {} read version {v} after {prev}",
                    op.key, op.client
                ));
            }
            _ => {
                last_seen.insert((op.client, op.key.as_str()), v);
            }
        }
    }
    out
}

/// Check the list-append workload against the final list of every key.
pub fn check_append(ops: &[AppendOp], final_lists: &BTreeMap<String, Vec<String>>) -> Anomalies {
    let mut out = Anomalies::default();
    let final_sets: BTreeMap<&str, HashSet<&str>> = final_lists
        .iter()
        .map(|(k, list)| (k.as_str(), list.iter().map(String::as_str).collect()))
        .collect();

    // Every accepted append must survive.
    for op in ops.iter().filter(|op| op.outcome == WriteOutcome::Accepted) {
        let Some(set) = final_sets.get(op.key.as_str()) else { continue };
        if !set.contains(op.element.as_str()) {
            out.lost_append += 1;
            out.example(format!(
                "{}: client {} appended {} and it is not in the final list",
                op.key, op.client, op.element
            ));
        }
    }

    // Every observed list must be a prefix of the final order.
    for op in ops {
        let Some(observed) = &op.observed else { continue };
        let Some(final_list) = final_lists.get(&op.key) else { continue };
        let set = &final_sets[op.key.as_str()];
        if let Some(missing) = observed.iter().find(|e| !set.contains(e.as_str())) {
            out.intermediate_read += 1;
            out.example(format!(
                "{}: client {} observed {} which never became durable",
                op.key, op.client, missing
            ));
        } else if !final_list.starts_with(observed) {
            out.divergent_order += 1;
            out.example(format!(
                "{}: client {} observed {:?}, not a prefix of the final {:?}",
                op.key,
                op.client,
                truncate(observed),
                truncate(final_list)
            ));
        }
    }
    out
}

fn truncate(list: &[String]) -> Vec<&str> {
    list.iter().take(6).map(String::as_str).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reg(client: usize, key: &str, read: i64, outcome: WriteOutcome) -> RegisterOp {
        RegisterOp {
            client,
            key: key.into(),
            read_version: Some(read),
            outcome,
        }
    }

    fn app(client: usize, key: &str, observed: &[&str], element: &str, outcome: WriteOutcome) -> AppendOp {
        AppendOp {
            client,
            key: key.into(),
            observed: Some(observed.iter().map(|s| s.to_string()).collect()),
            element: element.into(),
            outcome,
        }
    }

    #[test]
    fn serial_register_history_is_clean() {
        let ops = vec![
            reg(0, "k", 0, WriteOutcome::Accepted),
            reg(1, "k", 1, WriteOutcome::Accepted),
            reg(0, "k", 2, WriteOutcome::Accepted),
        ];
        let finals = BTreeMap::from([("k".to_string(), 3)]);
        assert_eq!(check_register(&ops, &finals).total(), 0);
    }

    #[test]
    fn two_writes_from_one_version_lose_one() {
        let ops = vec![
            reg(0, "k", 0, WriteOutcome::Accepted),
            reg(1, "k", 0, WriteOutcome::Accepted),
        ];
        let finals = BTreeMap::from([("k".to_string(), 1)]);
        let a = check_register(&ops, &finals);
        assert_eq!(a.lost_update, 1);
        assert_eq!(a.total(), 1);
    }

    #[test]
    fn a_typed_conflict_is_not_an_anomaly() {
        let ops = vec![
            reg(0, "k", 0, WriteOutcome::Accepted),
            reg(1, "k", 0, WriteOutcome::Conflict),
        ];
        let finals = BTreeMap::from([("k".to_string(), 1)]);
        assert_eq!(check_register(&ops, &finals).total(), 0);
    }

    #[test]
    fn final_shortfall_without_a_pair_is_still_lost() {
        // Two accepted writes from different versions, yet the store ends at 1.
        let ops = vec![
            reg(0, "k", 0, WriteOutcome::Accepted),
            reg(1, "k", 1, WriteOutcome::Accepted),
        ];
        let finals = BTreeMap::from([("k".to_string(), 1)]);
        assert_eq!(check_register(&ops, &finals).lost_update, 1);
    }

    #[test]
    fn version_going_backwards_is_non_monotonic() {
        let ops = vec![
            reg(0, "k", 2, WriteOutcome::Accepted),
            reg(0, "k", 1, WriteOutcome::Conflict),
        ];
        let finals = BTreeMap::from([("k".to_string(), 3)]);
        assert_eq!(check_register(&ops, &finals).non_monotonic_read, 1);
    }

    #[test]
    fn append_history_prefixes_are_clean() {
        let ops = vec![
            app(0, "k", &[], "a", WriteOutcome::Accepted),
            app(1, "k", &["a"], "b", WriteOutcome::Accepted),
            app(0, "k", &["a", "b"], "c", WriteOutcome::Accepted),
        ];
        let finals = BTreeMap::from([("k".to_string(), vec!["a".into(), "b".into(), "c".into()])]);
        assert_eq!(check_append(&ops, &finals).total(), 0);
    }

    #[test]
    fn overwritten_append_is_lost() {
        let ops = vec![
            app(0, "k", &[], "a", WriteOutcome::Accepted),
            app(1, "k", &[], "b", WriteOutcome::Accepted),
        ];
        let finals = BTreeMap::from([("k".to_string(), vec!["b".into()])]);
        let a = check_append(&ops, &finals);
        assert_eq!(a.lost_append, 1);
        assert_eq!(a.total(), 1);
    }

    #[test]
    fn reading_a_lost_element_is_an_intermediate_read() {
        let ops = vec![
            app(0, "k", &[], "a", WriteOutcome::Accepted),
            app(1, "k", &["a"], "b", WriteOutcome::Accepted),
            app(2, "k", &[], "c", WriteOutcome::Accepted),
        ];
        // "c" overwrote everything; client 1 saw "a", which did not survive.
        let finals = BTreeMap::from([("k".to_string(), vec!["c".into()])]);
        let a = check_append(&ops, &finals);
        assert_eq!(a.lost_append, 2);
        assert_eq!(a.intermediate_read, 1);
    }

    #[test]
    fn reordered_elements_are_a_divergent_order() {
        let ops = vec![app(0, "k", &["b", "a"], "c", WriteOutcome::Accepted)];
        let finals = BTreeMap::from([("k".to_string(), vec!["a".into(), "b".into(), "c".into()])]);
        assert_eq!(check_append(&ops, &finals).divergent_order, 1);
    }
}
