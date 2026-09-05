# Handoff: running GRAPH-ADVERSARIAL-v1 on the dedicated EC2 host

Written 2026-09-05 by the laptop session for the session running on the EC2
host (`lakecat`, Debian 13, 4 vCPU, 15 GiB RAM, 200 GB disk). The laptop
numbers in `reports/` and `RESULTS.md` were taken on a shared host with a
1-minute load average of 300–700; the point of moving here is clean numbers.

## What is already on this host

- `~/src/adversarial-graph/` — this repo (rsync of the laptop tree at the
  commit named below, minus `target/` and `reports/*/work`).
- `datasets/` — every S/M-tier file under 1.5 GB (17 files, 2.4 GB). The L
  tier (twitter-2010, com-friendster) was not copied; `scripts/fetch-datasets.sh`
  fetches it if wanted.
- `~/bootstrap.log` — the bootstrap that installed build-essential, cmake,
  clang, pkg-config, libssl-dev, protobuf-compiler, Docker (get.docker.com,
  compose plugin) and rustup (minimal profile). It ends with `BOOTSTRAP_DONE`
  when finished; `admin` was added to the `docker` group (re-login for it).

## Build

```
source ~/.cargo/env
cd ~/src/adversarial-graph
cargo build --release --features postgres,surreal,falkor,lancedb,neo4j,helix,ladybug
```

Took 37 min on the loaded laptop; expect ~1 h on 4 vCPU. Two facts to know:

- `grust-helix` and `grust-ladybug` are `publish = false` in Grust, so they
  are `git` dependencies on the `v0.13.0` tag, with `[patch.crates-io]`
  pointing `grust-core` at the same tag so registry and tagged crates share
  one `GraphStore` (see `Cargo.toml`). Never point them at a local checkout.
- `lbug` (Ladybug engine) downloads a prebuilt static library into
  `~/.cache/lbug-prebuilt` when one exists for the target; otherwise it builds
  the C++ engine from source (long). The link emits duplicate-symbol warnings
  (`simsimd`, `zstd` bundled in liblbug vs the Lance crates); the laptop
  binary linked and ran, but watch for it.

## Run: one system at a time

`scripts/run-ladder.sh` starts only the container a backend needs, runs the
requested datasets, stops it, and moves on. That is the mode for this host.

```
./target/release/ag backends                      # every backend + transport
scripts/run-ladder.sh                             # all backends, wiki-Talk + roadNet-CA smoke
scripts/run-ladder.sh --datasets wiki-Talk helix-sdk helix-http
python3 scripts/render-results.py                 # regenerates RESULTS.md
```

Notes for this host:

- `compose.yaml` limits every service to `BENCHMARK_CPU_LIMIT` (default 8)
  and `BENCHMARK_MEMORY_LIMIT_BYTES` (default 6 GiB). Export
  `BENCHMARK_CPU_LIMIT=4` here; the harness and the server share the four
  cores, and that is recorded in the run (`host_loadavg_1m_*`).
- Neo4j needs the `external` compose profile (the script passes it) and
  `NEO4J_HEAP`/`NEO4J_PAGECACHE` default 3G each inside the 6 GiB cap.
- FalkorDB: `FALKOR_RESULTSET_SIZE=-1` selects the tuned profile; the default
  image truncates every result at 10,000 rows silently and fails the
  `wrong_answer` gate on wiki-Talk (recorded in ADVERSARIAL-GRAPH.md §7).
- Surreal backends are capped at 10k edges by the script (adapter load is
  O(E²): `DELETE … WHERE in= AND out=; RELATE` without an `(in,out)` index).
- Results persist after every scenario (`reports/<run>/results.jsonl`, atomic
  `report.json`), so a crash loses nothing completed.
- `server_cpu`/`server_memory` come from the Docker Engine API; `src/probe.rs`
  uses `$DOCKER_SOCK`, else `~/.docker/run/docker.sock` (macOS), else
  `/var/run/docker.sock`. Check the first LOAD row's `server_cpu_us` is
  non-null; the socket needs the `docker` group (re-login after bootstrap).

## What was in flight on the laptop when this was written

- Backends added in this commit, not yet measured anywhere: `ladybug`,
  `helix-http`, `helix-sdk`, `surreal-http`, `neo4j-http`. The laptop smoke
  of `ladybug,helix-sdk,helix-http` on wiki-Talk was still in the Ladybug
  LOAD step after 15 min (the adapter loads through per-node/per-edge
  statements); treat Ladybug load throughput as a finding to record, not a
  bug to hide.
- Every other backend has laptop smoke rows in `RESULTS.md`; the eight-way
  comparison in ADVERSARIAL-GRAPH.md §7 is the contended-host baseline the
  clean runs here should replace, keeping the laptop rows in the run
  bundles.
- Open: ADVERSARIAL-GRAPH.md §7 needs the transport-pair table
  (http vs sdk for Surreal, Helix, Neo4j) once the runs exist.
