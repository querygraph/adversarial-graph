//! A7 — guarded-commit replay. The same idempotency key replayed with the
//! same digest must return the original receipt marked `replayed` and leave
//! exactly one durable effect; the same key with a different digest must be
//! rejected; recovery must read the receipt without writing. Concurrent
//! replays of one key from many handles must still yield one effect.
//! Layers: L0, L3. Only stores implementing `GraphCommitStore` participate.

use std::sync::Arc;

use grust::{Edge, GraphCommitStore, GraphMutation, GuardedGraphCommit, Node, Props};
use tokio::sync::Barrier;

use super::Ctx;
use crate::backends::Backend;
use crate::dataset::{EDGE_LABEL, NODE_LABEL};
use crate::report::ScenarioResult;

pub async fn run(ctx: &Ctx<'_>) -> ScenarioResult {
    let mut r = ScenarioResult::new("A7", ctx.backend.kind.name(), ctx.dataset);
    let Some(store) = &ctx.backend.turso else {
        r.unsupported("backend does not implement GraphCommitStore");
        return r;
    };
    let (hub, _) = ctx.oracle.max_out_degree_vertex();
    // Earlier scenarios may have mutated the hub; measure the live degree.
    let initial_degree = match ctx.backend.out_degree(&hub).await {
        Ok(degree) => degree,
        Err(e) => {
            r.gates.oom_or_crash += 1;
            r.notes.push(format!("could not read hub degree: {e}"));
            return r;
        }
    };
    r.observe("initial_out_degree", initial_degree);
    let target = "replay-target".to_string();
    let mutations = vec![
        GraphMutation::UpsertNode(Node::new(NODE_LABEL, target.clone(), Props::new())),
        GraphMutation::UpsertEdge(Edge::new(
            EDGE_LABEL,
            hub.as_str(),
            target.clone(),
            Props::new(),
        )),
    ];
    let commit = GuardedGraphCommit::new("ag-a7-key", "sha256:digest-a", mutations.clone());

    let first = match store.commit_guarded(&commit).await {
        Ok(receipt) => receipt,
        Err(e) => {
            r.gates.oom_or_crash += 1;
            r.notes.push(format!("first guarded commit failed: {e}"));
            return r;
        }
    };
    r.observe("first_replayed", first.replayed);
    if first.replayed {
        r.gates.non_deterministic_receipt += 1;
    }

    // Exact replay.
    match store.commit_guarded(&commit).await {
        Ok(second) => {
            r.observe("second_replayed", second.replayed);
            if !second.replayed || second.commit_id != first.commit_id {
                r.gates.non_deterministic_receipt += 1;
                r.notes
                    .push("replay did not return the original receipt".into());
            }
        }
        Err(e) => {
            r.gates.non_deterministic_receipt += 1;
            r.notes.push(format!("exact replay rejected: {e}"));
        }
    }
    // Same key, different digest.
    let drifted = GuardedGraphCommit::new("ag-a7-key", "sha256:digest-b", mutations.clone());
    match store.commit_guarded(&drifted).await {
        Ok(_) => {
            r.gates.duplicate_durable_mutation += 1;
            r.notes
                .push("key reuse with a different digest was accepted".into());
        }
        Err(e) => r.observe("drift_rejection", e.to_string()),
    }
    // Recovery is read-only.
    match store
        .recover_guarded_commit("ag-a7-key", "sha256:digest-a")
        .await
    {
        Ok(Some(receipt)) if receipt.replayed && receipt.commit_id == first.commit_id => {}
        Ok(other) => {
            r.gates.non_deterministic_receipt += 1;
            r.notes.push(format!("recovery returned {other:?}"));
        }
        Err(e) => {
            r.gates.non_deterministic_receipt += 1;
            r.notes.push(format!("recovery failed: {e}"));
        }
    }
    match store
        .recover_guarded_commit("ag-a7-unknown", "sha256:none")
        .await
    {
        Ok(None) => {}
        other => {
            r.gates.non_deterministic_receipt += 1;
            r.notes
                .push(format!("unknown-key recovery returned {other:?}"));
        }
    }

    // Concurrent replay of one key from many handles.
    let handles_n = if ctx.smoke { 4 } else { 8 };
    let barrier = Arc::new(Barrier::new(handles_n));
    let path = ctx.backend.turso_path.clone().expect("turso path");
    let mut tasks = Vec::new();
    for _ in 0..handles_n {
        let barrier = barrier.clone();
        let path = path.clone();
        let kind = ctx.backend.kind;
        let commit = GuardedGraphCommit::new(
            "ag-a7-concurrent",
            "sha256:digest-c",
            vec![
                GraphMutation::UpsertNode(Node::new(NODE_LABEL, "replay-concurrent", Props::new())),
                GraphMutation::UpsertEdge(Edge::new(
                    EDGE_LABEL,
                    hub.as_str(),
                    "replay-concurrent",
                    Props::new(),
                )),
            ],
        );
        tasks.push(tokio::spawn(async move {
            let store = Backend::connect_turso(kind, &path).await?;
            barrier.wait().await;
            store.commit_guarded(&commit).await
        }));
    }
    let mut ids = std::collections::HashSet::new();
    let mut errors = 0usize;
    for t in tasks {
        match t.await {
            Ok(Ok(receipt)) => {
                ids.insert(receipt.commit_id);
            }
            Ok(Err(_)) | Err(_) => errors += 1,
        }
    }
    r.observe("concurrent_commit_ids", ids.len());
    r.observe("concurrent_errors", errors);
    if ids.len() > 1 {
        r.gates.duplicate_durable_mutation += (ids.len() - 1) as u64;
    }
    if ids.is_empty() {
        r.gates.lost_write += 1;
    }
    match ctx.backend.out_degree_after_reopen(&hub).await {
        Ok(final_degree) => {
            r.observe("final_out_degree", final_degree);
            let expected = initial_degree + 2;
            if final_degree != expected {
                if final_degree < expected {
                    r.gates.lost_write += (expected - final_degree) as u64;
                } else {
                    r.gates.duplicate_durable_mutation += (final_degree - expected) as u64;
                }
                r.notes.push(format!(
                    "hub out-degree {final_degree} != expected {expected}"
                ));
            }
        }
        Err(e) => {
            r.gates.oom_or_crash += 1;
            r.notes.push(format!("reopen failed: {e}"));
        }
    }
    r
}
