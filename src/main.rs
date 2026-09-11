//! `ag` — GRAPH-ADVERSARIAL-v1 harness CLI.
//!
//!   ag run [--dataset NAME,...] [--backend NAME,...] [--scenario A1,...]
//!          [--smoke] [--limit-edges N] [--out DIR]
//!   ag conformance --backend a,b     typed multigraph read-back through each adapter; exit 1 on any FAIL
//!   ag datasets                  list the datasets in datasets/MANIFEST.json
//!   ag backends                  list built-in backends

#[cfg(feature = "age")]
mod age;
mod backends;
mod budget;
mod compact;
mod conformance;
mod dataset;
mod differential;
#[cfg(feature = "falkor")]
mod falkor_reader;
mod isolation;
#[cfg(feature = "neo4j")]
mod neo4j;
#[cfg(feature = "neo4j")]
mod neo4j_http;
mod oracle;
mod probe;
mod report;
mod scenarios;
mod typed_load;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use backends::{Backend, BackendKind};
use oracle::Oracle;
use report::Report;
use scenarios::Ctx;

fn usage() -> ! {
    eprintln!(
        "{}",
        include_str!("main.rs")
            .lines()
            .skip(1)
            .take(7)
            .map(|l| l.trim_start_matches("//!"))
            .collect::<Vec<_>>()
            .join("\n")
    );
    std::process::exit(2)
}

#[derive(Debug)]
struct Args {
    command: String,
    datasets: Vec<String>,
    backends: Vec<String>,
    scenarios: Vec<String>,
    smoke: bool,
    limit_edges: Option<usize>,
    out: PathBuf,
}

fn parse_args() -> Args {
    let mut argv = std::env::args().skip(1);
    let command = argv.next().unwrap_or_else(|| usage());
    let mut args = Args {
        command,
        datasets: vec!["wiki-Talk".into()],
        backends: vec!["memory".into(), "turso-wal".into(), "turso-mvcc".into()],
        scenarios: scenarios::all().iter().map(|s| s.to_string()).collect(),
        smoke: false,
        limit_edges: None,
        out: PathBuf::from("reports"),
    };
    while let Some(flag) = argv.next() {
        let mut value = || argv.next().unwrap_or_else(|| usage());
        match flag.as_str() {
            "--dataset" | "--datasets" => {
                args.datasets = value().split(',').map(String::from).collect()
            }
            "--backend" | "--backends" => {
                args.backends = value().split(',').map(String::from).collect()
            }
            "--scenario" | "--scenarios" => {
                args.scenarios = value().split(',').map(String::from).collect()
            }
            "--limit-edges" => args.limit_edges = value().parse().ok(),
            "--out" => args.out = PathBuf::from(value()),
            "--smoke" => args.smoke = true,
            _ => usage(),
        }
    }
    args
}

#[derive(serde::Deserialize, serde::Serialize, Clone)]
struct ManifestEntry {
    name: String,
    tier: String,
    url: String,
    file: String,
    bytes: u64,
    sha256: String,
    pathology: String,
}

#[derive(serde::Deserialize)]
struct Manifest {
    datasets: Vec<ManifestEntry>,
}

fn manifest(root: &Path) -> Manifest {
    let path = root.join("datasets/MANIFEST.json");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        eprintln!(
            "cannot read {}: {e}; run scripts/fetch-datasets.sh",
            path.display()
        );
        std::process::exit(1)
    });
    serde_json::from_str(&text).expect("manifest json")
}

#[tokio::main]
async fn main() {
    let args = parse_args();
    let root = std::env::current_dir().expect("cwd");
    match args.command.as_str() {
        "datasets" => {
            for d in manifest(&root).datasets {
                println!(
                    "{:<26} {} {:>12} B  {}",
                    d.name, d.tier, d.bytes, d.pathology
                );
            }
        }
        "backends" => {
            for b in BackendKind::all() {
                println!("{:<14} {}", b.name(), b.transport());
            }
            println!(
                "(network/embedded backends need --features postgres,surreal,falkor,lancedb,helix,ladybug,neo4j and compose.yaml)"
            );
        }
        "run" => run(&root, &args).await,
        "conformance" => {
            let code = conformance::run(&root, &args.backends, &args.out).await;
            std::process::exit(code);
        }
        _ => usage(),
    }
}

async fn run(root: &Path, args: &Args) {
    let manifest = manifest(root);
    let by_name: BTreeMap<String, ManifestEntry> = manifest
        .datasets
        .iter()
        .map(|d| (d.name.clone(), d.clone()))
        .collect();
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    // Validate every requested id before any work: a run that quietly skips
    // an unknown dataset or backend and then reports itself complete is how a
    // cell disappears (§35's `unknown backend age` runs did exactly that).
    let unknown_datasets: Vec<&String> = args
        .datasets
        .iter()
        .filter(|d| !by_name.contains_key(d.as_str()))
        .collect();
    let unknown_backends: Vec<&String> = args
        .backends
        .iter()
        .filter(|b| BackendKind::parse(b).is_none())
        .collect();
    if !unknown_datasets.is_empty() || !unknown_backends.is_empty() {
        for d in &unknown_datasets {
            eprintln!("unknown dataset {d}; see `ag datasets`");
        }
        for b in &unknown_backends {
            eprintln!("unknown backend {b}; see `ag backends`");
        }
        eprintln!(
            "refusing to run: every requested dataset and backend must exist before any cell is attempted"
        );
        std::process::exit(2);
    }
    let out_dir = args.out.join(&stamp);
    std::fs::create_dir_all(&out_dir).expect("report dir");
    let work_dir = out_dir.join("work");
    let mut report = Report::new();
    report.summary.insert("smoke".into(), args.smoke.into());
    let report_path = out_dir.join("report.json");
    let jsonl_path = out_dir.join("results.jsonl");
    // Persist after every result so a crash never loses completed scenarios.
    let persist = |report: &mut Report, result: &report::ScenarioResult| {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&jsonl_path)
            && let Ok(line) = serde_json::to_string(result)
        {
            let _ = writeln!(f, "{line}");
        }
        report.finalize();
        // A bundle written here is a partial one: a cap, a kill or a crash
        // can end the run before the final write flips this to true, and
        // the renderer marks such rows as "run ended after this row".
        report.summary.insert("complete".into(), false.into());
        let tmp = report_path.with_extension("json.tmp");
        if std::fs::write(&tmp, serde_json::to_string_pretty(report).expect("json")).is_ok() {
            let _ = std::fs::rename(&tmp, &report_path);
        }
    };

    let phase_rss = std::env::var_os("AG_PHASE_RSS").is_some();
    let rss_line = |phase: &str| {
        if phase_rss {
            if let Some(b) = probe::current_rss_bytes() {
                eprintln!("   phase-rss {phase} {:.2} GB", b as f64 / 1024f64.powi(3));
            }
        }
    };
    // Every (dataset, backend) requested must have at least one row before the
    // report may call itself complete.
    let mut expected: Vec<(String, String)> = Vec::new();
    let mut load_budget_hit = false;
    for d in &args.datasets {
        for b in &args.backends {
            expected.push((d.clone(), b.clone()));
        }
    }

    for dataset_name in &args.datasets {
        let Some(entry) = by_name.get(dataset_name) else {
            eprintln!("unknown dataset {dataset_name}; see `ag datasets`");
            continue;
        };
        let path = root.join("datasets").join(&entry.file);
        let limit = args
            .limit_edges
            .or(if args.smoke { Some(200_000) } else { None });
        // The compact reference (§46): a SNAP tier whose parsed `Graph`
        // would not fit the host is parsed into a CSR instead and fed to the
        // store in chunks. Decided by the manifest's file size against
        // AG_COMPACT_ABOVE_MB (default 50: cit-Patents and above; wiki-Talk,
        // web-Google and roadNet-CA stay materialized),
        // or forced either way with AG_COMPACT=1|0.
        let compact = match std::env::var("AG_COMPACT").ok().as_deref() {
            Some("1") => true,
            Some("0") => false,
            _ => {
                let above_mb: u64 = std::env::var("AG_COMPACT_ABOVE_MB")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(50);
                entry.bytes >= above_mb * 1024 * 1024
            }
        };
        let chunk_edges: usize = std::env::var("AG_CHUNK_EDGES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(5_000_000);
        eprintln!("== loading {dataset_name} ({})", path.display());
        let t = std::time::Instant::now();
        let (loaded, stats, schema) = match dataset::load_dataset(&path, limit, compact) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("load failed: {e}");
                for backend_name in &args.backends {
                    let mut r = report::ScenarioResult::new("LOAD", backend_name, dataset_name);
                    r.setup_failed(&format!("dataset did not load: {e}"));
                    report.push(r.clone());
                    persist(&mut report, &r);
                }
                continue;
            }
        };
        rss_line("after-graph");
        eprintln!(
            "   {} nodes, {} edges in {:?} [{} reference]{}",
            stats.nodes,
            stats.edges,
            t.elapsed(),
            loaded.reference_name(),
            if schema.is_untyped() {
                String::new()
            } else {
                format!(
                    " ({} node labels, {} relationship types)",
                    schema.node_labels.len(),
                    schema.relationship_labels.len()
                )
            }
        );
        let typed = !schema.is_untyped();
        let format = stats.format.clone();
        let skipped: Vec<&str> = args
            .scenarios
            .iter()
            .map(String::as_str)
            .filter(|id| !scenarios::applies(id, typed))
            .collect();
        if !skipped.is_empty() {
            eprintln!(
                "   skipping {} (not defined over this dataset's shape)",
                skipped.join(",")
            );
        }
        report.datasets.push(serde_json::json!({
            "manifest": entry,
            "load": stats,
            "schema": schema,
            "reference": loaded.reference_name(),
            "skipped_scenarios": skipped
        }));
        let (graph, compact_graph) = match &loaded {
            dataset::LoadedGraph::Full(g) => (Some(g), None),
            dataset::LoadedGraph::Compact(c) => (None, Some(c)),
        };
        let oracle = match (graph, compact_graph) {
            (_, Some(c)) => Ok(Oracle::compact(c, schema)),
            (Some(g), None) => Oracle::with_schema(g, schema),
            (None, None) => unreachable!("a loaded dataset is full or compact"),
        };
        let oracle = match oracle {
            Ok(o) => o,
            Err(e) => {
                eprintln!("oracle failed: {e}");
                for backend_name in &args.backends {
                    let mut r = report::ScenarioResult::new("LOAD", backend_name, dataset_name);
                    r.setup_failed(&format!("oracle could not be built: {e}"));
                    report.push(r.clone());
                    persist(&mut report, &r);
                }
                continue;
            }
        };
        rss_line("after-oracle");
        for backend_name in &args.backends {
            let Some(kind) = BackendKind::parse(backend_name) else {
                eprintln!("unknown backend {backend_name}; see `ag backends`");
                continue;
            };
            eprintln!("-- backend {}", kind.name());
            let backend = match Backend::open(kind, &work_dir, dataset_name).await {
                Ok(b) => std::sync::Arc::new(b),
                Err(e) => {
                    // A backend that cannot be opened is a failed LOAD row, never
                    // a missing one: the report keeps the reason and the gate.
                    eprintln!("   open failed: {e}");
                    let mut result = report::ScenarioResult::new("LOAD", kind.name(), dataset_name);
                    result.observe("transport", kind.transport());
                    result.gates.oom_or_crash += 1;
                    result.notes.push(format!("open failed: {e}"));
                    result.finish();
                    report.push(result.clone());
                    persist(&mut report, &result);
                    continue;
                }
            };
            let t = std::time::Instant::now();
            let load_probe = probe::Probe::start(kind.container());
            let mut load_result = report::ScenarioResult::new("LOAD", kind.name(), dataset_name);
            let load_budget = budget::load_budget();
            // A tier the store's own measured rate on this host says cannot
            // load inside the budget is not sent to spend it: the row says
            // what was projected from what, and the families do not run.
            if let Some(cap) = load_budget
                && budget::predict_enabled()
                && let Some(rate) = budget::measured_rate(&args.out, kind.name())
                && rate.projected(stats.edges as u64) > cap
            {
                let projected = rate.projected(stats.edges as u64);
                let why = format!(
                    "not attempted: {} edges at the store's measured {:.0} edges/s on this host ({} edges, {} in {}) is {:.1} h against the {:.1} h load budget",
                    stats.edges,
                    rate.edges_per_s,
                    rate.edges,
                    rate.dataset,
                    rate.run,
                    projected.as_secs_f64() / 3600.0,
                    cap.as_secs_f64() / 3600.0
                );
                eprintln!("   {why}");
                load_result.observe("transport", kind.transport());
                load_result.observe("projected_load_s", projected.as_secs());
                load_result.observe("measured_edges_per_s", rate.edges_per_s);
                load_result.not_attempted(&why);
                report.push(load_result.clone());
                persist(&mut report, &load_result);
                continue;
            }
            if let Some(cap) = load_budget {
                load_result.observe("load_budget_s", cap.as_secs());
            }
            // The load runs on its own task so the budget's timer is polled
            // whatever the adapter does: an adapter that never yields to the
            // runtime (the in-process store's load is synchronous; a Bolt
            // batch under memory pressure can hold for hours) cannot be
            // interrupted by a timeout wrapped around its own future, and on
            // 2026-09-11 a six-hour budget let a load run past it unbounded.
            // A task the timer beats is aborted; a task blocked in a
            // synchronous call keeps its thread until the process exits,
            // which is why the run ends with process::exit.
            let mut load_task = {
                let b = std::sync::Arc::clone(&backend);
                let g = graph.map(std::sync::Arc::clone);
                let c = compact_graph.map(std::sync::Arc::clone);
                tokio::spawn(async move {
                    match (g, c) {
                        (Some(g), _) => b.load(&g).await.map(|rep| (rep, None)),
                        (None, Some(c)) => b
                            .load_compact(&c, chunk_edges)
                            .await
                            .map(|(rep, chunks)| (rep, Some(chunks))),
                        (None, None) => unreachable!(),
                    }
                })
            };
            let loaded = match load_budget {
                Some(cap) => match tokio::time::timeout(cap, &mut load_task).await {
                    Ok(joined) => Ok(joined.unwrap_or_else(|e| {
                        Err(grust::GrustError::Backend(format!("load task failed: {e}")))
                    })),
                    Err(elapsed) => {
                        load_task.abort();
                        load_budget_hit = true;
                        Err(elapsed)
                    }
                },
                None => Ok(load_task.await.unwrap_or_else(|e| {
                    Err(grust::GrustError::Backend(format!("load task failed: {e}")))
                })),
            };
            let loaded_report = match loaded {
                Ok(Ok((rep, chunks))) => {
                    if let Some(chunks) = chunks {
                        load_result.observe("reference", "compact");
                        load_result.observe("load_chunks", chunks);
                        load_result.observe("chunk_edges", chunk_edges);
                    }
                    Ok(rep)
                }
                Ok(Err(e)) => Err(e),
                // The load did not finish inside its budget: a finding with
                // its gate, and the families do not run on a partial store.
                Err(_) => Err(grust::GrustError::Backend(format!(
                    "the load did not finish inside the {} s load budget ({} nodes, {} edges offered)",
                    load_budget.map(|c| c.as_secs()).unwrap_or(0),
                    stats.nodes,
                    stats.edges
                ))),
            };
            let load_timed_out =
                matches!(loaded_report, Err(ref e) if e.to_string().contains("load budget"));
            match loaded_report {
                Ok(rep) => {
                    eprintln!(
                        "   loaded {} nodes / {} edges in {:?}",
                        rep.nodes,
                        rep.edges,
                        t.elapsed()
                    );
                    load_result.observe("load_path", backend.read_path());
                    load_result.observe("transport", kind.transport());
                    if let Some(profile) = kind.profile() {
                        load_result.observe("profile", profile);
                    }
                    load_result.observe("nodes", rep.nodes);
                    load_result.observe("edges", rep.edges);
                    // A load the store reports short is not a load: the
                    // compact path once delivered every vertex and no edge to
                    // three stores while this row read "pass".
                    if rep.nodes != stats.nodes || rep.edges != stats.edges {
                        load_result.gates.lost_write += 1;
                        load_result.notes.push(format!(
                            "the store reported {} of {} nodes and {} of {} edges offered",
                            rep.nodes, stats.nodes, rep.edges, stats.edges
                        ));
                    }
                    load_result.observe(
                        "edges_per_s",
                        rep.edges as f64 / t.elapsed().as_secs_f64().max(1e-9),
                    );
                }
                Err(e) => {
                    let text = e.to_string();
                    eprintln!(
                        "   load failed: {}",
                        text.chars().take(300).collect::<String>()
                    );
                    // A multigraph refused by a structural edge key: the
                    // Grust SQL adapters upsert an edge on (from, label, to),
                    // and PostgreSQL rejects a batch that repeats that key
                    // ("ON CONFLICT DO UPDATE command cannot affect row a
                    // second time"). The store said no, typed, and is up:
                    // parallel edges are not preserved on this route, which
                    // is the tier's pathology answered, not a crash.
                    let structural_key_refusal = stats.parallel_edges > 0
                        && text.contains("ON CONFLICT (from_id, label, to_id)");
                    if structural_key_refusal {
                        load_result.unsupported(
                            "parallel edges are not preserved: the adapter keys an edge on (from, label, to) and PostgreSQL refuses a batch that repeats the key (ON CONFLICT DO UPDATE cannot affect a row a second time)",
                        );
                    } else if load_timed_out {
                        load_result.gates.hang_or_timeout_without_refusal += 1;
                    } else {
                        load_result.gates.oom_or_crash += 1;
                    }
                    // The adapters put the whole statement in the message;
                    // the row keeps its head and tail, not 20 KB of VALUES.
                    let note = if text.chars().count() > 1_200 {
                        let head: String = text.chars().take(800).collect();
                        let tail: String = text
                            .chars()
                            .rev()
                            .take(300)
                            .collect::<Vec<_>>()
                            .into_iter()
                            .rev()
                            .collect();
                        format!(
                            "{head} … [{} chars elided] … {tail}",
                            text.chars().count() - 1_100
                        )
                    } else {
                        text
                    };
                    load_result.notes.push(note);
                    // The container's own state, when there is one: an HTTP
                    // or Bolt error after the kernel took the store at its
                    // memory limit reads as a transport failure otherwise.
                    if let Some(state) = kind.container().and_then(probe::container_state) {
                        eprintln!("   {state}");
                        load_result.notes.push(state);
                    }
                }
            }
            load_result.wall_ms = t.elapsed().as_millis();
            load_probe.finish(&mut load_result);
            rss_line("after-store-load");
            load_result.finish();
            // The families run only on a store that holds the graph: a load
            // that failed, ran past its budget, was refused, or was not
            // attempted leaves nothing to measure (PostgreSQL at
            // sx-stackoverflow, 2026-09-11: an unsupported load followed by
            // 6,444 gates against an empty store).
            let load_failed = load_result.gates.total() > 0
                || !matches!(load_result.outcome, report::Outcome::Pass);
            eprintln!(
                "   LOAD {:<12} {:?}  cpu={:.2}  load1m={}",
                load_result.backend,
                load_result.outcome,
                load_result
                    .observations
                    .get("client_cpu_ratio")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0),
                load_result
                    .observations
                    .get("host_loadavg_1m_end")
                    .cloned()
                    .unwrap_or_default()
            );
            report.push(load_result.clone());
            persist(&mut report, &load_result);
            if load_failed {
                continue;
            }
            let hub_writes = std::sync::atomic::AtomicUsize::new(0);
            for scenario in args
                .scenarios
                .iter()
                .filter(|id| scenarios::applies(id, typed))
            {
                let ctx = Ctx {
                    dataset: dataset_name,
                    format: &format,
                    graph,
                    compact: compact_graph.map(|c| &**c),
                    oracle: &oracle,
                    backend: &backend,
                    smoke: args.smoke,
                    hub_writes: &hub_writes,
                };
                let mut result = scenarios::run(scenario, &ctx).await;
                if result.gates.oom_or_crash > 0
                    && let Some(state) = kind.container().and_then(probe::container_state)
                    && state.contains("exit")
                {
                    result.notes.push(state);
                }
                eprintln!(
                    "   {:<3} {:<12} {:?}  gates={}  wall={}ms cpu={:.2} load1m={}{}  {}",
                    result.scenario,
                    result.backend,
                    result.outcome,
                    result.gates.total(),
                    result.wall_ms,
                    result
                        .observations
                        .get("client_cpu_ratio")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(0.0),
                    result
                        .observations
                        .get("host_loadavg_1m_end")
                        .cloned()
                        .unwrap_or_default(),
                    result
                        .observations
                        .get("server_cpu_us")
                        .and_then(|v| v.as_u64())
                        .map(|c| format!(" server_cpu={}ms", c / 1000))
                        .unwrap_or_default(),
                    result.notes.join(" | ")
                );
                report.push(result.clone());
                persist(&mut report, &result);
            }
        }
    }
    report.finalize();
    let path = report_path.clone();
    // Complete means every requested cell is accounted for -- by a pass, a
    // failure, a refusal or a recorded setup failure -- never by silence.
    let missing: Vec<String> = expected
        .iter()
        .filter(|(d, b)| {
            !report
                .results
                .iter()
                .any(|r| &r.dataset == d && &r.backend == b)
        })
        .map(|(d, b)| format!("{d}/{b}"))
        .collect();
    report
        .summary
        .insert("expected_cells".into(), (expected.len() as u64).into());
    report
        .summary
        .insert("missing_cells".into(), missing.clone().into());
    report
        .summary
        .insert("complete".into(), missing.is_empty().into());
    std::fs::write(&path, serde_json::to_string_pretty(&report).expect("json"))
        .expect("write report");
    let _ = std::fs::remove_dir_all(&work_dir);
    eprintln!(
        "== report: {}  hard_gate_total={}  outcomes={}",
        path.display(),
        report.gates.total(),
        report.summary["outcomes"]
    );
    // An explicit exit: a load task the budget abandoned may still hold a
    // worker thread in a synchronous call, and the runtime's shutdown would
    // wait for it.
    if load_budget_hit {
        eprintln!("== a load ran past its budget; ending the process without waiting for it");
    }
    std::process::exit(if report.gates.total() > 0 { 1 } else { 0 });
}
