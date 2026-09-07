//! A5 — recursive deletes. A root message and its whole reply tree are
//! deleted through the store's own delete path, one vertex at a time, root
//! first (the shape an application produces when it removes a post and
//! then its replies), while concurrent readers on their own handles keep
//! reading the tree. Afterwards the store is read back against the
//! in-process graph with the same vertices removed: a tree vertex still
//! present is a `lost_write` (the delete did not take); a vertex outside
//! the tree that vanished, an edge still pointing at a deleted vertex, or a
//! survivor's out-degree that disagrees with the oracle is a `wrong_answer`.
//! What the readers saw during the delete (a reply whose parent was
//! already gone) is recorded, not gated: without a transaction spanning
//! the whole tree every store exposes that window, and how wide it is on
//! each is the observation. Layers: L1, L3.
//!
//! Over LDBC SNB: `Message` vertices with `kind = Post`, replies through
//! `REPLY_OF` edges (child to parent), creators through `HAS_CREATOR`,
//! likes through `LIKES` (person to message). The StackOverflow temporal
//! graph named in the spec waits for a typed loader.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use grust::{EdgeQuery, NodeId, Value};

use super::Ctx;
use crate::backends::Backend;
use crate::differential::schema_of;
use crate::report::{Latency, ScenarioResult, histogram, record};

const MESSAGE: &str = "Message";
const REPLY_OF: &str = "REPLY_OF";
const HAS_CREATOR: &str = "HAS_CREATOR";
const LIKES: &str = "LIKES";

/// One reply tree: the root and every descendant, breadth-first, with each
/// vertex's parent.
struct Tree {
    root: NodeId,
    /// Root first, then by depth.
    order: Vec<NodeId>,
    parent: HashMap<NodeId, NodeId>,
}

impl Tree {
    fn size(&self) -> usize {
        self.order.len()
    }
}

/// Reply trees of the graph, largest first.
fn reply_trees(graph: &grust::Graph) -> Vec<Tree> {
    let mut children: HashMap<&NodeId, Vec<&NodeId>> = HashMap::new();
    for edge in graph.edges.iter().filter(|e| e.label.as_str() == REPLY_OF) {
        children.entry(&edge.to).or_default().push(&edge.from);
    }
    let mut trees: Vec<Tree> = graph
        .nodes
        .iter()
        .filter(|n| {
            n.label.as_str() == MESSAGE
                && matches!(n.props.get("kind"), Some(Value::String(k)) if k == "Post")
                && children.contains_key(&n.id)
        })
        .map(|root| {
            let mut order = vec![root.id.clone()];
            let mut parent = HashMap::new();
            let mut frontier = vec![&root.id];
            let mut seen: HashSet<&NodeId> = HashSet::from([&root.id]);
            while let Some(node) = frontier.pop() {
                for &child in children.get(node).into_iter().flatten() {
                    if seen.insert(child) {
                        order.push(child.clone());
                        parent.insert(child.clone(), node.clone());
                        frontier.push(child);
                    }
                }
            }
            Tree {
                root: root.id.clone(),
                order,
                parent,
            }
        })
        .collect();
    trees.sort_by(|a, b| b.size().cmp(&a.size()).then_with(|| a.root.cmp(&b.root)));
    trees
}

/// What one reader observed during the delete.
#[derive(Default, serde::Serialize)]
struct ReaderReport {
    reads: usize,
    /// Reads that found the vertex present.
    present: usize,
    /// Reads that found a reply present after its parent's delete had
    /// returned: the non-atomic window.
    orphans: usize,
    errors: usize,
}

pub async fn run(ctx: &Ctx<'_>) -> ScenarioResult {
    let mut r = ScenarioResult::new("A5", ctx.backend.kind.name(), ctx.dataset);
    if schema_of(ctx.format) != Some("ldbc-snb") {
        r.unsupported("A5 runs over LDBC SNB reply trees (the StackOverflow graph has no typed loader yet)");
        return r;
    }
    let want = if ctx.smoke { 1 } else { 3 };
    let trees: Vec<Tree> = reply_trees(ctx.graph).into_iter().take(want).collect();
    if trees.is_empty() {
        r.unsupported("no post with replies in the loaded slice");
        return r;
    }
    let deleted: HashSet<NodeId> = trees.iter().flat_map(|t| t.order.iter().cloned()).collect();

    // The store must be able to address a typed vertex by id before any
    // "gone" check means anything.
    match ctx.backend.store.get_node(&trees[0].root).await {
        Ok(Some(_)) => {}
        Ok(None) => {
            r.unsupported(&format!(
                "the adapter's get_node cannot address {} by id (typed label); A5 needs a label-aware get",
                trees[0].root.as_str()
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
    r.observe("roots", trees.iter().map(|t| t.root.as_str()).collect::<Vec<_>>());
    r.observe("tree_sizes", trees.iter().map(Tree::size).collect::<Vec<_>>());

    // Survivors to check afterwards: creators of and likers of every deleted
    // message, with the out-degree the oracle expects once the tree is gone.
    let mut survivors: BTreeMap<NodeId, usize> = BTreeMap::new();
    for edge in &ctx.graph.edges {
        let candidate = if edge.label.as_str() == HAS_CREATOR && deleted.contains(&edge.from) {
            Some(&edge.to)
        } else if edge.label.as_str() == LIKES && deleted.contains(&edge.to) {
            Some(&edge.from)
        } else {
            None
        };
        if let Some(id) = candidate
            && !deleted.contains(id)
        {
            survivors.entry(id.clone()).or_insert(0);
        }
    }
    for edge in &ctx.graph.edges {
        if let Some(expected) = survivors.get_mut(&edge.from)
            && !deleted.contains(&edge.to)
        {
            *expected += 1;
        }
    }
    r.observe("survivors_checked", survivors.len());

    // Readers on their own handles, polling the trees until the delete ends.
    let readers = if ctx.smoke { 2 } else { 4 };
    let stop = Arc::new(AtomicBool::new(false));
    let deleted_at: Arc<Mutex<HashMap<NodeId, Instant>>> = Arc::new(Mutex::new(HashMap::new()));
    let parents: Arc<HashMap<NodeId, NodeId>> = Arc::new(
        trees
            .iter()
            .flat_map(|t| t.parent.iter().map(|(c, p)| (c.clone(), p.clone())))
            .collect(),
    );
    let targets: Arc<[NodeId]> = trees
        .iter()
        .flat_map(|t| t.order.iter().cloned())
        .collect::<Vec<_>>()
        .into();
    let mut reader_tasks = Vec::with_capacity(readers);
    for i in 0..readers {
        let store = match ctx.backend.extra_handle().await {
            Ok(s) => s,
            Err(e) => {
                r.notes.push(format!("reader {i}: could not open a handle: {e}"));
                continue;
            }
        };
        let (stop, deleted_at, parents, targets) = (
            stop.clone(),
            deleted_at.clone(),
            parents.clone(),
            targets.clone(),
        );
        reader_tasks.push(tokio::spawn(async move {
            let mut report = ReaderReport::default();
            let mut cursor = i;
            while !stop.load(Ordering::Relaxed) {
                let id = &targets[cursor % targets.len()];
                cursor += readers;
                report.reads += 1;
                match store.get_node(id).await {
                    Ok(Some(_)) => {
                        report.present += 1;
                        let parent_gone = parents
                            .get(id)
                            .is_some_and(|p| deleted_at.lock().expect("deleted_at").contains_key(p));
                        if parent_gone {
                            report.orphans += 1;
                        }
                    }
                    Ok(None) => {}
                    Err(_) => report.errors += 1,
                }
            }
            report
        }));
    }

    // The delete, root first, then each level.
    let mut h = histogram();
    let mut deletes = 0usize;
    let mut delete_errors = Vec::new();
    'trees: for tree in &trees {
        for id in &tree.order {
            let t = Instant::now();
            let outcome = ctx.backend.delete_node(id).await;
            record(&mut h, t.elapsed());
            match outcome {
                Ok(()) => {
                    deletes += 1;
                    deleted_at
                        .lock()
                        .expect("deleted_at")
                        .insert(id.clone(), Instant::now());
                }
                Err(e) if Backend::is_unsupported(&e) => {
                    stop.store(true, Ordering::Relaxed);
                    for task in reader_tasks {
                        let _ = task.await;
                    }
                    r.unsupported(&format!("no delete path: {e}"));
                    return r;
                }
                Err(e) => {
                    delete_errors.push(format!("{}: {e}", id.as_str()));
                    if delete_errors.len() > 8 {
                        break 'trees;
                    }
                }
            }
        }
    }
    stop.store(true, Ordering::Relaxed);
    let mut reader_reports = Vec::with_capacity(reader_tasks.len());
    for task in reader_tasks {
        match task.await {
            Ok(report) => reader_reports.push(report),
            Err(e) => r.notes.push(format!("reader task panicked: {e}")),
        }
    }
    r.observe("deletes", deletes);
    r.observe("delete_errors", delete_errors.len());
    if let Some(sample) = delete_errors.first() {
        r.observe("delete_error_sample", sample);
    }
    r.observe("readers", &reader_reports);
    r.observe(
        "orphan_observations",
        reader_reports.iter().map(|x| x.orphans).sum::<usize>(),
    );
    if deletes > 0 {
        r.latency = Some(Latency::from_histogram(&h));
    }

    // Read back: every tree vertex gone.
    let mut surviving_tree_vertices = 0usize;
    for id in &deleted {
        match ctx.backend.store.get_node(id).await {
            Ok(Some(_)) => {
                surviving_tree_vertices += 1;
                if surviving_tree_vertices <= 4 {
                    r.notes.push(format!("{} still present after its delete", id.as_str()));
                }
            }
            Ok(None) => {}
            Err(e) => r.notes.push(format!("{}: read-back failed: {e}", id.as_str())),
        }
    }
    if surviving_tree_vertices > 0 {
        r.gates.lost_write += surviving_tree_vertices as u64;
    }
    r.observe("surviving_tree_vertices", surviving_tree_vertices);

    // Read back: survivors present, no dangling edges, degree as the oracle.
    let (mut vanished, mut dangling, mut degree_mismatches) = (0usize, 0usize, 0usize);
    for (id, expected) in &survivors {
        match ctx.backend.store.get_node(id).await {
            Ok(Some(_)) => {}
            Ok(None) => {
                vanished += 1;
                if vanished <= 4 {
                    r.notes.push(format!("{} (outside the tree) is gone", id.as_str()));
                }
                continue;
            }
            Err(e) => {
                r.notes.push(format!("{}: survivor read failed: {e}", id.as_str()));
                continue;
            }
        }
        let edges = match ctx
            .backend
            .store
            .get_edges(EdgeQuery {
                from: Some(id.clone()),
                to: None,
                label: None,
            })
            .await
        {
            Ok(edges) => edges,
            Err(e) if Backend::is_unsupported(&e) => {
                r.notes.push(format!("survivor edges not readable: {e}"));
                break;
            }
            Err(e) => {
                r.notes.push(format!("{}: edge read failed: {e}", id.as_str()));
                continue;
            }
        };
        let to_deleted = edges.iter().filter(|e| deleted.contains(&e.to)).count();
        if to_deleted > 0 {
            dangling += to_deleted;
            if dangling <= 4 {
                r.notes.push(format!(
                    "{}: {to_deleted} edges still point at deleted vertices",
                    id.as_str()
                ));
            }
        }
        if edges.len() != *expected {
            degree_mismatches += 1;
            if degree_mismatches <= 4 {
                r.notes.push(format!(
                    "{}: out-degree {} after the delete, oracle {expected}",
                    id.as_str(),
                    edges.len()
                ));
            }
        }
    }
    r.gates.wrong_answer += (vanished + dangling + degree_mismatches) as u64;
    r.observe("survivors_vanished", vanished);
    r.observe("dangling_edges", dangling);
    r.observe("degree_mismatches", degree_mismatches);
    r
}
