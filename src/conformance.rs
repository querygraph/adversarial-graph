//! Adapter contract conformance: a deterministic typed multigraph that every
//! enabled backend must load and read back exactly, before any large dataset
//! is spent on it (Astra's review, gate 2). A successful load response is not
//! evidence; stored cardinalities, labels, edge identities and property values
//! are checked independently through the same adapter the benchmarks use.
//!
//!   ag conformance --backends memory,turso-wal,postgres,...
//!
//! Every check prints PASS, FAIL or UNSUPPORTED. An operation the adapter
//! declares unsupported is recorded as such, never as a failure; anything
//! else that disagrees with the fixture fails, and the process exits non-zero.

use std::collections::BTreeMap;
use std::path::Path;

use grust::{EdgePolicy, EdgeQuery, Graph, Label, NodeId, Value};

use crate::backends::{Backend, BackendKind};

const BATCH_BOUNDARY: usize = 500;

/// The fixture: small enough to read by hand, shaped to catch what large
/// graphs hide.
pub fn fixture() -> Graph {
    // The builder's default policy dedupes on (from, label, to), which
    // silently dropped k12b before any adapter saw it (every adapter then
    // "failed" the parallel-edge checks identically). The fixture wants
    // both.
    let mut b = Graph::builder().edge_policy(EdgePolicy::AllowDuplicates);
    // Two labels, a mix of value types, a null, a missing property, Unicode,
    // and an id that needs escaping in most query languages.
    let _ = b
        .node("Person", "p1")
        .prop("name", "Ada")
        .prop("age", 36i64)
        .prop("active", true)
        .finish();
    let _ = b
        .node("Person", "p2")
        .prop("name", "Bjørn Ünïcode ✓")
        .prop("score", 2.5f64)
        .finish();
    let _ = b
        .node("Person", "p3")
        .prop("name", "Cy")
        .prop("note", Value::Null)
        .finish();
    let _ = b
        .node("Company", "c1")
        .prop("name", "O'Reilly \"Quotes\" & Co")
        .finish();
    let _ = b
        .node("Company", "it's/odd id")
        .prop("name", "escaped")
        .finish();
    // An isolated vertex: no edges at all.
    let _ = b.node("Person", "isolated").finish();
    // A self-loop, parallel edges with distinct ids, and cross-label edges.
    let _ = b.edge("KNOWS", "p1", "p1").id("loop").finish();
    let _ = b
        .edge("KNOWS", "p1", "p2")
        .id("k12a")
        .prop("since", 2019i64)
        .finish();
    let _ = b
        .edge("KNOWS", "p1", "p2")
        .id("k12b")
        .prop("since", 2021i64)
        .finish();
    let _ = b.edge("WORKS_AT", "p1", "c1").id("w1").finish();
    let _ = b.edge("WORKS_AT", "p2", "it's/odd id").id("w2").finish();
    let _ = b.edge("KNOWS", "p3", "p1").id("k31").finish();
    b.build()
}

/// A graph exactly around a bulk batch boundary: sizes N-1, N, N+1 of nodes
/// and edges, so an off-by-one in chunking loses or duplicates a row.
pub fn boundary_fixture(n: usize) -> Graph {
    let mut b = Graph::builder();
    for i in 0..n {
        let _ = b.node("V", format!("v{i}")).prop("i", i as i64).finish();
    }
    for i in 0..n {
        let _ = b
            .edge("E", format!("v{i}"), format!("v{}", (i + 1) % n))
            .id(format!("e{i}"))
            .finish();
    }
    b.build()
}

struct Tally {
    pass: usize,
    fail: usize,
    unsupported: usize,
}

impl Tally {
    fn check(&mut self, name: &str, outcome: Result<bool, grust::GrustError>, detail: &str) {
        match outcome {
            Ok(true) => {
                self.pass += 1;
                println!("  PASS        {name}");
            }
            Ok(false) => {
                self.fail += 1;
                println!("  FAIL        {name}: {detail}");
            }
            Err(e) if Backend::is_unsupported(&e) => {
                self.unsupported += 1;
                println!("  UNSUPPORTED {name}: {e}");
            }
            Err(e) => {
                self.fail += 1;
                println!("  FAIL        {name}: {e}");
            }
        }
    }
}

async fn edges(
    store: &dyn grust::GraphStore,
    from: Option<&str>,
    to: Option<&str>,
    label: Option<&str>,
) -> grust::Result<Vec<grust::Edge>> {
    store
        .get_edges(EdgeQuery {
            from: from.map(NodeId::from),
            to: to.map(NodeId::from),
            label: label.map(Label::from),
        })
        .await
}

async fn conform(kind: BackendKind, work_dir: &Path, tally: &mut Tally) -> grust::Result<()> {
    let backend = Backend::open(kind, work_dir, "conformance").await?;
    let store = backend.store.as_ref();
    let graph = fixture();
    let report = backend.load(&graph).await?;
    tally.check(
        "load report counts the fixture",
        Ok(report.nodes == graph.nodes.len() && report.edges == graph.edges.len()),
        &format!(
            "reported {}/{} of {}/{}",
            report.nodes,
            report.edges,
            graph.nodes.len(),
            graph.edges.len()
        ),
    );

    // Nodes: every id, its label, and every property value, including the
    // null, the Unicode string and the id that needs escaping.
    for node in &graph.nodes {
        let got = store.get_node(&node.id).await;
        let name = format!("node {} reads back with label and props", node.id.as_str());
        match got {
            Ok(Some(n)) => {
                let same_label = n.label == node.label;
                let same_props = node.props.iter().all(|(k, v)| n.props.get(k) == Some(v));
                tally.check(
                    &name,
                    Ok(same_label && same_props),
                    &format!("got label {:?} props {:?}", n.label, n.props),
                );
            }
            Ok(None) => tally.check(&name, Ok(false), "missing"),
            Err(e) => tally.check(&name, Err(e), ""),
        }
    }

    // Edges: cardinalities by endpoint and label, including the loop and
    // the parallel pair, whose ids must both survive.
    let expect: Vec<(&str, Option<&str>, Option<&str>, Option<&str>, usize)> = vec![
        ("all edges out of p1", Some("p1"), None, None, 4),
        (
            "parallel KNOWS p1->p2 both kept",
            Some("p1"),
            Some("p2"),
            Some("KNOWS"),
            2,
        ),
        ("self-loop p1->p1", Some("p1"), Some("p1"), Some("KNOWS"), 1),
        (
            "edge into the escaped id",
            None,
            Some("it's/odd id"),
            Some("WORKS_AT"),
            1,
        ),
        (
            "isolated vertex has no edges",
            Some("isolated"),
            None,
            None,
            0,
        ),
        (
            "label filter WORKS_AT out of p1",
            Some("p1"),
            None,
            Some("WORKS_AT"),
            1,
        ),
    ];
    for (name, from, to, label, want) in expect {
        let got = edges(store, from, to, label).await;
        match got {
            Ok(v) => tally.check(
                name,
                Ok(v.len() == want),
                &format!("got {} want {want}", v.len()),
            ),
            Err(e) => tally.check(name, Err(e), ""),
        }
    }
    match edges(store, Some("p1"), Some("p2"), Some("KNOWS")).await {
        Ok(v) => {
            let mut ids: Vec<String> = v
                .iter()
                .filter_map(|e| e.id.as_ref().map(|i| i.as_str().to_string()))
                .collect();
            ids.sort();
            tally.check(
                "parallel edges keep distinct ids",
                Ok(ids == ["k12a", "k12b"]),
                &format!("ids {ids:?}"),
            );
        }
        Err(e) => tally.check("parallel edges keep distinct ids", Err(e), ""),
    }

    // Update: a second put of p1 with a changed value must be read back.
    let mut updated = graph
        .nodes
        .iter()
        .find(|n| n.id.as_str() == "p1")
        .cloned()
        .expect("p1");
    updated.props.insert("age".into(), Value::Int(37));
    match store.put_node(&updated).await {
        Ok(_) => match store.get_node(&updated.id).await {
            Ok(Some(n)) => tally.check(
                "update is visible on the next read",
                Ok(n.props.get("age") == Some(&Value::Int(37))),
                &format!("age {:?}", n.props.get("age")),
            ),
            Ok(None) => tally.check(
                "update is visible on the next read",
                Ok(false),
                "missing after update",
            ),
            Err(e) => tally.check("update is visible on the next read", Err(e), ""),
        },
        Err(e) => tally.check("update is visible on the next read", Err(e), ""),
    }

    // Delete: p3 goes, and so must its edge to p1. Deletion goes through the
    // backend's own delete path (GraphMutationStore on the Grust stores, native
    // DETACH DELETE on the Cypher ones), as A5 does, not through GraphStore.
    match backend.delete_node(&NodeId::from("p3")).await {
        Ok(()) => {
            let gone = store.get_node(&NodeId::from("p3")).await;
            match gone {
                Ok(v) => tally.check("deleted node is absent", Ok(v.is_none()), "still present"),
                Err(e) => tally.check("deleted node is absent", Err(e), ""),
            }
            match edges(store, Some("p3"), None, None).await {
                Ok(v) => tally.check(
                    "deleted node's edges are gone",
                    Ok(v.is_empty()),
                    &format!("{} edges remain", v.len()),
                ),
                Err(e) => tally.check("deleted node's edges are gone", Err(e), ""),
            }
        }
        Err(e) => tally.check("delete node", Err(e), ""),
    }

    // Batch boundary: N-1, N, N+1 rows around the incremental batch size.
    for n in [BATCH_BOUNDARY - 1, BATCH_BOUNDARY, BATCH_BOUNDARY + 1] {
        let b = Backend::open(kind, work_dir, &format!("conformance-{n}")).await?;
        let g = boundary_fixture(n);
        let r = b.load(&g).await;
        let name = format!("batch boundary {n} nodes/{n} edges load and count");
        match r {
            Ok(_) => {
                let mut ok = true;
                let mut detail = String::new();
                for i in [0, n / 2, n - 1] {
                    if b.store
                        .get_node(&NodeId::from(format!("v{i}").as_str()))
                        .await?
                        .is_none()
                    {
                        ok = false;
                        detail = format!("v{i} missing");
                    }
                }
                let out: BTreeMap<usize, usize> =
                    [0, n - 1].into_iter().map(|i| (i, 0usize)).collect();
                for (i, _) in out {
                    let e =
                        edges(b.store.as_ref(), Some(&format!("v{i}")), None, Some("E")).await?;
                    if e.len() != 1 {
                        ok = false;
                        detail = format!("v{i} has {} edges, want 1", e.len());
                    }
                }
                tally.check(&name, Ok(ok), &detail);
            }
            Err(e) => tally.check(&name, Err(e), ""),
        }
    }
    Ok(())
}

pub async fn run(root: &Path, backends: &[String], out: &Path) -> i32 {
    let work = out.join("conformance-work");
    let _ = std::fs::create_dir_all(&work);
    let mut exit = 0;
    for name in backends {
        let Some(kind) = BackendKind::parse(name) else {
            eprintln!("unknown backend {name}; see `ag backends`");
            return 2;
        };
        println!("== conformance {} ({})", kind.name(), kind.transport());
        let mut tally = Tally {
            pass: 0,
            fail: 0,
            unsupported: 0,
        };
        if let Err(e) = conform(kind, &work.join(kind.name()), &mut tally).await {
            if Backend::is_unsupported(&e) {
                tally.unsupported += 1;
                println!("  UNSUPPORTED {e}");
            } else {
                tally.fail += 1;
                println!("  FAIL        {e}");
            }
        }
        println!(
            "== {}: {} pass, {} fail, {} unsupported",
            kind.name(),
            tally.pass,
            tally.fail,
            tally.unsupported
        );
        if tally.fail > 0 {
            exit = 1;
        }
    }
    let _ = root;
    exit
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fixture_keeps_both_parallel_edges_and_its_isolated_vertex() {
        let g = fixture();
        let k12: Vec<String> = g
            .edges
            .iter()
            .filter(|e| e.from.as_str() == "p1" && e.to.as_str() == "p2")
            .filter_map(|e| e.id.as_ref().map(|id| id.as_str().to_string()))
            .collect();
        assert_eq!(k12, vec!["k12a", "k12b"]);
        assert!(g.nodes.iter().any(|n| n.id.as_str() == "isolated"));
        assert!(
            g.edges
                .iter()
                .any(|e| e.from.as_str() == "p1" && e.to.as_str() == "p1")
        );
    }
}
