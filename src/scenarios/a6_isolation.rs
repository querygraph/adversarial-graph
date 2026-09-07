//! A6 — isolation under mixed load. N clients, each on its own handle,
//! hammer a small shared set of typed vertices with read-then-write
//! operations: a *register* workload (read a version counter, write it
//! back incremented and stamped with the client id) and a *list-append*
//! workload (read a list property, write it back with one element of the
//! client's own appended). The history of what every client observed and
//! wrote, plus the store's final state, goes to the checker in
//! `crate::isolation`, which derives the anomaly classes (lost update,
//! lost append, intermediate read, divergent order, non-monotonic read)
//! without knowing which store produced them. Every anomaly is an
//! `isolation_anomaly` gate.
//!
//! The write is conditional on the value read where the store offers a
//! compare-and-set: Turso's guarded commit with an `Exact` expectation on
//! the node as read. Everywhere else it is the portable whole-node upsert,
//! which is last-writer-wins, and the report says so in `write_mode`: on
//! that path a lost update is the expected outcome of two clients racing,
//! and it is still counted, because the family asks whether a store lets
//! concurrent read-modify-write lose data silently or refuses it with a
//! typed conflict. Layers: L3.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

use grust::{
    GraphCommitStore, GraphExpectation, GraphMutation, GraphStore, GrustError, GuardedGraphCommit,
    Node, NodeId, TursoGraphStore, Value,
};
use tokio::sync::Barrier;

use super::Ctx;
use crate::backends::{Backend, BackendKind};
use crate::differential::schema_of;
use crate::isolation::{AppendOp, RegisterOp, WriteOutcome, check_append, check_register};
use crate::report::{Latency, ScenarioResult, histogram, record};

const VERSION: &str = "a6_version";
const OWNER: &str = "a6_owner";
const LOG: &str = "a6_log";
const KEY_LABEL: &str = "Person";
const KEYS: usize = 4;

/// One client's connection and the write discipline it can express.
enum Client {
    /// Compare-and-set through a guarded commit (Turso).
    Cas(TursoGraphStore),
    /// Whole-node upsert, last-writer-wins.
    Plain(Arc<dyn GraphStore>),
}

impl Client {
    async fn open(backend: &Backend) -> grust::Result<Self> {
        match backend.kind {
            BackendKind::TursoWal | BackendKind::TursoMvcc => {
                let path = backend.turso_path.as_ref().expect("turso path");
                Ok(Self::Cas(Backend::connect_turso(backend.kind, path).await?))
            }
            _ => Ok(Self::Plain(backend.extra_handle().await?)),
        }
    }

    fn mode(&self) -> &'static str {
        match self {
            Self::Cas(_) => "guarded-commit-cas",
            Self::Plain(_) => "upsert-last-writer-wins",
        }
    }

    async fn read(&self, id: &NodeId) -> grust::Result<Option<Node>> {
        match self {
            Self::Cas(s) => s.get_node(id).await,
            Self::Plain(s) => s.get_node(id).await,
        }
    }

    /// Write `next` in place of `expected`; on the CAS path the store
    /// rejects the write with a typed error if `expected` is no longer what
    /// it holds.
    async fn write(&self, expected: &Node, next: Node, attempt: &str) -> grust::Result<()> {
        match self {
            Self::Cas(s) => {
                let commit = GuardedGraphCommit::new(
                    format!("ag-a6-{attempt}"),
                    "sha256:a6",
                    vec![GraphMutation::UpsertNode(next)],
                )
                .with_expectations(vec![GraphExpectation::Exact(expected.clone())]);
                s.commit_guarded(&commit).await.map(|_| ())
            }
            Self::Plain(s) => s.put_node(&next).await.map(|_| ()),
        }
    }
}

fn classify(err: &GrustError) -> WriteOutcome {
    if matches!(err, GrustError::GraphExpectationFailed(_))
        || super::a4_hot_node::is_conflict(err)
    {
        WriteOutcome::Conflict
    } else {
        WriteOutcome::Error
    }
}

fn version_of(node: &Node) -> i64 {
    match node.props.get(VERSION) {
        Some(Value::Int(v)) => *v,
        _ => 0,
    }
}

fn log_of(node: &Node) -> Vec<String> {
    match node.props.get(LOG) {
        Some(Value::StringArray(items)) => items.clone(),
        _ => Vec::new(),
    }
}

/// Deterministic key choice per client and operation, spread so every
/// key sees every client and neighbours collide often.
fn key_index(client: usize, seq: usize, keys: usize) -> usize {
    (client * 7 + seq * 3 + seq / keys) % keys
}

/// Per-client tallies of one workload phase.
#[derive(Default)]
struct Tally {
    accepted: usize,
    conflicts: usize,
    errors: usize,
    read_failures: usize,
    error_sample: Option<String>,
}

impl Tally {
    fn note(&mut self, outcome: WriteOutcome, err: Option<&GrustError>) {
        match outcome {
            WriteOutcome::Accepted => self.accepted += 1,
            WriteOutcome::Conflict => self.conflicts += 1,
            WriteOutcome::Error => {
                self.errors += 1;
                if self.error_sample.is_none() {
                    self.error_sample = err.map(ToString::to_string);
                }
            }
        }
    }

    fn merge(&mut self, other: Tally) {
        self.accepted += other.accepted;
        self.conflicts += other.conflicts;
        self.errors += other.errors;
        self.read_failures += other.read_failures;
        if self.error_sample.is_none() {
            self.error_sample = other.error_sample;
        }
    }

    fn observe(&self, r: &mut ScenarioResult, phase: &str) {
        r.observe(&format!("{phase}_accepted"), self.accepted);
        r.observe(&format!("{phase}_conflicts"), self.conflicts);
        r.observe(&format!("{phase}_errors"), self.errors);
        r.observe(&format!("{phase}_read_failures"), self.read_failures);
        if let Some(sample) = &self.error_sample {
            r.observe(&format!("{phase}_error_sample"), sample);
        }
    }
}

pub async fn run(ctx: &Ctx<'_>) -> ScenarioResult {
    let mut r = ScenarioResult::new("A6", ctx.backend.kind.name(), ctx.dataset);
    if schema_of(ctx.format) != Some("ldbc-snb") {
        r.unsupported("A6 runs over LDBC SNB (Person vertices carry the register and the list)");
        return r;
    }
    let keys: Vec<NodeId> = ctx
        .graph
        .nodes
        .iter()
        .filter(|n| n.label.as_str() == KEY_LABEL)
        .take(KEYS)
        .map(|n| n.id.clone())
        .collect();
    if keys.len() < KEYS {
        r.unsupported(&format!("fewer than {KEYS} {KEY_LABEL} vertices in the loaded slice"));
        return r;
    }
    let clients = if ctx.smoke { 4 } else { 8 };
    let ops_per_client = if ctx.smoke { 10 } else { 40 };

    // Self-test of the write discipline, serially, before any contention:
    // a store whose CAS rejects the node as it was just read cannot be
    // measured on that path, and must not be reported as a clean pass.
    let probe = match Client::open(ctx.backend).await {
        Ok(c) => c,
        Err(e) if Backend::is_unsupported(&e) => {
            r.unsupported(&format!("backend cannot open a client handle: {e}"));
            return r;
        }
        Err(e) => {
            r.gates.oom_or_crash += 1;
            r.notes.push(format!("could not open a client handle: {e}"));
            return r;
        }
    };
    r.observe("write_mode", probe.mode());
    r.observe("keys", keys.iter().map(NodeId::as_str).collect::<Vec<_>>());
    r.observe("clients", clients);
    r.observe("ops_per_client_per_phase", ops_per_client);
    let shared_handle = matches!(&probe, Client::Plain(s) if Arc::ptr_eq(s, &ctx.backend.store));
    r.observe("handles_shared", shared_handle);
    match probe.read(&keys[0]).await {
        Ok(Some(node)) => {
            let mut next = node.clone();
            next.props.insert(VERSION.into(), Value::Int(version_of(&node)));
            next.props.insert(OWNER.into(), Value::String("probe".into()));
            next.props.insert(LOG.into(), Value::StringArray(log_of(&node)));
            if let Err(e) = probe.write(&node, next, "probe").await {
                if Backend::is_unsupported(&e) {
                    r.unsupported(&format!("backend cannot write a vertex property: {e}"));
                } else {
                    r.unsupported(&format!(
                        "serial write of the node as read was rejected ({}); the {} path is not measurable here: {e}",
                        match classify(&e) {
                            WriteOutcome::Conflict => "typed conflict with no contention",
                            _ => "error",
                        },
                        probe.mode()
                    ));
                }
                return r;
            }
        }
        Ok(None) => {
            // The load reported the vertex and A8 reads it through Cypher, so
            // this is the adapter's `get_node` not addressing a typed label
            // (the Bolt, HTTP and AGE adapters look for `:V` and return no
            // properties), not a store that lost it.
            r.unsupported(&format!(
                "the adapter's get_node cannot address {} by id (typed label and properties); A6 needs a label-aware get/put",
                keys[0].as_str()
            ));
            return r;
        }
        Err(e) if Backend::is_unsupported(&e) => {
            r.unsupported(&format!("backend cannot read a vertex by id: {e}"));
            return r;
        }
        Err(e) => {
            r.gates.oom_or_crash += 1;
            r.notes.push(format!("probe read failed: {e}"));
            return r;
        }
    }
    drop(probe);

    // Register phase.
    let (register_ops, register_tally, register_h) =
        register_phase(ctx, &keys, clients, ops_per_client, &mut r).await;
    // List-append phase.
    let (append_ops, append_tally, append_h) =
        append_phase(ctx, &keys, clients, ops_per_client, &mut r).await;

    // Final state, read through the primary handle.
    let mut final_versions = BTreeMap::new();
    let mut final_lists = BTreeMap::new();
    for key in &keys {
        match ctx.backend.store.get_node(key).await {
            Ok(Some(node)) => {
                final_versions.insert(key.as_str().to_string(), version_of(&node));
                final_lists.insert(key.as_str().to_string(), log_of(&node));
            }
            Ok(None) => r.notes.push(format!("{}: missing at the final read", key.as_str())),
            Err(e) => r.notes.push(format!("{}: final read failed: {e}", key.as_str())),
        }
    }

    let register = check_register(&register_ops, &final_versions);
    let append = check_append(&append_ops, &final_lists);
    let anomalies = register.total() + append.total();
    r.gates.isolation_anomaly += anomalies;
    for example in register.examples.iter().chain(&append.examples) {
        r.notes.push(example.clone());
    }
    register_tally.observe(&mut r, "register");
    append_tally.observe(&mut r, "append");
    r.observe("register_anomalies", &register);
    r.observe("append_anomalies", &append);
    r.observe("final_versions", &final_versions);
    r.observe(
        "final_list_lengths",
        final_lists
            .iter()
            .map(|(k, v)| (k.clone(), v.len()))
            .collect::<BTreeMap<_, _>>(),
    );
    r.observe("register_history", &register_ops);
    r.observe("append_history", &append_ops);
    let mut merged = register_h;
    let _ = merged.add(&append_h);
    if register_tally.accepted + append_tally.accepted > 0 {
        r.latency = Some(Latency::from_histogram(&merged));
    }
    r
}

/// Run one phase: every client, on its own handle, released together.
async fn phase<Op, F>(
    ctx: &Ctx<'_>,
    clients: usize,
    r: &mut ScenarioResult,
    body: F,
) -> (Vec<Op>, Tally, hdrhistogram::Histogram<u64>)
where
    Op: Send + 'static,
    F: Fn(usize, Client, Arc<Barrier>) -> tokio::task::JoinHandle<(Vec<Op>, Tally, hdrhistogram::Histogram<u64>)>,
{
    let barrier = Arc::new(Barrier::new(clients));
    let mut tasks = Vec::with_capacity(clients);
    for c in 0..clients {
        match Client::open(ctx.backend).await {
            Ok(client) => tasks.push(body(c, client, barrier.clone())),
            Err(e) => {
                r.gates.oom_or_crash += 1;
                r.notes.push(format!("client {c}: could not open a handle: {e}"));
            }
        }
    }
    let mut ops = Vec::new();
    let mut tally = Tally::default();
    let mut h = histogram();
    for task in tasks {
        match task.await {
            Ok((o, t, th)) => {
                ops.extend(o);
                tally.merge(t);
                let _ = h.add(&th);
            }
            Err(e) => {
                r.gates.oom_or_crash += 1;
                r.notes.push(format!("client task panicked: {e}"));
            }
        }
    }
    (ops, tally, h)
}

async fn register_phase(
    ctx: &Ctx<'_>,
    keys: &[NodeId],
    clients: usize,
    ops_per_client: usize,
    r: &mut ScenarioResult,
) -> (Vec<RegisterOp>, Tally, hdrhistogram::Histogram<u64>) {
    let keys: Arc<[NodeId]> = keys.into();
    phase(ctx, clients, r, |c, client, barrier| {
        let keys = Arc::clone(&keys);
        tokio::spawn(async move {
            let mut ops = Vec::with_capacity(ops_per_client);
            let mut tally = Tally::default();
            let mut h = histogram();
            barrier.wait().await;
            for seq in 0..ops_per_client {
                let key = &keys[key_index(c, seq, keys.len())];
                let read = match client.read(key).await {
                    Ok(Some(node)) => node,
                    _ => {
                        tally.read_failures += 1;
                        ops.push(RegisterOp {
                            client: c,
                            key: key.as_str().into(),
                            read_version: None,
                            outcome: WriteOutcome::Error,
                        });
                        continue;
                    }
                };
                let version = version_of(&read);
                let mut next = read.clone();
                next.props.insert(VERSION.into(), Value::Int(version + 1));
                next.props.insert(OWNER.into(), Value::String(format!("c{c}")));
                let t = Instant::now();
                let outcome = client.write(&read, next, &format!("r-{c}-{seq}")).await;
                record(&mut h, t.elapsed());
                let (outcome, err) = match &outcome {
                    Ok(()) => (WriteOutcome::Accepted, None),
                    Err(e) => (classify(e), Some(e)),
                };
                tally.note(outcome, err);
                ops.push(RegisterOp {
                    client: c,
                    key: key.as_str().into(),
                    read_version: Some(version),
                    outcome,
                });
            }
            (ops, tally, h)
        })
    })
    .await
}

async fn append_phase(
    ctx: &Ctx<'_>,
    keys: &[NodeId],
    clients: usize,
    ops_per_client: usize,
    r: &mut ScenarioResult,
) -> (Vec<AppendOp>, Tally, hdrhistogram::Histogram<u64>) {
    let keys: Arc<[NodeId]> = keys.into();
    phase(ctx, clients, r, |c, client, barrier| {
        let keys = Arc::clone(&keys);
        tokio::spawn(async move {
            let mut ops = Vec::with_capacity(ops_per_client);
            let mut tally = Tally::default();
            let mut h = histogram();
            barrier.wait().await;
            for seq in 0..ops_per_client {
                let key = &keys[key_index(c, seq, keys.len())];
                let element = format!("c{c}-{seq}");
                let read = match client.read(key).await {
                    Ok(Some(node)) => node,
                    _ => {
                        tally.read_failures += 1;
                        ops.push(AppendOp {
                            client: c,
                            key: key.as_str().into(),
                            observed: None,
                            element,
                            outcome: WriteOutcome::Error,
                        });
                        continue;
                    }
                };
                let observed = log_of(&read);
                let mut list = observed.clone();
                list.push(element.clone());
                let mut next = read.clone();
                next.props.insert(LOG.into(), Value::StringArray(list));
                let t = Instant::now();
                let outcome = client.write(&read, next, &format!("a-{c}-{seq}")).await;
                record(&mut h, t.elapsed());
                let (outcome, err) = match &outcome {
                    Ok(()) => (WriteOutcome::Accepted, None),
                    Err(e) => (classify(e), Some(e)),
                };
                tally.note(outcome, err);
                ops.push(AppendOp {
                    client: c,
                    key: key.as_str().into(),
                    observed: Some(observed),
                    element,
                    outcome,
                });
            }
            (ops, tally, h)
        })
    })
    .await
}
