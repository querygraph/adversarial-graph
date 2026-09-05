//! `ag` — GRAPH-ADVERSARIAL-v1 harness CLI.
//!
//!   ag run [--dataset NAME,...] [--backend NAME,...] [--scenario A1,...]
//!          [--smoke] [--limit-edges N] [--out DIR]
//!   ag datasets                  list the datasets in datasets/MANIFEST.json
//!   ag backends                  list built-in backends

mod backends;
mod dataset;
#[cfg(feature = "falkor")]
mod falkor_reader;
#[cfg(feature = "neo4j")]
mod neo4j;
#[cfg(feature = "neo4j")]
mod neo4j_http;
mod oracle;
mod probe;
mod report;
mod scenarios;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use backends::{Backend, BackendKind};
use oracle::Oracle;
use report::Report;
use scenarios::Ctx;

fn usage() -> ! {
    eprintln!("{}", include_str!("main.rs").lines().skip(1).take(6).map(|l| l.trim_start_matches("//!")).collect::<Vec<_>>().join("\n"));
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
            "--dataset" => args.datasets = value().split(',').map(String::from).collect(),
            "--backend" => args.backends = value().split(',').map(String::from).collect(),
            "--scenario" => args.scenarios = value().split(',').map(String::from).collect(),
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
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| { eprintln!("cannot read {}: {e}; run scripts/fetch-datasets.sh", path.display()); std::process::exit(1) });
    serde_json::from_str(&text).expect("manifest json")
}

#[tokio::main]
async fn main() {
    let args = parse_args();
    let root = std::env::current_dir().expect("cwd");
    match args.command.as_str() {
        "datasets" => {
            for d in manifest(&root).datasets {
                println!("{:<26} {} {:>12} B  {}", d.name, d.tier, d.bytes, d.pathology);
            }
        }
        "backends" => {
            for b in BackendKind::all() {
                println!("{:<14} {}", b.name(), b.transport());
            }
            println!("(network/embedded backends need --features postgres,surreal,falkor,lancedb,helix,ladybug,neo4j and compose.yaml)");
        }
        "run" => run(&root, &args).await,
        _ => usage(),
    }
}

async fn run(root: &Path, args: &Args) {
    let manifest = manifest(root);
    let by_name: BTreeMap<String, ManifestEntry> = manifest.datasets.iter().map(|d| (d.name.clone(), d.clone())).collect();
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
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
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&jsonl_path)
            && let Ok(line) = serde_json::to_string(result)
        {
            let _ = writeln!(f, "{line}");
        }
        report.finalize();
        let tmp = report_path.with_extension("json.tmp");
        if std::fs::write(&tmp, serde_json::to_string_pretty(report).expect("json")).is_ok() {
            let _ = std::fs::rename(&tmp, &report_path);
        }
    };

    for dataset_name in &args.datasets {
        let Some(entry) = by_name.get(dataset_name) else {
            eprintln!("unknown dataset {dataset_name}; see `ag datasets`");
            continue;
        };
        let path = root.join("datasets").join(&entry.file);
        let limit = args.limit_edges.or(if args.smoke { Some(200_000) } else { None });
        eprintln!("== loading {dataset_name} ({})", path.display());
        let t = std::time::Instant::now();
        let (graph, stats) = match dataset::load_snap_edge_list(&path, limit) {
            Ok(v) => v,
            Err(e) => { eprintln!("load failed: {e}"); continue; }
        };
        eprintln!("   {} nodes, {} edges in {:?}", stats.nodes, stats.edges, t.elapsed());
        report.datasets.push(serde_json::json!({ "manifest": entry, "load": stats }));
        let oracle = match Oracle::new(&graph) {
            Ok(o) => o,
            Err(e) => { eprintln!("oracle failed: {e}"); continue; }
        };
        for backend_name in &args.backends {
            let Some(kind) = BackendKind::parse(backend_name) else {
                eprintln!("unknown backend {backend_name}; see `ag backends`");
                continue;
            };
            eprintln!("-- backend {}", kind.name());
            let backend = match Backend::open(kind, &work_dir, dataset_name).await {
                Ok(b) => b,
                Err(e) => { eprintln!("   open failed: {e}"); continue; }
            };
            let t = std::time::Instant::now();
            let load_probe = probe::Probe::start(kind.container());
            let mut load_result = report::ScenarioResult::new("LOAD", kind.name(), dataset_name);
            match backend.load(&graph).await {
                Ok(rep) => {
                    eprintln!("   loaded {} nodes / {} edges in {:?}", rep.nodes, rep.edges, t.elapsed());
                    load_result.observe("load_path", backend.read_path());
                    load_result.observe("transport", kind.transport());
                    load_result.observe("nodes", rep.nodes);
                    load_result.observe("edges", rep.edges);
                    load_result.observe("edges_per_s", rep.edges as f64 / t.elapsed().as_secs_f64().max(1e-9));
                }
                Err(e) => { eprintln!("   load failed: {e}"); load_result.gates.oom_or_crash += 1; load_result.notes.push(e.to_string()); }
            }
            load_result.wall_ms = t.elapsed().as_millis();
            load_probe.finish(&mut load_result);
            load_result.finish();
            let load_failed = load_result.gates.total() > 0;
            eprintln!("   LOAD {:<12} {:?}  cpu={:.2}  load1m={}", load_result.backend, load_result.outcome,
                load_result.observations.get("client_cpu_ratio").and_then(|v| v.as_f64()).unwrap_or(0.0),
                load_result.observations.get("host_loadavg_1m_end").cloned().unwrap_or_default());
            persist(&mut report, &load_result);
            report.push(load_result);
            if load_failed { continue; }
            for scenario in &args.scenarios {
                let ctx = Ctx { dataset: dataset_name, graph: &graph, oracle: &oracle, backend: &backend, smoke: args.smoke };
                let result = scenarios::run(scenario, &ctx).await;
                eprintln!(
                    "   {:<3} {:<12} {:?}  gates={}  wall={}ms cpu={:.2} load1m={}{}  {}",
                    result.scenario, result.backend, result.outcome, result.gates.total(), result.wall_ms,
                    result.observations.get("client_cpu_ratio").and_then(|v| v.as_f64()).unwrap_or(0.0),
                    result.observations.get("host_loadavg_1m_end").cloned().unwrap_or_default(),
                    result.observations.get("server_cpu_us").and_then(|v| v.as_u64()).map(|c| format!(" server_cpu={}ms", c / 1000)).unwrap_or_default(),
                    result.notes.join(" | ")
                );
                persist(&mut report, &result);
                report.push(result);
            }
        }
    }
    report.finalize();
    let path = report_path.clone();
    std::fs::write(&path, serde_json::to_string_pretty(&report).expect("json")).expect("write report");
    let _ = std::fs::remove_dir_all(&work_dir);
    eprintln!("== report: {}  hard_gate_total={}  outcomes={}", path.display(), report.gates.total(), report.summary["outcomes"]);
    if report.gates.total() > 0 {
        std::process::exit(1);
    }
}
