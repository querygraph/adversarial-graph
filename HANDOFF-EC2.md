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

Took 37 min on the loaded laptop, 25 min here (2026-09-05). Three facts to know:

- The bootstrap's rustup step never ran: Debian 13 ships a `rustup` proxy in
  `/usr/bin` with a stable toolchain, so `command -v cargo` succeeded and the
  script's final `~/.cargo/bin/cargo` check failed. `cargo` 1.98 from
  `/usr/bin` builds the tree; `source ~/.cargo/env` is not needed.
- `docker` group membership only applies to a fresh login. Until then wrap
  docker and the ladder in `sg docker -c '…'`.

- `grust-helix` and `grust-ladybug` are `publish = false` in Grust, so they
  are `git` dependencies on the `v0.13.0` tag, with `[patch.crates-io]`
  pointing `grust-core` at the same tag so registry and tagged crates share
  one `GraphStore` (see `Cargo.toml`). Never point them at a local checkout.
- `lbug` (Ladybug engine) downloads a prebuilt static library into
  `~/.cache/lbug-prebuilt` when one exists for the target; otherwise it builds
  the C++ engine from source (long). x86_64-linux has a prebuilt. `liblbug.a`
  bundles zstd and simsimd objects that the Lance crates also link, and lld
  on Linux rejects the duplicates that ld64 tolerated on the laptop, so
  `build.rs` passes `--allow-multiple-definition` for the `ag` binary when
  the `ladybug` feature is on.

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
- The `helix` image was pinned to an arm64 manifest digest from the laptop;
  on x86 the container died with `exec /bin/tini: exec format error` and
  `run-ladder.sh` waited forever in `wait_ready`. `compose.yaml` now pins the
  multi-arch index digest (same build, both platforms).
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

## State after the first EC2 pass (2026-09-05, evening)

- Run bundles are git-ignored (`reports/*/`); this host's live in
  `~/src/adversarial-graph/reports/20260905T08…` onwards, the laptop's in the
  rsync'd bundles before them. `RESULTS.md` is the committed record.
- Every backend has clean-host rows in `RESULTS.md` (`x86_64/4`); the
  findings are written up in ADVERSARIAL-GRAPH.md §7 ("Clean-host results").
- Ladybug: wiki-Talk 200k took 5.1 h to load (≈11 edges/s, 6.3 GB peak RSS
  through the engine's default buffer pool, which `grust-ladybug` does not
  let the caller size). Run it detached (`setsid nohup …`): the Claude Code
  harness's memory watchdog killed the first attempt. The roadNet-CA slice was
  started last, `scripts/run-ladder.sh --datasets roadNet-CA ladybug`.
- Helix: the 200k load fails with HTTP 408 in both transports (two node scans
  per edge, no usable index, 30 s gateway timeout); the 10k slice loads at
  ≈33 edges/s and `helix-http` passes A1/A2/A4 there, while `helix-sdk`
  cannot read at all (`unknown variant \`Read\``). Next experiment: does a
  `NWhere` filter use the runtime `NodeEquality` index the harness now
  creates at bootstrap? Start `helix`, create the index, insert 10k nodes,
  time one `NWhere id = …` query with and without it.
- FalkorDB rows now carry `profile` (`resultset_size=-1` tuned,
  `resultset_size=10000` image default); both were rerun here.

## State on 2026-09-06 morning: publication done, what is next

- **Published**: `adversarial-site` publication `2026-09-06` (19 runs, 174
  cells, hard-gate total 9, manifest `1ebc157a…`, harness revision pinned
  in `scripts/verify-strain-evidence.mjs`; the Neo4j pair's outlier run is
  in the bundle and superseded per cell by its rerun), from harness `6b4b08c` with every
  Grust crate pinned to `querygraph/grust` `3840d152` (Memory snapshot reads
  and `traverse_ids`, Turso single-transaction load and resident snapshot,
  Ladybug Arrow bulk load, per-label tables, query-per-table traversal,
  buffer-pool cap and concurrent-writes option). Superseded pinned bundles
  are in `reports-dev/pinned-131308f`, `pinned-1baddcd`, `pinned-73f2e14`.
- **Next in the harness track**: the LSQB execution class for durable
  stores with a resident index (`docs/notes/task-turso-resident-index.md`):
  `TursoGraphStore::indexed_snapshot` exists on grust main; the harness
  class, plan registry, Python validator, site verifier `allowedClasses`,
  differential validation and `run-grust.sh` resume mode remain. Builds of
  `benchmarks/lsqb` are heavy; do them when no ladder is running.
- **Ladybug crate PR**: branch `prebuilt-cache-and-symbol-localization`
  in `~/src/ladybug-rust` (commit `2114886`), verified here; opened as
  https://github.com/LadybugDB/ladybug-rust/pull/33 on 2026-09-06.
- **Helix SDK**: cannot open against the `enterprise-dev` image under the
  pinned adapter (v3 client); measuring it needs the source-built server
  the LSQB harness qualifies it against.

## State on 2026-09-06 afternoon: LSQB resident index, PostgreSQL, resume mode

Everything from `docs/notes/task-turso-resident-index.md` is on `grust`
main except the SF0.3 measurement and the full SF0.1 matrix:

- **Class and stores** (grust `97ac532`, `b2a9730`): `backend-resident-index-rust-count`
  in the harness, plan registry, `validate-matrix-publication.py` and the
  site verifier (`allowedClasses` for `turso` and `postgres`), each with
  tests. `TursoGraphStore::indexed_snapshot` and
  `PostgresGraphStore::indexed_snapshot` build the write-invalidated
  `TypedGraphIndex` under the connection gate; the LSQB worker builds it
  before READY (Turso inside its per-observation reload, PostgreSQL from a
  read-back of the once-loaded service at attach). PostgreSQL's 22
  example-scale cases were executed here against a live 18.6 container:
  every count matched the oracle, 18 under the resident class, about 100 ms
  of setup per observation at that scale.
- **Resume mode** (grust `119202d`, site `5ca467a`):
  `RESUME_FROM=<prior OUTPUT_DIR>` copies every prior cell that verifies
  (receipt hashes, revision, images, timeout, valid cell, clean watchdog)
  and executes the rest; the receipt's `reused_cells` names each with the
  prior directory, receipt digest and Compose project, and both validators
  check the copied watchdog records against it. `resume-cells.sh` has its own
  test (`test-resume-cells.sh`).
- **SF0.1 diagnostic on this host** (`DISCOVERY`-style, in-process, not
  publishable; `docs/GRUST_SPEED_PROGRESS.md` "Resident index at SF0.1"):
  Turso baseline, 432,235 nodes / 2,080,404 edges. Worker setup ≈71 s per
  observation (≈68 s chunked load, the rest read-back and index build).
  Resident-index queries answer within a few percent of the Memory cell
  (6–200 ms); the two queries the dialect still routes to Turso's own
  scalar SQL count take 14.7 s (q4) and 260 s (q1) against 166 ms and
  70 ms on Memory. Whether scalar SQL should keep priority over a proven
  resident plan is now a measured question, not a design assumption; the
  route order was not changed. Never build while a timing run is on: a
  `nice -n 19 cargo build` raised setup to 180 s and timed q1 out. Launch
  long runs with `systemd-run --user` (the session watchdog killed
  `setsid nohup` wrappers mid-run).
- **Site registry verification** (site `d8abe1d`): the graph verifier
  now accepts the manifest's `execution_plans` registry and schema-3
  observation `plan` fields, and admits a not-materialized row shape or a
  Turso/PostgreSQL native aggregate only through a matching registry entry
  (mirror of `validate-matrix-publication.py`, with tests). Before this,
  every bundle produced by the current harness was rejected by the site for
  its unexpected manifest and observation fields.
- **Follow-ups from the laptop's first SF0.1 cohort, both done** (grust
  `5c34fc2`, `9d06b2c`; site `5ca467a`/lifecycle commit): the proven
  `count-factorized` plan now comes before the store's scalar SQL count
  (q1 65.9 ms and q4 176 ms over the index instead of 260 s and 14.7 s
  through Turso's SQL; all 22 cases register as resident entries for Turso
  and PostgreSQL), and Turso workers copy the coordinator's prebuilt store
  file (`per-observation-worker-copy`: 4.7 s setup per observation instead
  of 71 s, the nine-query cell in 2 min 9 s). What remains is the laptop's
  rerun of the Turso and PostgreSQL cells with `RESUME_FROM` against
  `benchmarks/lsqb/out/matrix-sf0.1-w2r10-68d1b09-f1`, or a fresh full run.
- **Not done**: SF0.3 (not measured on this host). The ladybug-rust PR is open:
  https://github.com/LadybugDB/ladybug-rust/pull/33 (fork
  `querygraph/ladybug-rust`, branch `prebuilt-cache-and-symbol-localization`;
  `gh` is installed and logged in as alexy on this host).

## Grust store speed work (2026-09-05, evening)

- Branch `fable/strain-adapter-reads` in `~/src/grust`, pushed at
  `fdc685ee` (memory snapshot reads, `traverse_ids`, Turso single-transaction
  load; ADVERSARIAL-GRAPH.md §7.2). The harness pins every Grust crate to
  that revision (`Cargo.toml` git `rev` plus `[patch.crates-io]`).
- The pinned ladder for the 2026-09-06 publication runs from
  `final-ladders.sh` (session scratch dir): every backend one at a time,
  both Falkor profiles, Helix at 10k and 200k, then Ladybug on both datasets
  (about 10 h). Then regenerate RESULTS.md and bundle:
  `scripts/bundle-site-evidence.py OUT --host "lakecat, 4 vCPU EC2, load ≈1" --since 20260905T082424Z`
  (`--since` keeps the laptop bundles out of the EC2 publication). The site
  verifier (`adversarial-site/scripts/verify-strain-evidence.mjs`) pins one
  manifest digest and harness revision per publication: add an entry, do not
  edit the laptop one.
- Reports now carry `harness_revision` and `grust_source` (build.rs stamps
  `git rev-parse` and the Cargo.lock source of `grust-core`); a backend that
  cannot be opened is a failing LOAD row, never a missing one.
- `reports-dev/` (ignored) holds every bundle that is not publishable:
  development builds against the local checkout, and `pinned-131308f/`,
  a partial pinned run superseded by the rerun at `1baddcd`.
- The key was authorized on 2026-09-05 evening; both repos' remotes are SSH.
  The site is cloned at `~/src/adversarial-site`, node 20 is installed; the
  strain page carries the drafted 2026-09-06 publication with placeholders
  that `publish-site.py` (session scratch dir; steps in ADVERSARIAL-GRAPH.md
  §7.2 and this file) fills from the bundle.

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
