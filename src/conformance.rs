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

/// The two shapes the benchmarks send a store: the SNAP shape (one node
/// label `V`, one relationship type `E`, read through the scenarios' own
/// path -- `Backend::neighbors` and `out_degree`) and the typed shape of
/// the M2 datasets (labels and relationship types, read through the
/// `GraphStore` API). An adapter's answer can differ between them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Shape {
    Untyped,
    Typed,
}

impl Shape {
    fn name(self) -> &'static str {
        match self {
            Self::Untyped => "untyped (SNAP shape, scenario read path)",
            Self::Typed => "typed (labels and relationship types, GraphStore reads)",
        }
    }
}

/// The fixture: small enough to read by hand, shaped to catch what large
/// graphs hide.
pub fn fixture(shape: Shape) -> Graph {
    let mut graph = typed_fixture();
    if shape == Shape::Untyped {
        for node in &mut graph.nodes {
            node.label = Label::from(crate::dataset::NODE_LABEL);
        }
        for edge in &mut graph.edges {
            edge.label = Label::from(crate::dataset::EDGE_LABEL);
        }
    }
    graph
}

fn typed_fixture() -> Graph {
    // No parallel edges here: every harness loader dedupes on (from, label,
    // to), so the benchmarks never send a store two edges with that key,
    // and the adapters differ on it (see `parallel_edges_probe`). The
    // builder's default policy dedupes the same way.
    let mut b = Graph::builder();
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
    // A self-loop, an edge with a property, and cross-label edges.
    let _ = b.edge("KNOWS", "p1", "p1").id("loop").finish();
    let _ = b
        .edge("KNOWS", "p1", "p2")
        .id("k12a")
        .prop("since", 2019i64)
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

/// Two KNOWS edges p3->p2 with distinct ids in one batch: a capability the
/// adapters differ on (kept both / kept one / refused the batch) and the
/// benchmarks never exercise, since every loader dedupes on (from, label,
/// to). Reported as a capability line, never a failure.
pub fn parallel_edges_probe() -> Graph {
    let mut b = Graph::builder().edge_policy(EdgePolicy::AllowDuplicates);
    // the endpoints ride along: an adapter may resolve them within the batch
    let _ = b.node("Person", "p3").finish();
    let _ = b.node("Person", "p2").finish();
    let _ = b.edge("KNOWS", "p3", "p2").id("q1").finish();
    let _ = b.edge("KNOWS", "p3", "p2").id("q2").finish();
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

/// Edges `from` -> `to` (any `to` when None) under `label` (any when None):
/// through the scenarios' read path for the untyped shape, through
/// `GraphStore::get_edges` for the typed one.
async fn count_out(
    backend: &Backend,
    shape: Shape,
    from: &str,
    to: Option<&str>,
    label: Option<&str>,
) -> grust::Result<usize> {
    match shape {
        Shape::Untyped => {
            let from = NodeId::from(from);
            match to {
                None => backend.out_degree(&from).await,
                Some(to) => Ok(backend
                    .neighbors(&from)
                    .await?
                    .iter()
                    .filter(|id| id.as_str() == to)
                    .count()),
            }
        }
        Shape::Typed => Ok(edges(backend.store.as_ref(), Some(from), to, label)
            .await?
            .len()),
    }
}

async fn conform(
    kind: BackendKind,
    work_dir: &Path,
    shape: Shape,
    tally: &mut Tally,
) -> grust::Result<()> {
    let backend = Backend::open(kind, work_dir, "conformance").await?;
    let store = backend.store.as_ref();
    let graph = fixture(shape);
    // Expected cardinalities come from the fixture itself, so the same
    // checks hold in both shapes.
    let expect_out = |from: &str, to: Option<&str>, label: Option<&str>| -> usize {
        graph
            .edges
            .iter()
            .filter(|e| e.from.as_str() == from)
            .filter(|e| to.is_none_or(|t| e.to.as_str() == t))
            .filter(|e| label.is_none_or(|l| e.label.as_str() == l))
            .count()
    };
    let knows = if shape == Shape::Typed {
        "KNOWS"
    } else {
        crate::dataset::EDGE_LABEL
    };
    let works_at = if shape == Shape::Typed {
        "WORKS_AT"
    } else {
        crate::dataset::EDGE_LABEL
    };
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
    // A label that comes back in another case (SurrealDB's tables are
    // lower-case) is a capability note, not a wrong answer: no scenario
    // reads a label back. A different label, or a missing property, fails.
    let mut case_folded = false;
    for node in &graph.nodes {
        let got = store.get_node(&node.id).await;
        let name = format!("node {} reads back with label and props", node.id.as_str());
        match got {
            Ok(Some(n)) => {
                let same_label = n.label == node.label;
                let folded_label =
                    !same_label && n.label.as_str().eq_ignore_ascii_case(node.label.as_str());
                case_folded |= folded_label;
                let same_props = node.props.iter().all(|(k, v)| n.props.get(k) == Some(v));
                tally.check(
                    &name,
                    Ok((same_label || folded_label) && same_props),
                    &format!("got label {:?} props {:?}", n.label, n.props),
                );
            }
            Ok(None) => tally.check(&name, Ok(false), "missing"),
            Err(e) => tally.check(&name, Err(e), ""),
        }
    }

    if case_folded {
        println!(
            "  CAPABILITY  node labels read back case-folded (no scenario reads a label back)"
        );
    }

    // Edges: cardinalities by endpoint and label, including the loop.
    let expect: Vec<(&str, &str, Option<&str>, Option<&str>)> = vec![
        ("all edges out of p1", "p1", None, None),
        ("edge p1->p2", "p1", Some("p2"), Some(knows)),
        ("self-loop p1->p1", "p1", Some("p1"), Some(knows)),
        ("isolated vertex has no edges", "isolated", None, None),
        ("label filter out of p1", "p1", None, Some(works_at)),
    ];
    for (name, from, to, label) in expect {
        let want = expect_out(from, to, label);
        match count_out(&backend, shape, from, to, label).await {
            Ok(got) => tally.check(name, Ok(got == want), &format!("got {got} want {want}")),
            Err(e) => tally.check(name, Err(e), ""),
        }
    }
    match edges(store, None, Some("it's/odd id"), Some(works_at)).await {
        Ok(v) => tally.check(
            "edge into the escaped id",
            Ok(v.len() == 1),
            &format!("got {} want 1", v.len()),
        ),
        Err(e) => tally.check("edge into the escaped id", Err(e), ""),
    }
    match edges(store, Some("p1"), Some("p2"), Some(knows)).await {
        Ok(v) => {
            let ids: Vec<String> = v
                .iter()
                .filter_map(|e| e.id.as_ref().map(|i| i.as_str().to_string()))
                .collect();
            tally.check(
                "edge id and property read back",
                Ok(ids == ["k12a"] && v[0].props.get("since") == Some(&Value::from(2019i64))),
                &format!("ids {ids:?} props {:?}", v.first().map(|e| &e.props)),
            );
        }
        Err(e) => tally.check("edge id and property read back", Err(e), ""),
    }

    // Capability, not conformance (typed shape only; the answers do not
    // depend on labels): an edge batch that does not carry its
    // endpoints, which the store already holds. The compact loader's edge
    // chunks are shaped like this unless the backend declares otherwise
    // (`BackendKind::edge_batches_carry_endpoints`); a mismatch here is a
    // harness defect to fix before any large tier.
    if shape == Shape::Typed {
        let mut eo = Graph::builder();
        let _ = eo.edge("KNOWS", "p2", "p3").id("eo1").finish();
        let accepted = match backend.load(&eo.build()).await {
            Err(e) => format!("refused: {e}"),
            Ok(_) => match edges(store, Some("p2"), Some("p3"), Some("KNOWS")).await {
                Ok(v) if v.len() == 1 => "accepted".to_string(),
                Ok(v) => format!("accepted but read back {} edges", v.len()),
                Err(e) => format!("read failed: {e}"),
            },
        };
        println!("  CAPABILITY  edge batch without its endpoints: {accepted}");
        let declared = kind.edge_batches_carry_endpoints();
        tally.check(
            "compact edge chunks are shaped for this adapter",
            Ok(declared == accepted.starts_with("refused")),
            &format!("adapter {accepted}, harness declares carry_endpoints={declared}"),
        );

        // Capability, not conformance: two parallel edges in one batch.
        let outcome = match backend.load(&parallel_edges_probe()).await {
            Err(e) => format!("refused the batch: {e}"),
            Ok(_) => match edges(store, Some("p3"), Some("p2"), Some("KNOWS")).await {
                Err(e) => format!("read failed after the batch: {e}"),
                Ok(v) => {
                    let mut ids: Vec<String> = v
                        .iter()
                        .filter_map(|e| e.id.as_ref().map(|i| i.as_str().to_string()))
                        .collect();
                    ids.sort();
                    match ids.len() {
                        2 => format!("kept both {ids:?}"),
                        1 => format!("kept one {ids:?}"),
                        n => format!("kept {n} {ids:?}"),
                    }
                }
            },
        };
        println!(
            "  CAPABILITY  parallel edges (same from/label/to, distinct ids) in one batch: {outcome}; not exercised by the benchmarks, whose loaders dedupe that key"
        );
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
            match count_out(&backend, shape, "p3", None, None).await {
                Ok(n) => tally.check(
                    "deleted node's edges are gone",
                    Ok(n == 0),
                    &format!("{n} edges remain"),
                ),
                Err(e) => tally.check("deleted node's edges are gone", Err(e), ""),
            }
        }
        Err(e) => tally.check("delete node", Err(e), ""),
    }

    // Batch boundary: N-1, N, N+1 rows around the incremental batch size
    // (V/E rows; once, in the untyped shape).
    for n in [BATCH_BOUNDARY - 1, BATCH_BOUNDARY, BATCH_BOUNDARY + 1] {
        if shape == Shape::Typed {
            break;
        }
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
    // 1: the untyped shape -- what every published SNAP row relies on --
    // failed somewhere; 3: only the typed shape failed (the M2 families
    // already record such an adapter as unsupported); 0: clean.
    let mut untyped_failed = Vec::new();
    let mut typed_failed = Vec::new();
    for name in backends {
        let Some(kind) = BackendKind::parse(name) else {
            eprintln!("unknown backend {name}; see `ag backends`");
            return 2;
        };
        for shape in [Shape::Untyped, Shape::Typed] {
            println!(
                "== conformance {} ({}), {}",
                kind.name(),
                kind.transport(),
                shape.name()
            );
            let mut tally = Tally {
                pass: 0,
                fail: 0,
                unsupported: 0,
            };
            let dir = work.join(format!("{}-{:?}", kind.name(), shape).to_lowercase());
            if let Err(e) = conform(kind, &dir, shape, &mut tally).await {
                if Backend::is_unsupported(&e) {
                    tally.unsupported += 1;
                    println!("  UNSUPPORTED {e}");
                } else {
                    tally.fail += 1;
                    println!("  FAIL        {e}");
                }
            }
            println!(
                "== {} {:?}: {} pass, {} fail, {} unsupported",
                kind.name(),
                shape,
                tally.pass,
                tally.fail,
                tally.unsupported
            );
            if tally.fail > 0 {
                match shape {
                    Shape::Untyped => untyped_failed.push(kind.name()),
                    Shape::Typed => typed_failed.push(kind.name()),
                }
            }
        }
    }
    let _ = root;
    if !untyped_failed.is_empty() {
        println!(
            "== NOT CONFORMANT (untyped shape): {}",
            untyped_failed.join(", ")
        );
    }
    if !typed_failed.is_empty() {
        println!("== typed shape not conformant: {}", typed_failed.join(", "));
    }
    if !untyped_failed.is_empty() {
        1
    } else if !typed_failed.is_empty() {
        3
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fixture_has_its_edges_and_the_probe_keeps_both_parallel_edges() {
        let g = fixture(Shape::Typed);
        let u = fixture(Shape::Untyped);
        assert!(u.nodes.iter().all(|n| n.label.as_str() == "V"));
        assert!(u.edges.iter().all(|e| e.label.as_str() == "E"));
        assert_eq!(u.edges.len(), g.edges.len());
        let k12: Vec<String> = g
            .edges
            .iter()
            .filter(|e| e.from.as_str() == "p1" && e.to.as_str() == "p2")
            .filter_map(|e| e.id.as_ref().map(|id| id.as_str().to_string()))
            .collect();
        assert_eq!(k12, vec!["k12a"]);
        assert_eq!(parallel_edges_probe().edges.len(), 2);
        assert!(g.nodes.iter().any(|n| n.id.as_str() == "isolated"));
        assert!(
            g.edges
                .iter()
                .any(|e| e.from.as_str() == "p1" && e.to.as_str() == "p1")
        );
    }
}
