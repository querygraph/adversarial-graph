# Fable to Fable: handoff from the laptop session to the EC2 session

Written 2026-09-05 16:55 UTC by the Claude session on the laptop, for the
Claude session on `lakecat` (the EC2 host). Everything below is committed;
what is pushed and where is in §5. Nothing here is benchmark evidence.

## 1. What the Codex session (Astra) did to the Grust read executor

Between roughly 02:00 and 08:00 PDT on 2026-09-05, Codex implemented the
executor work proposed in `GRUST-FAST.md` in `~/src/grust`, as one large
uncommitted worktree. Its own ledger is `docs/GRUST_SPEED_PROGRESS.md` with
four linked sub-ledgers (`GRUST_SPEED_CANDIDATES.md`, `GRUST_SPEED_SINGLE_PASS.md`,
`GRUST_SPEED_ADJACENCY.md`, `INDEXED_READS.md`). The pieces, in dependency order:

1. **Typed adjacency snapshot** (`grust-core/src/typed_graph_index.rs`,
   `grust-memory/src/indexed_snapshot.rs`). Immutable per-relationship-type
   incoming and outgoing adjacency over vertex slots, dense offsets for
   populated types and sparse source lists for rare ones, so auxiliary storage
   is O(V + E) rather than O(V × types). `MemoryGraphStore` caches the
   snapshot across reads and invalidates it on writes; the serialized graph
   size is cached exactly so the bounded API does not re-serialize per query.
2. **A separate indexed read entrypoint** (`grust-cypher/src/read/indexed.rs`,
   `read_policy/`). Same `ReadQueryPolicy` gates, same cumulative work, byte
   and deadline charges. Load and index construction stay outside the query
   budget and inside the benchmark's load interval.
3. **Exact non-materializing count plans** (`grust-cypher/src/read/count_*.rs`):
   factorized counts for proven pattern forests (chains, shared-variable
   stars, disconnected products); undirected two-hop wedges with unequal
   outer nodes; independent optional leaves with null padding and bag
   multiplicity through a restricted `WITH`; weighted tag intersections and
   tag/wedge anti-joins by witness existence; degree-oriented weighted
   support triangles with rank-space strict-suffix intersections (each
   distinct triangle visited once); directed four-cycles with adaptive
   merge/probe intersections; symmetric location triangles with sparse path
   weights; scalar node/edge scans, zero-hop identity paths, bounded range
   counts, null and constant probes, scalar unions. Every plan carries a
   structural proof; anything unproven falls back to the existing
   clause-by-clause executor, which remains the oracle.
4. **Reference executor fixes** found along the way: relationship uniqueness
   now uses physical edge slots (comma paths, mixed fixed/variable paths,
   `WITH` aliases); borrowed scalar property comparisons avoid copying JSON.
5. **SQL scalar count pushdown** for Turso and PostgreSQL under exact
   string-equality guards, and Surreal endpoint predicate pushdown.
6. **Harness side** (`benchmarks/lsqb`): a hash-bound plan registry, the row
   admission gate exempting only admitted non-materializing plans, a
   `plan` field on every observation, host preflight receipts, hardened
   watchdog cancellation, and a load-once Memory profiler.

What it verified: all 22 LSQB shapes (nine queries, thirteen count attacks)
match the oracle at `sfexample`, SF0.1 and SF0.3. Its native load-once
diagnostic runs all 22 at SF0.1 in 6.25 s including load and index build, and
at SF0.3 in 17.7 s. Those are diagnostics under host contention, not cohorts.
It never got a qualified Docker cohort because its own host preflight never
passed on the laptop (embedding workers and orphaned test loops), and it hit
its usage limit with the whole worktree uncommitted.

## 2. How the laptop session picked it up

- Ran the full gate on the worktree: 1,047 engine tests, 118 runner tests,
  strict clippy, fmt, all green.
- Committed it in two logical commits, engine (`3100db1`) and harness
  (`d0f57df`), preserving Codex's changelog bullets and ledgers.
- Made the preflight's aggregate-CPU limit an explicit, recorded parameter
  (`86668cb`): default two cores, at most four, written into
  `host-preflight.json` and re-validated by the publication validator. The
  busy-process rule (no process at or above one core) is unchanged. The
  laptop idles at 180–340 percent from system daemons alone, which is why the
  hardcoded limit never passed.
- Launched the qualified SF0.1 publication run (`run-grust.sh`, W2/R10,
  60 s deadline, 8 CPU / 6 GiB, the protocol every published cohort uses)
  once the embedding workers stopped. It is running as this is written; the
  output lands under `benchmarks/lsqb/out/matrix-sf0.1-w2r10-86668cb-a22`.
  SF0.3 follows the same way.
- Loose end: the changelog's `0.13.2 "Krill"` was never published;
  crates.io has `grust-cypher` 0.13.0 and `grust-graph` 0.13.1. The speed
  work will need a release and `cargo publish` per `AGENTS.md`.

## 3. State of the strain benchmark (this repo)

- Thirteen backends including the transport pairs (`surreal-http`/`-sdk`,
  `helix-http`/`-sdk`, `neo4j`/`neo4j-http`) and embedded `ladybug`; the
  five new ones have no measurements yet. `HANDOFF-EC2.md` has the build and
  run instructions for this host; `scripts/run-ladder.sh` runs one system at
  a time.
- The laptop numbers (contended, load average 300–950) are published as the
  baseline strain ledger at adversari.al/graph/strain, with their own
  evidence root (`public/evidence/strain/2026-09-05`) and verifier
  (`scripts/verify-strain-evidence.mjs`) in `~/src/adversarial-site`. The
  verifier pins the manifest digest and harness revision, so a new
  publication from this host is a new dated directory, produced by
  `scripts/bundle-site-evidence.py`.
- A reviewed comparison of the two benchmarks is `~/src/grust/BENCHMARK-REVIEW.md`.

## 4. Where the strain benchmark meets the new executor

The strain scenarios mostly do not go through Cypher, so the LSQB speed-up
does not move them by itself:

| Scenario | Path in Grust | What a speed-up would touch |
|---|---|---|
| A1 fan-out, A2 deep paths | `GraphStore::traverse` / `get_edges` | store adapters: Memory's `BTreeMap` adjacency could share the typed snapshot; Turso/Postgres `get_edges` by indexed endpoint; Falkor adapter reads (currently `Unsupported`) |
| A3 policy bounds | `run_bounded_read_query` on Memory | the indexed bounded entrypoint; A3 is the one strain cell that measures the executor |
| A4 hot node, A7 replay | `put_edge` / `GraphCommitStore` | adapter write batching and the Turso MVCC commit path |
| LOAD | `put_graph` | Surreal adapter's O(E²) load; Helix/Ladybug batch sizes |

If the goal is to make the strain numbers move, the work is in the adapters
and the Memory store's adjacency, not in `grust-cypher`, and every change
shows up as a new dated strain publication with the same probes (client
rusage, container CPU and memory, host load).

## 5. Repositories and what is pushed

| Repo | Remote | State |
|---|---|---|
| `~/src/grust` | `github.com/querygraph/grust` main | pushed through the preflight and docs commits; Codex's worktree is in `3100db1` and `d0f57df` |
| `~/src/adversarial-graph` (this repo) | `github.com/querygraph/adversarial-graph` | remote created 2026-09-05 by the user; laptop `main` pushed; this host's checkout diverged from `d9e0afb` and needs `git fetch origin` then a merge, not a reset |
| `~/src/adversarial-site` | `github.com/querygraph/adversarial-site` master | pushed, including the strain ledger |

On this host, after `git fetch origin`: laptop `main` adds the `.pypi`
ignore, the transport-aware `RESULTS.md`, `scripts/bundle-site-evidence.py`
and this file. Merge with `git merge origin/main`; conflicts, if any, will be
in `RESULTS.md` (regenerate it) and `reports/`. The `.pypi` token file exists
only on the laptop and must never be committed.

## 6. Next task (added 2026-09-06)

See `docs/notes/task-turso-resident-index.md`: a resident typed index for the
durable Grust stores, the harness execution class it needs, and a resume mode
for `run-grust.sh`. The laptop is running the SF0.1 matrix on `af2efa8` and
cannot touch grust until it ends.

## 7. EC2 to laptop: what landed on 2026-09-06 and what the laptop runs next

Written 2026-09-06 18:40 UTC by the EC2 session. Everything below is pushed;
pull all three repos before doing anything: `~/src/grust` main at `7429fc7`,
`~/src/adversarial-graph` main, `~/src/adversarial-site` master at `192133f`.

### What is on grust main (in order)

| Commit | What |
|---|---|
| `97ac532`, `b2a9730` | `backend-resident-index-rust-count`: the harness class, plan registry, Python validator and site `allowedClasses` for Turso and PostgreSQL; `TursoGraphStore::indexed_snapshot` and `PostgresGraphStore::indexed_snapshot` (write-invalidated, built under the connection gate). |
| `119202d` | `RESUME_FROM=<prior OUTPUT_DIR>` for `run-grust.sh`; the receipt's `reused_cells` list; `resume-cells.sh` with `test-resume-cells.sh`. |
| `dfb60dc` | `resident_index_built` telemetry (nodes, edges, serialized bytes, build ms) in the coordinator's cell log. |
| `5c34fc2` | Follow-up 1: the proven `count-factorized` plan comes before the store's scalar SQL count. All 22 pinned cases register as resident entries for Turso and PostgreSQL; `sql-count` stays, under its own class, for a count the proof does not admit. |
| `9d06b2c` | Follow-up 2: Turso workers copy the coordinator's prebuilt store file (`per-observation-worker-copy`) instead of reloading the CSVs. Validator, merge script and fixtures expect it. |
| `7429fc7` | One `host_cpu_steal` progress record per cell in the run log (steal ms, wall ms). |

Site: `d8abe1d` validates the manifest's `execution_plans` registry and
observation `plan` fields (before it, the site rejected every bundle the
current harness produces), `5ca467a` accepts `reused_cells`, `192133f`
admits either process-owned Turso lifecycle so the 68d1b09 receipt still
verifies. All 208 site tests pass.

### What was measured here (diagnostic, not publishable; ledger section
"Resident index at SF0.1")

Turso baseline cell, SF0.1, this host quiet (zero steal):

| | Before | After 5c34fc2 + 9d06b2c |
|---|---:|---:|
| q1 | 260 s (Turso SQL) | 65.9 ms (resident index) |
| q4 | 14.7 s (Turso SQL) | 176 ms |
| q2, q5, q7, q8 | 143–202 ms | 146–198 ms (Memory: 150–186 ms) |
| worker setup per observation | 71–73 s (67 s CSV reload) | 4.7 s (0.24 s copy of a 553 MB file, 4.4 s read-back and index) |
| nine-query cell, one iteration | ≈ 15 min | 2 min 9 s |

Every count matched the oracle in every run; no store file is left behind.

### What the laptop does now

1. Pull grust to `7429fc7` or later and the site to `192133f` or later.
2. Run a **fresh full SF0.1 matrix**. Not a resume: resume reuses a cell only
   at the same source revision, by design, so
   `benchmarks/lsqb/out/matrix-sf0.1-w2r10-68d1b09-f1` cannot seed a run at
   the new revision. Keep that directory as the pre-change baseline.
   ```
   CELL_TIMEOUT_MS=3600000 SF=0.1 RUNS=10 WARMUPS=2 \
     OUTPUT_DIR=benchmarks/lsqb/out/matrix-sf0.1-w2r10-<rev>-f1 benchmarks/lsqb/run-grust.sh
   ```
   The Turso cells now take minutes, and q1 no longer sits at a timeout, so
   the run is bounded by the PostgreSQL cells (attach plus a read-back over
   the wire per observation; not changed here).
3. If one cell fails, rerun into a fresh directory at the same revision with
   `RESUME_FROM=<that OUTPUT_DIR>`; only the failed cell executes, and the
   receipt lists the reused ones.
4. Publish through the site; the ledger gets a new dated publication.
5. Before citing numbers, read each cell's `host_cpu_steal` line in its run
   log. On this host (a burstable t2.xlarge) a repeat that started after
   2.5 h of continuous compute ran 2× slower with 8.2 h of accumulated
   steal; the load average never showed it. The laptop is not burstable,
   but the record is free.

### Also

- `LadybugDB/ladybug-rust` PR #33 is open from the `querygraph` fork
  (prebuilt cache out of the crate source tree; opt-in localization of the
  bundled C symbols), with `LADYBUG-NOTES.md` linked.
- The strain harness records `host_steal_us` per scenario row (`00fdca6`);
  RESULTS.md shows it in the Host column.
- Not done: SF0.3 on this host; per-observation worker CPU time in the LSQB
  observation record (an observation-schema change across validator, merge
  script and site).

### Added 2026-09-06 20:30 UTC

The strain ladder was rerun on this host in Unlimited credit mode with the
new `host_steal_us` column and published as `2026-09-06-unlimited` (site
verifier trust entry added; harness `b4659ad`, 17 runs, 174 cells, gates 9,
53 minutes). Every finding reproduces. ADVERSARIAL-GRAPH.md §7.3 has the
steal story. For the laptop's LSQB run nothing changes: the `host_cpu_steal`
line lands in each cell's run log at grust `7429fc7` or later.

## 8. Laptop to EC2: the fresh SF0.1 matrix at `7429fc7` (2026-09-06, 19:04–20:27 UTC)

Ran exactly as §7 asked, after pulling grust to `7429fc7` and the site to
`192133f` and re-running the runner, evidence, resume and publication suites.

- Receipt `eb05aa31…`, verified; 83 minutes end to end; host CPU steal zero
  on every cell. Memory, Turso and PostgreSQL pass all 22 cases with exact
  counts, Turso and PostgreSQL entirely on the resident index: q1 50 ms on
  both, q4 128 / 130 ms, q6 4 / 5 ms, q9 12 / 12 ms, a1 51 / 51 ms. Turso's
  cells took 8.4 and 12.4 minutes against 121 and 174 before. FalkorDB still
  terminates at q9 and a7 in warm-up 1, so `all_required_outcomes_valid`
  stays false, disclosed.
- Ledger: `docs/GRUST_SPEED_PROGRESS.md`, "Qualified SF0.1 cohort after the
  route and reload changes". Site: published as the second bundle of
  2026-09-06 under `grust/sf0.1-7429fc7` (the verifier now binds a revision
  suffix in a bundle path to the receipt's source revision, so two receipts
  at one scale on one date never share a location); the `68d1b09` bundle
  stays as the pre-change baseline.
- Open on the LSQB side: SF0.3 on a host with 8 CPUs and the memory for a
  6.2-million-edge resident index; per-observation worker CPU time in the
  observation record; FalkorDB q9/a7 within a 60-second deadline is a
  FalkorDB finding, not a harness one.

## 9. The plan to complete the adversarial graph benchmarks (laptop, 2026-09-06 21:10 UTC)

The laptop session owns completion of both ledgers from here. This section
is the whole remaining set, who runs what, and in what order. Neutral
framing throughout; every result is a dated publication with its own
receipt or manifest, and nothing already published is edited.

### 9.1 What is done

- Strain ledger: families A1, A2, A3, A4, A7 on all thirteen backends, on
  wiki-Talk and roadNet-CA, at 200,000-edge slices (10,000 for Surreal and
  Helix), on the dedicated host with steal recorded (`2026-09-06`,
  `2026-09-06-unlimited`); the contended laptop baseline (`2026-09-05`).
- LSQB ledger: example-scale matrix, native Neo4j and Sail at SF0.1, and the
  first two receipt-bound Grust matrices at SF0.1 (`68d1b09`, `7429fc7`).

### 9.2 What remains, and where it runs

| Item | Host | Why that host |
|---|---|---|
| Full-graph tiers of A1/A2/A4/A7 for the stores that load fast enough: wiki-Talk, roadNet-CA, web-Google (about 5 million edges each), cit-Patents (16.5 M), soc-LiveJournal1 (69 M), com-Orkut (117 M) | laptop | the in-process oracle and Memory need tens of GB for the M tier; the laptop has 64 GB, the EC2 host 15 |
| LSQB SF0.3 matrix (resident index of 6.2 M edges must fit the 6 GiB component cap; measure, do not assume) and a clean native Neo4j SF0.3 rerun | laptop | 8-CPU / 6 GiB envelope of the published cohorts |
| Site admission of every new bundle, both ledgers | laptop | the receipts land there |
| A5 recursive deletes, A6 isolation under mixed load, A8 differential Cypher, A12 cold start and footprint: loaders, scenarios, gates, then clean-host slices on all backends | EC2 | scenario development, then dedicated-host measurement at slice scale |
| Memgraph and Apache AGE adapters; the Helix SDK fix; the Ladybug PR follow-through | EC2 | adapter work, small-host measurement |
| A9, A10, A11 stack-integrity families (Grust memory layer, LakeCat outbox, QueryGraph proof bases) | EC2, after A5/A6/A8/A12 | need the sibling stack repos built there |
| The L tier (twitter-2010, Friendster, GAP-road) | deferred | 1.5 to 1.8 billion edges; revisit after the M tier says what the oracle costs |

### 9.3 EC2 assignments, in order

1. **Loaders** (`src/dataset.rs`): LDBC SNB CsvBasic (`ldbc-snb-sf0.1`,
   `ldbc-snb-sf1`, typed labels and properties; reuse the projected-FK
   knowledge from `benchmarks/lsqb/src/dataset.rs` in grust) and ICIJ
   Offshore Leaks (`icij-offshore-leaks`, the CSV zip). Both through the
   same `Graph` the SNAP loader produces, with an oracle that knows labels.
2. **A8 differential Cypher** (`scenarios/v1/A8.json`, gate `wrong_answer`):
   a fixed set of read queries (the nine LSQB shapes, the thirteen count
   attacks, plus row-returning variants with `ORDER BY` and `LIMIT`) run
   through `grust-cypher`'s reference executor on the in-process graph as
   the oracle, and through every backend that accepts Cypher: Grust
   pushdown on Turso and PostgreSQL, native openCypher on FalkorDB, Neo4j
   (Bolt and HTTP) and Memgraph, the resident-index plans where proven.
   Compare full result sets, not counts. Record the execution class per
   backend as the LSQB harness does.
3. **A6 isolation under mixed load** (gate `isolation_anomaly`,
   `ldbc-snb-sf0.1`): Elle-style histories. Register workload: N clients
   read-then-write a property on a shared set of vertices with
   client-supplied versions; list-append workload: clients append their id
   to a list property. Checker: from the recorded history, detect lost
   updates, dirty reads, write cycles (build the ww/wr/rw dependency graph
   for list-append; a cycle is an anomaly). Report the anomaly class per
   backend; a store that refuses concurrent writes with a typed conflict
   passes, one that silently loses an append fails.
4. **A5 recursive deletes** (gates `wrong_answer`, `lost_write`,
   `ldbc-snb-sf1` and `sx-stackoverflow`): delete a root (a Post with its
   reply tree; a StackOverflow question with its answer and comment chain)
   through each store's delete path, then read back: the oracle is the
   in-process graph after the same delete. Concurrent readers during the
   delete record what they saw.
5. **A12 cold start and footprint** (`cold_start_ms`, `peak_rss_bytes`,
   `p999_us`): time from process or container start to first correct
   answer; peak RSS from the probes that already exist; an open-loop
   scheduler at a fixed arrival rate (start with 50 and 200 requests per
   second of A1 one-hop reads) with HdrHistogram p99.9 of service time,
   not response time, and both reported.
6. **Adapters**: `memgraph` through the Bolt store already in `src/neo4j.rs`
   (the compose service exists on 17688; Memgraph speaks Bolt); `age`
   through a PostgreSQL image with the Apache AGE extension and a
   harness-side Cypher store over `cypher()`; fix `helix-sdk` (the adapter
   sends `Read` where the pinned SDK expects `read`; either pin the SDK
   the server speaks or fix the casing in `grust-helix`, then re-measure
   both Helix transports).
7. **Publish** each family as it lands: clean-host slices on every backend,
   a dated strain publication, `RESULTS.md`, a §7 note in
   `ADVERSARIAL-GRAPH.md`, and a numbered section here. The laptop admits
   it on the site and runs the full tiers.
8. **A9 to A11** after that, with a design note first (what "pass" is for
   each gate, which sibling crates are needed, which synthetic datasets).

### 9.3a A third host: `grust` (m-class 2xlarge, 8 vCPU, 31 GiB, 849 GB)

Added 2026-09-06 22:10 UTC. It is crawling Hacker News for six days, at a
tenth of one core and a load average of 0.1, which makes it the quietest
host we have. The laptop session drives it over ssh (`grust`); both repos
are cloned under `~/src`, the S and M datasets are synced, the harness is
built there. Its role, in order:

1. Full-graph tiers of the slow-loading strain backends (LanceDB, Ladybug
   with the Arrow bulk load, Surreal and Helix at a 200,000-edge slice),
   in parallel with the laptop's fast stores, published as its own dated
   strain bundle with host, load and steal on every row.
2. Correctness runs of A5, A6, A8 and A12 across all backends as the EC2
   session lands them, so the EC2 box keeps developing.
3. If its load stays this low, the LSQB SF0.3 matrix and the native Neo4j
   SF0.3 rerun move there from the laptop: it meets the 8-CPU envelope
   natively and is not burstable. Decided after the first strain tiers
   show its steady load.

### 9.4 Laptop queue

1. Rebuild the harness at this revision; run the full-graph tiers of the
   M1 families in the order above, one system at a time, stopping a
   backend at the first tier whose load exceeds two hours (the note says
   which backend stopped where and why).
2. LSQB SF0.3: a full matrix at the current grust revision; native Neo4j
   SF0.3 rerun with a clean host screen. Publish both.
3. Site admission for whatever EC2 publishes; keep the strain verifier's
   trust table and the graph verifier's contracts current.

### 9.5 Rules that stay

- No grust edits on the laptop while an LSQB matrix runs there.
- Pull all three repos before touching any; write a numbered section here
  on every handoff.
- Neutral framing; superseded failures stay visible; unsupported is never a
  pass; every number carries its host, slice, class and steal.

## 10. Laptop to EC2: Ladybug maintainer reply, adapter follow-ups (2026-09-06 22:45 UTC)

Arun replied to the note and PR #33: bulk loading is the recommended path;
point-lookup planning is what 0.20.2's thread-local prepared-statement cache
avoids; `ladybug` PR #925 (merged 2026-09-06) fixes a shadow FileHandle
leak across checkpoints that hurts long bulk loads under a size cap. The
adapter items that follow are in `docs/notes/ladybug-findings.md`, last
section: keep prepared statements alive per query text in `grust-ladybug`
and keep the store's calls on one thread; confirm `put_graph` uses the Arrow
bulk path in the strain harness; re-measure on the release with #925 and
record the lbug version per row. These sit after §9.3 item 6 in your queue.

## 11. Taskmaster: rebalancing to shorten the critical path (2026-09-06 23:40 UTC)

The critical path is scenario development, not compute. Two changes:

1. **The laptop takes the Memgraph and Apache AGE adapters and the A12
   open-loop scheduler** (§9.3 items 5 and 6, minus the Helix SDK fix).
   They are harness code, built into a separate target directory so the
   running full-tier ladder keeps its binary. lakecat keeps A8, A6 and A5,
   in that order, and the Helix SDK fix in `grust-helix`. When the laptop
   pushes `memgraph`, `age` and `a12_cold_start`, lakecat and the grust box
   add them to their runs like any other backend or family.
2. **The grust box caps the two largest datasets at one hour** for the slow
   stores: `--cap 3600` for soc-LiveJournal1 and com-Orkut (keep 7200 for
   the 5-million-edge graphs and cit-Patents). A store that cannot load 69
   million edges in an hour is the finding; two hours adds nothing.

Everything else in §9 stands.

## 12. Laptop to both hosts: `memgraph`, `age` and A12 landed (2026-09-07 00:30 UTC)

Commit `53aedf9` on `main`. Pull before touching anything.

**What it adds.**

1. `memgraph`: Memgraph 3.12 over Bolt through the Neo4j store
   (`BoltDialect::Memgraph` picks the `memgraph` session database and the
   `CREATE INDEX ON :V(id)` DDL). Compose service `memgraph`, external
   profile, port 17688. No new feature flag: it rides on `neo4j`.
2. `age`: Apache AGE 1.8 on PostgreSQL 18.6, `src/age.rs`, feature `age`
   (`tokio-postgres`; parameters travel as an `agtype` map in text format,
   results are cast to `text`). Compose service `age`, external profile,
   port 55434. Bootstrap creates the graph, the `V`/`E` labels, a GIN index
   on `V.properties` and B-tree indexes on `E.start_id`/`E.end_id`; the
   plan for `MATCH (a:V {id: $id})-[:E]->(b)` uses both. AGE rejects graph
   names shorter than three characters, so the graph is `adversarial`.
3. A12 is implemented and in `scenarios::all()`, so every ladder run picks
   it up: the cold-start hub degree on a fresh handle, then an open-loop
   stream of one-hop reads at 50 and 200 requests per second for 30 s
   each (10 s in smoke) over sixteen handles, with service-time and
   response-time histograms, late arrivals over 1 s, and wrong answers.
   It adds about 65 s per (backend, dataset) pair. A4's appended hub
   edges are carried into A12's expected degree through
   `Ctx::hub_writes`, since both families run against one load; the
   first Memgraph smoke caught exactly that (100 "wrong" hub answers).
4. `scripts/run-full-tiers.sh` and `scripts/run-ladder.sh` know the
   `memgraph` and `age` services. **grust box:** your local commit
   `b959bd9` adds the same script; `git pull --rebase` will conflict on
   it. Take the pushed version (`git checkout --theirs` under rebase
   means the upstream file) and re-apply your `--cap` choices on the
   command line, not in the script.

**Smoke, 200k wiki-Talk slice, laptop (contended, load 7–11).**

| Backend | Load | A1 | A4 | A12 cold start | A12 50 rps p50 / p99 / p99.9 (service, µs) | A12 200 rps p50 / p99 / p99.9 | Server CPU during A12 |
|---|---|---|---|---|---|---|---|
| memgraph | 40.1 s | pass | pass | 38 ms | 4,139 / 43,039 / 49,311 | 1,842 / 38,591 / 42,015 | 7.6 s |
| age | 45.5 s | pass | pass | 44 ms | 10,599 / 44,575 / 47,999 | 10,543 / 51,327 / 76,607 | 28.0 s |

No late arrivals, no errors, zero hard gates on either. A3 and A7 are
`unsupported` on both (policy bounds run through the reference executor;
neither implements `GraphCommitStore`), as for Neo4j. AGE's server CPU is
3.7× Memgraph's for the same stream: each `cypher()` call re-parses and
re-plans the Cypher text inside PostgreSQL (the harness sends an unnamed
prepared statement per query, like the Neo4j adapter). Whether a named
prepared statement per connection changes that is a fair follow-up
measurement, not a change I have made; the adapter measures the
documented access path first. Load rate on both is roughly 5–7k edges/s
through `UNWIND` batches of 5,000, so wiki-Talk is ~12 min, cit-Patents
~40 min, and soc-LiveJournal1 is beyond a one-hour cap: that is the
expected finding, not a launcher fault.

**Who runs what.** As §11 says: lakecat and the grust box add `memgraph`
and `age` to their clean-host slices and full tiers like any other
backend (`--features …,age` when you build; `docker compose --profile
external up -d age memgraph` is what the ladders do themselves). The
laptop's full-tier ladder is on `turso-mvcc roadNet-CA`; when it reaches
the end I run the two new backends here as well for the contended
baseline. lakecat keeps A8, A6, A5 and the Helix SDK fix.

## 13. Host memory is a host outcome, not a store finding (2026-09-07 04:30 UTC; corrected 04:50)

lakecat wedged around 03:20 UTC on 2026-09-07 (SSH and Tailscale both
unreachable; the user is rebooting it). The likely cause is memory: the
harness keeps the reference graph and its adjacency index in the client
process, and the embedded stores (`turso-wal`, `turso-mvcc`, `ladybug`,
`lancedb`, `memory`) add the store on top of that in the same process.
On the laptop the Turso MVCC load reached 20.7 GB resident on roadNet-CA
(and about the same on wiki-Talk, the other 5-million-edge graph) and
26 GB twelve minutes into cit-Patents; lakecat has 15 GiB. A run that
needs 20 GB is not a failing store on a 15 GiB host; it is a tier that
does not fit that host. And a run that survives at 12 GB on that host is
measured under page-cache and swap pressure the same store never sees
elsewhere, which is just as unfair. So the rule is placement, not
limits: no cell runs on a host where its resident set comes near the RAM.

**Rules from here.**

1. **Placement by resident set.** Embedded-store full tiers do not run
   on lakecat at all; the clean-host 200k slices in §7 are its embedded
   numbers. Network backends carry only the oracle in the client and a
   6 GiB container-bounded server, and fit lakecat through
   soc-LiveJournal1.

   | Host | RAM | Embedded stores | Network backends |
   |---|---|---|---|
   | lakecat | 15 GiB | 200k clean-host slices only (done) | every tier through soc-LiveJournal1 |
   | grust box | 31 GiB | full tiers through web-Google | as already queued |
   | laptop | 64 GB | cit-Patents and up; contended baseline | contended baseline |

   Network backends: `postgres`, `neo4j`, `neo4j-http`, `falkor`,
   `surreal-http`, `surreal-sdk`, `helix-http`, `helix-sdk`, `memgraph`,
   `age`. Embedded: `memory`, `turso-wal`, `turso-mvcc`, `ladybug`,
   `lancedb`.
2. **The guard is a safety net, never a scheduler.** `AG_RSS_LIMIT_GB`
   in `scripts/run-full-tiers.sh` (commit `b057a98`) kills any `ag run`
   whose resident set passes it and logs
   `## host.memory-exceeded: …`. A killed run writes no report bundle,
   and `RESULTS.md` and the site render only from bundles, so a guard
   kill can never appear as a store outcome; the log line only says the
   tier was misplaced and must run on a host it fits. Set it at the
   host's real capacity (lakecat 13, grust box 28, laptop 44), never
   lower to make room, and treat a fired guard as a placement error in
   rule 1 to correct.
3. When a cell is reported from a different host than the rest of a
   backend's ladder, the report's `host` field already says so
   (`arch/vCPUs`); the render keeps it in the Host column. Nothing else
   to mark.
4. **lakecat, after the reboot:** `dmesg -T | grep -i -E "oom|hung task"`
   first and paste what it says into your next section; then pull
   (`53aedf9` and later), read §12, and resume the ladder with the
   network backends only, `AG_RSS_LIMIT_GB=13` as the net. Reports on
   disk from finished cells are intact; only the in-flight cell is lost.
5. **grust box:** the embedded stores' full tiers through web-Google are
   yours (`memory`, `turso-wal`, `turso-mvcc`, `ladybug`, `lancedb`,
   `--datasets wiki-Talk,roadNet-CA,web-Google`, `AG_RSS_LIMIT_GB=28`),
   after the bracket you are on. The laptop has `memory` through
   com-Orkut and `turso-wal`/`turso-mvcc` through web-Google already;
   yours are the clean-host versions of those rows.

**Evidence from lakecat's previous boot (read by the laptop at 05:35 UTC,
`journalctl -k -b -1`).** The wedge was memory, and the offender was the
typed-dataset work, not a Turso tier: the kernel killed `ag` four times
between 22:06 and 23:10 UTC on 2026-09-06, each at about 12.0 GB anonymous
resident (`icij-full.service: Failed with result 'oom-kill'`, so the ICIJ
Offshore Leaks full graph with the in-process oracle), then a `rustc` at
23:55 (4 GB), and from then until the reboot at 05:14 the journal is only
`systemd-journald: Under memory pressure, flushing caches` with nothing
killed: the reclaim livelock that leaves a 15 GiB host with no swap
unreachable but alive. Rule 1 therefore also covers the typed datasets:
ICIJ full and LDBC SNB above SF0.1 do not run on lakecat; they run on
the grust box or the laptop, and lakecat keeps the proportional slices
its A8 commit already defines.

## 14. Division of work across the three hosts (2026-09-07 05:30 UTC)

lakecat is rebooting after the §13 wedge. From here, by host:

**lakecat (15 GiB, clean host).** Network backends at full tiers through
soc-LiveJournal1, in this order, with `AG_RSS_LIMIT_GB=13` as the net:

```
AG_RSS_LIMIT_GB=13 scripts/run-full-tiers.sh --cap 7200 \
  --datasets wiki-Talk,roadNet-CA,web-Google,cit-Patents,soc-LiveJournal1 \
  postgres neo4j neo4j-http memgraph age falkor
AG_RSS_LIMIT_GB=13 scripts/run-full-tiers.sh --cap 1800 \
  --datasets wiki-Talk surreal-http surreal-sdk helix-http helix-sdk
```

Surreal and Helix are last under a 30-minute cap because §7 already
shows they do not load a full tier (per-row statements, gateway
timeouts); one capped wiki-Talk cell each is the finding. Before the
ladder: the clean-host 200k slices for `memgraph` and `age`
(`scripts/run-ladder.sh memgraph age`) so the §7 table has them. Between
runs: A8, A6, A5 and the Helix SDK fix, as in §11. First thing after the
reboot: the `journalctl -k -b -1` check in §13 rule 4.

**grust box (31 GiB, crawler tenant).** Embedded stores at full tiers
through web-Google, clean-host readings of rows the laptop has only as
the contended baseline:

```
AG_RSS_LIMIT_GB=28 scripts/run-full-tiers.sh --cap 7200 \
  --datasets wiki-Talk,roadNet-CA,web-Google \
  memory turso-wal turso-mvcc ladybug lancedb
```

after the bracket you are on. No LSQB matrices unless told. Pull first:
`b959bd9` conflicts with the pushed ladder script (§12 item 4).

**laptop (64 GB, contended).** Finishes the running ladder (neo4j,
neo4j-http, falkor, lancedb, then memgraph and age) as the contended
baseline; the embedded stores at cit-Patents and above, which fit
nowhere else; then the LSQB SF0.3 matrix and the native Neo4j SF0.3
rerun; the dated strain publication; site admissions of every bundle the
other two hosts publish.

Laptop ladder so far (full tiers, contended): memory through com-Orkut;
turso-wal through soc-LiveJournal1; turso-mvcc through web-Google
(cit-Patents hit the 2 h cap at 26 GB resident); postgres through
cit-Patents (soc-LiveJournal1 loaded and passed A1, A2 hit the cap);
neo4j loaded the full wiki-Talk in under three minutes and passed A1,
A2, A4 and A12 with zero gates.

Rough wall-clock: lakecat a day to a day and a half, the grust box a
day, the laptop a day plus SF0.3; in parallel, about two days to close
the set.

## 15. lakecat after the reboot: what the kernel says, and the ladder relaunched (2026-09-07 05:45 UTC)

Pulled to `79d6a3b` and rebased the A8 commit on top (`e8f926d`); the
Neo4j "clear every node, whatever its label" change that was sitting
uncommitted here is in this commit too. Read §12–§14. Kernel log first,
as §13 rule 4 asks.

**What `journalctl -k -b -1` says.** Two different events, and the one
that took the host down is not the one the RSS guard is built for.

1. Four global OOM kills on the evening of the 6th, all of `ag` at about
   12 GB anonymous resident, every one of them the `memory` backend
   loading the full ICIJ offshore-leaks graph (the transient unit
   `icij-full.service` and three retries from the session shell):

   ```
   Sep 06 22:06:16 kernel: Out of memory: Killed process 290925 (ag) total-vm:12435800kB, anon-rss:12027488kB
   Sep 06 22:33:15 kernel: Out of memory: Killed process 291675 (ag) total-vm:12487408kB, anon-rss:12078688kB   (task_memcg=…/icij-full.service)
   Sep 06 22:57:34 kernel: Out of memory: Killed process 297813 (ag) total-vm:12423208kB, anon-rss:12100052kB
   Sep 06 23:09:59 kernel: Out of memory: Killed process 297987 (ag) total-vm:12403768kB, anon-rss:12079328kB
   Sep 06 23:55:44 kernel: Out of memory: Killed process 313710 (rustc) total-vm:5852404kB, anon-rss:3989396kB
   ```

   The process table at 22:06 shows `ag` at 11.5 GB, then `claude` at
   0.4 GB, and 2.7 GB of shmem (container tmpfs and Postgres shared
   buffers); nothing else above 50 MB. This is exactly the §13 case: an
   embedded store plus the oracle for a graph that does not fit 15 GiB.
   The fifth kill is a `rustc` at 4 GB, a build running next to a run,
   which the standing rule already forbids.

2. **The wedge.** No OOM kill at all. The last finished bundle is
   `reports-dev/20260907T015649Z` (falkor, A8 on icij-offshore-leaks,
   client peak 5.8 GB); the next bundle directory `20260907T015853Z` is
   empty. Neo4j's container started at 01:58:33 for the neo4j A8 ICIJ
   cell, and by 02:00:43 dockerd logged its health check timing out.
   From then until the reboot at 05:14 the journal is nothing but
   `systemd-journald: Under memory pressure, flushing caches`, networkd
   DHCP timeouts, `dbus` auth timeouts of 1,636 s, tailscaled reporting
   subscribers 2 h 22 min late, and sshd dropping the client at 03:26.
   The `ag` client for A8 on ICIJ needs about 6 GB (four cells in a row
   say 5.8–6.1 GB); Neo4j's container is 6 GiB (3 G heap + 3 G page
   cache); `grust-pg-dev`, a Postgres container outside `compose.yaml`
   with no memory limit, had been up since 07:54 on the 6th; plus
   `claude`, docker, tailscale. That sums to the host. With no swap the
   kernel spent three hours reclaiming file pages (the binaries) instead
   of finding a process to kill, because every reclaim scan still found
   a little to free ("all_unreclaimable? no"). Docker's own memcg limit
   on Neo4j never fired either: the container was under its cap; it was
   the host that was over.

**Consequence for the guard.** `AG_RSS_LIMIT_GB` alone would not have
caught either the wedge (client at 6 GB, under any sane limit) or
prevented the kills (they were the kernel doing the guard's job). So
the ladder script now takes a second, host-level floor:
`AG_MEM_AVAILABLE_MIN_GB` kills the `ag run` when `/proc/meminfo`
`MemAvailable` drops below it and logs the same
`## host.memory-exceeded:` line with the available figure, so the
outcome stays a host outcome. The guard loop polls every 5 s instead
of 15 (at 02:00 the host went from healthy to unreachable in about two
minutes). lakecat runs with `AG_RSS_LIMIT_GB=13 AG_MEM_AVAILABLE_MIN_GB=1`.
Nothing changes for the other two hosts unless they set the second
variable. `grust-pg-dev` stays stopped on lakecat during ladders; it is
not part of the benchmark and it is unbounded.

**Also fixed on this host.** `loginctl enable-linger admin` was off, so
a dropped SSH session could have taken the user manager and its
transient units with it; it is on now. And `compose.yaml` defaults to
`BENCHMARK_CPU_LIMIT=8`, which Docker refuses on this 4-vCPU host, so
the first launch died in its first `docker compose up`; the ladder runs
with `BENCHMARK_CPU_LIMIT=4` exported, which is what the earlier
lakecat sessions must have had in their shell. Worth a line in
`README.md` for anyone on a small host; not changed in the script,
because the limit is part of what makes the network numbers comparable
across hosts and the value belongs with the host, not the repo.

**What is running.** One transient unit, `lakecat-ladder`, in the §14
order: the clean-host 200k slices for `memgraph` and `age`
(`scripts/run-ladder.sh memgraph age`), then the network backends'
full tiers through soc-LiveJournal1 with `--cap 7200`, then
`surreal-http surreal-sdk helix-http helix-sdk` on wiki-Talk with
`--cap 1800`. `reports/` on lakecat holds only smoke cells so far (the
§7 slices), so every full-tier cell is new. A8, A6, A5 and the Helix
SDK fix resume between stages; the lost cell is neo4j A8 on ICIJ, and
per the placement rule it reruns only with Neo4j alone on the host,
which it now will be.
