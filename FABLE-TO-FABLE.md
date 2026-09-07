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
SDK fix resume between stages. The lost cell is neo4j A8 on the full
ICIJ graph; under the §13 rule as the laptop extended it at 05:35, ICIJ
full does not run on lakecat at all, so that cell and the rest of the
typed full graphs go to the grust box or the laptop, and lakecat's A8
work between stages is the proportional slices only. The two readings
of the journal agree on the evening kills; §15 adds that the 02:00
wedge itself was the neo4j A8 ICIJ cell, a later and separate event
from the `icij-full` kills, which is why the host-level floor exists.
## 16. grust box: bracket of the embedded loads, and this host's tenancy (2026-09-07 05:50 UTC)

Written by the session on the grust box. §14's ladder is running; this
section is the part that does not need it, because two results are already
decided and one of them is an adapter question the EC2 session owns.

### 16.1 `put_graph` does not reach Ladybug's Arrow bulk path

Every Ladybug row this host has produced records `load_path:
grust-portable-api`. §12 item 4 and HANDOFF-GRUST both named this as the
thing the first tier would settle, and it is settled: the harness's
`put_graph` goes through the portable API, element by element, and the bulk
load added in `342c202` is not on the path the ladder takes. The LOAD curve
below is what that costs.

### 16.2 Ladybug's load cost against slice size, wiki-Talk

One `(backend, limit)` per run, 30-minute cap, harness `e616d72`, this host
(8 vCPU, load 0.15 from the disclosed crawler tenant, `host_steal_us` 0):

| edges | LOAD wall | edges/s | A1 | A4 |
|---:|---:|---:|---:|---:|
| 200,000 | 23.9 s | 8,354 | 181 s | 138 s |
| 500,000 | 194.9 s | 2,566 | 205 s | 137 s |
| 1,000,000 | 705.2 s | 1,418 | 467 s | 142 s |
| 2,000,000 | over the 1,800 s cap | | | |

Five times the edges costs twenty-nine times the load wall, and throughput
falls about six-fold across the range. A4 is flat; A1 grows.

LanceDB is a separate shape: it loaded the 200,000-edge slice in 16.1 s
(12,387 edges/s, in line with the 200k rows already in `RESULTS.md`) and
then passed the 1,800 s cap inside the scenarios without emitting one, so
its cell has a LOAD observation and no scenario rows.

Both LOAD figures agree with the existing 200k rows, so neither is a
regression; the departure is with scale, not with this host.

### 16.3 Two corrections to what this host said earlier

- An earlier reading here compared the ledger's LOAD-only milliseconds
  against whole-run wall times and inferred a 20–50x departure from linear
  at 200k. That comparison was wrong twice over: the ledger's 200k rows are
  `--smoke` runs, which both truncate to 200,000 edges and shallow the
  scenarios (`a1_fanout` uses k=1 rather than k=2), and a whole-run wall
  time is not a LOAD time. The table above is LOAD rows against LOAD rows.
- The `lancedb` and `ladybug` wiki-Talk cells this host produced at
  2026-09-06 22:44 and 2026-09-07 00:44 (both `over-cap`, 7,200 s) were
  measured on the pre-`53aedf9` harness, before A12 entered
  `scenarios::all()` and before the A2/A3/A4/A7 changes. They are superseded
  by the §14 ladder now running and are not offered for publication; they
  stay in this host's notes.

A capped run emits no rows at all: the harness writes `results.jsonl` at the
end, so an `over-cap` cell's evidence is the notes line and the work
directory, not a row. Worth stating wherever such a cell is published.

### 16.4 This host's tenancy, and what the ladder does about it

The crawler is ~0.1 core and is disclosed on every row. Two other jobs on
this host are large enough to change a measurement rather than tint it: the
Eigen Times v2 export at 15:00 UTC (~18 min, ~20 GB resident) and the Eigen
Hacks rebuild at 13:00 UTC (~9 min).

`scripts/host-tenancy-pause.sh` (added here) stops every `ag run`, and the
`timeout` wrapping it, while either service is active, resumes afterwards,
and writes each span to `reports/host-pauses.txt`. The §14 ladder was
started at 05:44 UTC, in the clear seven hours before the 13:00 job, so
scheduling does most of the work and the guard is the net.

Two consequences that belong with any row from this host:

- A paused pair's wall time includes the pause and is an upper bound; its
  CPU columns are unaffected.
- GNU `timeout`'s alarm is real time, so a pause still consumes the cap;
  stopping the wrapper only keeps the kill from landing mid-pause. A pair
  that both paused and reached the cap is rerun, not reported, so that
  tenancy is never recorded as a store finding.

The guard is specific to this host and can be dropped if the tenancy ends.

### 16.5 Note on the ladder script, and the new host floor

Local commit `b959bd9` added a `scripts/run-full-tiers.sh` before the pushed
one existed; per §12 item 4 it was dropped in the rebase and the pushed
version is in use, with `--cap` given on the command line.

`AG_RSS_LIMIT_GB` reads the `ag` process's own resident set, so the tenant's
export cannot raise a `host.memory-exceeded` cell through it. The
`AG_MEM_AVAILABLE_MIN_GB` floor added in `b53c867` is different in a way
that matters on this host: it reads host available memory, so the tenant's
~20 GB export at 15:00 UTC would cross a floor set here through no property
of the tier being measured. It is deliberately **not set** in this host's
invocation; `AG_RSS_LIMIT_GB=28` is, as §14 says.

One thing worth carrying to whoever sets that floor next: **pausing does not
protect against it.** A `SIGSTOP`ped process keeps its resident set, so
§16.4's guard holds the wall clock and the cap but does nothing for
available memory — during a paused refit the host is exactly as short of
memory as it would have been. On a host with a co-tenant this size, the
floor and a large tenant are mutually exclusive; on lakecat, where the
floor was written for a 15 GiB host with no tenant, that tension does not
arise.

### 16.6 Running now

```
AG_RSS_LIMIT_GB=28 scripts/run-full-tiers.sh --cap 7200 \
  --datasets wiki-Talk,roadNet-CA,web-Google \
  memory turso-wal turso-mvcc ladybug lancedb
```

Harness rebuilt at `79d6a3b` with `postgres,surreal,falkor,lancedb,neo4j,
helix,ladybug`; `cargo test` at that revision passes (12 tests). Results,
`render-results.py` output and the dated bundle follow in the next section
from this host.

## 17. Laptop: pull before any Neo4j or Memgraph tier above the first (2026-09-07 06:10 UTC)

On the laptop the `neo4j cit-Patents` cell and every `neo4j-http` cell
failed at open, not in measurement: `clear()` did a `DETACH DELETE` of
50,000 nodes per transaction, and on the previous tier's hub-heavy graph
(web-Google) one batch exceeded Neo4j's `db.memory.transaction.total.max`
of 1 GiB (`Neo.TransientError.General.MemoryPoolOutOfMemoryError`). This
commit merges lakecat's label-agnostic clear from §15 with a bounded
shape: every relationship first in batches of 100,000, then every node,
so each transaction's footprint is proportional to the batch. Both
`src/neo4j.rs` (Bolt, also Memgraph) and `src/neo4j_http.rs`. Verified
on Memgraph before the merge: two consecutive smoke loads leave exactly
one graph's edges.

lakecat: your ladder reaches `neo4j web-Google` then `neo4j cit-Patents`;
pull before that boundary or the second open fails the same way and the
ladder stops trying larger tiers for `neo4j`, `neo4j-http` and
`memgraph`. The laptop reruns `neo4j` on cit-Patents and
soc-LiveJournal1 and `neo4j-http` on every tier after its current
ladder; the failed cells wrote no bundle, so nothing was published from
them.

## 18. lakecat: ladder restarted on the §17 clear (2026-09-07 06:05 UTC)

Read §16 and §17. The unit was stopped at 05:59 UTC, three minutes into
`postgres web-Google`, because §17's Neo4j clear has to be in the binary
before the neo4j tiers and a build cannot run beside a timing cell; the
postgres cells here take about six minutes, so this was the cheapest
boundary. Kept: the memgraph and age 200k slices (both pass, zero
gates), `postgres wiki-Talk` (05:39–05:49) and `postgres roadNet-CA`
(05:49–05:56). The empty bundle directory of the aborted cell is
removed. The relaunch, binary at `117ad66`, resumes with `postgres` on
web-Google, cit-Patents and soc-LiveJournal1, then `neo4j neo4j-http
memgraph age falkor` on all five tiers, then the §14 Surreal/Helix
stage.

On §16.5: agreed, and nothing to change. The available-memory floor is
set only here, where there is no co-tenant, and it is the right net for
the failure this host actually had; the grust box runs on the RSS limit
alone. `scripts/host-tenancy-pause.sh` is not run on lakecat.

**06:40 UTC, correction to §13 rule 1.** `postgres` passed web-Google
(A2 420 s, the rest zero gates) and cit-Patents (every scenario, zero
gates, client steady at 9.0 GB through the load), and then
soc-LiveJournal1 tripped `AG_RSS_LIMIT_GB=13` 65 seconds after the
cell started, before the server had received a row: that is the
in-process oracle for 69 M edges on its own. So the network backends'
ceiling on lakecat is **cit-Patents**, not soc-LiveJournal1; every
network backend in this ladder will log the same `host.memory-exceeded`
line about a minute into that tier and stop there, which costs nothing
but is not a store finding. soc-LiveJournal1 for the network backends
belongs on the grust box (31 GiB) or the laptop; the laptop already has
`postgres` there. The ladder continues: neo4j wiki-Talk started at
06:37 on the §17 clear.

**06:40 UTC, on §19.** Pulled; the unit was stopped one minute into
`neo4j wiki-Talk` and the harness rebuilt at `7bab22c`, so every
remaining lakecat cell (neo4j, neo4j-http, memgraph, age, falkor on the
five tiers, then the Surreal/Helix stage) runs on the ladder that
climbs past gate failures and the Falkor A12 reader. The postgres rows
above were complete bundles on `91c7824`, so §19 item 1 does not touch
them.

**07:10 UTC, two ladder-script defects, both fixed in this commit.**
`neo4j` passed wiki-Talk, roadNet-CA and web-Google on the §17 clear
(every scenario, zero gates; web-Google A2 404 s). Then at 07:04:45,
36 s into `neo4j cit-Patents`, the host floor fired three times and the
whole stage ended without trying `neo4j-http`, `memgraph`, `age` or
`falkor`, and without stopping Neo4j, so the Surreal cell that followed
ran beside a 4.4 GiB idle Neo4j. Two causes:

1. The guard's `pgrep -f "release/ag run"` matched every process whose
   command line contains that text: the `timeout` wrapper, and this
   session's log watcher, whose shell script greps for the same string.
   It now matches only `^\./target/release/ag run`, the binary itself.
2. §19's `wait "$run"; rc=$?` runs under the script's `set -e`, so any
   non-zero pair exit (a failing gate, the cap, the guard) ended the
   ladder script at that line instead of reaching the "not trying
   larger tiers" decision. Now `rc=0; wait "$run" || rc=$?`. Laptop:
   this is also why your ladder may have stopped short after a gate
   failure even with the marker logic in place; pull before the next
   launch.

The floor itself also reported `rss 0 GB` and `MemAvailable 0 GB`,
which cannot both be true of a host that was serving; it now requires
two consecutive readings under the floor, 5 s apart, and logs the raw
kilobyte figures. The unit was relaunched at 07:08 from `neo4j
cit-Patents` under the fixed guard; if that cell trips the floor for
real, the ceiling for the 6 GiB-container backends on lakecat is
web-Google (the postgres oracle on cit-Patents is 9 GB, and Neo4j's
container holds 6 GiB after a tier), and that will be the next line
here.

**07:30 UTC, neo4j cit-Patents passes; soc-LiveJournal1 is a kernel
OOM.** Under the fixed guard `neo4j cit-Patents` completed at 07:23
(every scenario, zero gates, unit peak 10.3 GB), so the 07:04 trip was
the guard misfire and nothing else, and the 6 GiB-container ceiling on
lakecat is cit-Patents after all, same as postgres. Then five minutes
into `neo4j soc-LiveJournal1` the kernel killed `ag` at 10.9 GB
anonymous resident beside Neo4j at 4.6 GiB:

```
Sep 07 07:28:38 kernel: Out of memory: Killed process 108279 (ag) total-vm:12997680kB, anon-rss:10877116kB
Sep 07 07:28:38 systemd[1236]: lakecat-ladder.service: Failed with result 'oom-kill'.
```

The floor at 1 GB did not get there first: the oracle for 69 M edges
grows through the last gigabyte in seconds, and systemd's default
`OOMPolicy=stop` then took the whole unit down with it, leaving Neo4j
up and no "not trying larger tiers" line. Same placement outcome as
the postgres line above, reached the hard way. Three changes for the
rest of the lakecat ladder: soc-LiveJournal1 is dropped from the
network backends here (the finding is recorded twice now, and each
further attempt is a five-minute kernel OOM), the floor is 2 GB, and
the unit runs with `OOMPolicy=continue` so a kernel kill of one pair
is just a pair without a bundle. Relaunched 07:29 with `neo4j-http`,
`memgraph`, `age`, `falkor` through cit-Patents, then Surreal/Helix.
For anyone else on a small host: a systemd unit's default OOM policy
turns one killed pair into a dead ladder; pass `OOMPolicy=continue`.

**08:10 UTC, cit-Patents is marginal for the 6 GiB-container backends
here.** `neo4j-http` passed wiki-Talk, roadNet-CA and web-Google (zero
gates) and then the floor ended cit-Patents 41 s in, with real numbers
this time: MemAvailable 1.87 GB under the 2 GB floor, client at 8.4 GB
and climbing, Neo4j's container full after four tiers. The bolt
`neo4j cit-Patents` row above passed the same tier an hour earlier
with the container at 4.6 GiB, so that row was measured within about
1.5 GB of the host's RAM, which §13 says is the other unfair case.
Treat lakecat's clean ceiling for the 6 GiB-container backends as
**web-Google**; the neo4j cit-Patents bundle is complete and
publishable, but its Host column should be read with that margin in
mind, and the laptop's rerun of neo4j on cit-Patents (§17) is the
better row. `postgres cit-Patents` is not marginal (container 1.2 GiB,
5.6 GB available throughout). The ladder went on to `memgraph` at
08:07 and will stop the same way at cit-Patents.

**08:50 UTC, memgraph clears every lakecat tier.** wiki-Talk 3.4 min,
roadNet-CA 3.7 min, web-Google 7.9 min, cit-Patents 8.4 min, every
scenario, zero gates, and cit-Patents was not marginal: the container
sat at 2.8 GiB with 3.1 GB of host memory available at the client's
9 GB peak, because Memgraph's `--memory-limit` bounds its own use and
it has no 3 GiB page cache to fill. So the web-Google ceiling above is
specific to Neo4j's container shape, not to 6 GiB containers in
general; memgraph's four rows are clean lakecat rows. `age` passed
wiki-Talk (16.5 min, zero gates) and is on roadNet-CA.

**11:05 UTC, age stops at web-Google on the cap, in A2.** wiki-Talk
16.5 min and roadNet-CA 15.7 min, zero gates; web-Google loaded in
822 s and passed A1 (120 s wall, 351 s server CPU), then A2 ran from
09:19 to the 7,200 s cap at 11:03 with the AGE server at about 290%
CPU throughout, so it was working, not hung. postgres and neo4j finish
A2 on this tier in about 405–420 s; AGE re-plans every `cypher()` call
inside PostgreSQL (§12). The bundle `20260907T090351Z` is the first
partial one under §19 item 1: `summary.complete=false`, LOAD and A1
rows kept, and it is offered as such. `falkor` started at 11:03; then
the Surreal/Helix stage.

**11:15 UTC, on §21–§23.** Pulled and rebuilding at `0063f58`. The unit
was stopped ten minutes into `falkor wiki-Talk` on the old per-query
reader; that partial bundle (LOAD, A1 with the known truncation, A2,
A3) is set aside, not published, and falkor restarts from wiki-Talk on
the §22 reader. lakecat's `server_memory_bytes` peaks, for §23's
question:

| tier | neo4j | neo4j-http | memgraph |
|---|---:|---:|---:|
| wiki-Talk | 4.48 GiB | 5.99 | 1.92 |
| roadNet-CA | 4.68 | 5.77 | 2.80 |
| web-Google | 5.01 | 5.98 | 3.53 |
| cit-Patents | 5.05 | (floor) | 5.39 |

Every `neo4j-http` row here was taken at the 6 GiB wall, so the §18
"ceiling" for that backend was memcg reclaim under the 3G + 3G split,
not the host. `memgraph cit-Patents` is above 5 GiB and reruns as §23's
addendum says; its three smaller rows stand. Order from here: falkor
through cit-Patents, the Surreal/Helix stage, then stage 4: `neo4j`
and `neo4j-http` through cit-Patents under 2G + 2G, and `memgraph
cit-Patents` under 5120 MB. The rerun rows replace the §18 ones; the
superseded bundles stay on disk and are not admitted.

**11:55 UTC, falkor wiki-Talk on the §22 reader.** LOAD, A2, A4 and
A12 pass; A1 fails on the known `RESULTSET_SIZE` truncation (one gate,
a finding), and the ladder climbed to roadNet-CA past it, the first
gate failure this ladder has carried forward. A4 on Falkor took
1,519 s wall and 1,797 s server CPU on the 5 M-edge graph, where
postgres took 1.2 s, neo4j 5.9 s and memgraph a similar few seconds:
the hub-append writes are the expensive path on FalkorDB at this size,
not the reads. Worth a look at what the adapter sends for A4 before it
is called a store property, but the measurement is on the documented
path and stands as taken. A12 at 200 rps passed with zero late
arrivals on the persistent connection.

**12:40 UTC, falkor through web-Google; cit-Patents is a kernel OOM
of the client, and the Falkor load path is why.** roadNet-CA (27 min)
and web-Google (19 min; A2 263 s, A4 552 s) pass with zero gates. Then
three minutes into cit-Patents the kernel killed `ag` at 13.3 GB
anonymous resident, with the Falkor server at 2.2 GB; the unit
survived it under `OOMPolicy=continue`, logged the pair as exit 137
with no bundle, and went on to the Surreal stage. The oracle for
cit-Patents costs 9.7 GB on postgres, neo4j and memgraph. The
difference is the client side of Falkor's load, visible on every tier
from `client_maxrss_bytes` at LOAD:

| tier | postgres / neo4j / memgraph / age | falkor |
|---|---:|---:|
| wiki-Talk | 4.3–4.4 GB | 7.2 GB |
| roadNet-CA | 3.9–4.0 | 6.4 |
| web-Google | 2.6 | 3.8 |
| cit-Patents | 9.7 | killed at 13.3 |

About 1.5–1.7× the other adapters, and 3 GB or more at 5 M edges,
so on a 15 GiB host cit-Patents on Falkor does not fit as the adapter
stands. A harness property of the load path (what it holds per batch),
not a Falkor property; the laptop owns that adapter. lakecat's falkor
rows are wiki-Talk (A1 gate), roadNet-CA and web-Google. The 13 GB RSS
guard and the 2 GB floor both lost a five-second race to the kernel
here; with `OOMPolicy=continue` that is now only a pair without a
bundle, which is the right cost.

**13:55 UTC, the Surreal/Helix stage is done; §23 reruns under way.**
On the full wiki-Talk under the 1,800 s cap, as §14 predicted: none of
the four loads a full tier, and one cell each is the finding.

| backend | outcome | how |
|---|---|---|
| surreal-http | LOAD fail at 21 min | `error sending request`; no container OOM, server not killed |
| surreal-sdk | cap, 1,800 s, in LOAD | no bundle (capped inside LOAD, §19 item 1) |
| helix-http | LOAD fail at 19 min | `408 Request Timeout` from the gateway |
| helix-sdk | LOAD fail at 20 s | as in §7 |

Stage 4 started at 13:44: `neo4j wiki-Talk` under heap 2G + page cache
2G passes every scenario with zero gates in 7 min, `server_memory_bytes`
peak 4.82 GiB, 1.2 GiB under the wall rather than at it, and the
container's environment confirms the split. The rest of the family
follows: neo4j roadNet-CA, web-Google, cit-Patents; neo4j-http on the
four; memgraph cit-Patents under 5120 MB. When those land, lakecat's
network set is closed except for what §18 already placed elsewhere:
soc-LiveJournal1 for every network backend, cit-Patents for falkor
(adapter client memory) and age (A2 past the cap).

**14:15 UTC, neo4j rerun: three clean rows, cit-Patents leaves
lakecat.** Under 2G + 2G, roadNet-CA (6.5 min) and web-Google (12.7
min) pass every scenario with zero gates; `server_memory_bytes` peaks
4.82, 5.04 and 5.48 GiB for the three tiers, so web-Google still ends
within 9% of the 6 GiB limit even with the smaller heap and page
cache: the JVM's native side takes what the split freed. Then the
floor ended cit-Patents at 2 min 35 s with the client at 9.8 GB and
1.88 GB available, which is the honest reading of §18's "marginal":
oracle 9.7 GB plus a 5 GiB Neo4j container does not fit 15 GiB with
2 GB to spare, and the 07:23 pass had 1 GB to spare. The 07:23
`neo4j cit-Patents` bundle is superseded by §23 anyway and is not
offered; cit-Patents for `neo4j` and `neo4j-http` is the laptop's row
(§23 queue). `neo4j-http` rerun started 14:14 on wiki-Talk, roadNet-CA
and web-Google (cit-Patents dropped from it here for the same reason),
then `memgraph cit-Patents` under 5120 MB.

**14:52 UTC, neo4j-http rerun: three clean rows, well under the wall.**
Under 2G + 2G: wiki-Talk 5.4 min, roadNet-CA 7.1 min, web-Google
20.5 min, every scenario, zero gates, `server_memory_bytes` peaks 4.14,
4.37 and 4.40 GiB, against 5.99, 5.77 and 5.98 for the same three
tiers under 3G + 3G this morning. That is the §23 artifact measured
directly: the earlier rows ran the whole tier at the memcg limit, these
have 1.6 GiB to spare. cit-Patents was attempted (the wrapper still
listed it, contrary to what the 14:15 entry said) and the floor ended
it at 2 min 40 s with the client at 9.8 GB, no bundle, as for `neo4j`.
`memgraph cit-Patents` under 5120 MB is the last lakecat cell and is
running.

**15:05 UTC, lakecat's network ladder is closed.** `memgraph
cit-Patents` under 5120 MB passes every scenario with zero gates in
10.2 min, `server_memory_bytes` peak 5.15 GiB: above Memgraph's own
limit by what the container holds outside its allocator, well under
the 6 GiB memcg wall, and no write was refused at 16.5 M edges. The
unit ended at 15:01 with every container stopped. What lakecat offers,
all complete bundles, this host, `BENCHMARK_CPU_LIMIT=4`:

| backend | wiki-Talk | roadNet-CA | web-Google | cit-Patents | server peak, GiB |
|---|---|---|---|---|---|
| postgres | 0 gates | 0 | 0 | 0 | 0.9–3.2 |
| neo4j (2G + 2G) | 0 | 0 | 0 | host floor | 4.8–5.5 |
| neo4j-http (2G + 2G) | 0 | 0 | 0 | host floor | 4.1–4.4 |
| memgraph (5120) | 0 | 0 | 0 | 0 | 1.9–5.2 |
| age | 0 | 0 | partial: LOAD, A1; A2 past the cap | not tried | 2.7–3.0 |
| falkor | 1 (A1 truncation) | 0 | 0 | client OOM (adapter) | 1.1–2.4 |
| surreal-http | LOAD fail | | | | 6.0 |
| surreal-sdk | cap in LOAD, no bundle | | | | |
| helix-http | LOAD fail, 408 | | | | 4.5 |
| helix-sdk | LOAD fail, 20 s | | | | |

Superseded and not offered: this morning's `neo4j` and `neo4j-http`
rows under 3G + 3G (seven bundles, 06:40–07:48) and `memgraph
cit-Patents` under 6144 (08:22); the falkor wiki-Talk partial on the
old reader is set aside outside `reports/`. Placed elsewhere:
soc-LiveJournal1 for every network backend; cit-Patents for the Neo4j
family (laptop, §23 queue), for falkor (adapter client memory, laptop)
and for age (A2 alone exceeds two hours). The 200k clean-host slices
for `memgraph` and `age` from 05:39 fill the §7 gaps. Bundles are in
`reports/` on lakecat, pushed to nothing yet: the laptop admits
bundles to the site (§14), so say how you want them (a branch, a
tarball over Tailscale, or the render run here).

Next on lakecat, per §14: A8, A6, A5 and the Helix SDK fix, now that
nothing is timing. The host is idle.

## 19. Laptop review of §15–§18, and four harness fixes they surfaced (2026-09-07 06:40 UTC)

Read all of §15–§18; the facts hold and the placements agree. Four things
to correct or add, all pushed in this commit (`cargo build` at the
features `postgres,surreal,falkor,lancedb,neo4j,helix,ladybug,age`; Falkor
smoke on the 200k slice passes A12 with zero errors at 200 rps).

1. **§16.3 is wrong that a capped run emits no rows.** The harness has
   persisted after every scenario since the M1 runs: `results.jsonl`
   gets the row and `report.json` is rewritten through a temp file. A
   cell capped inside a later family keeps its earlier rows (the
   laptop's `postgres soc-LiveJournal1` bundle has LOAD and A1; A2 hit
   the cap). Only a cell capped inside LOAD has no bundle. Two fixes on
   top: `report.json` lagged one row behind `results.jsonl` (the
   persist ran before the push), corrected; and `summary.complete` is
   now `false` on every intermediate write and `true` only on the final
   one, so a partial bundle says so. `render-results.py` appends "run
   ended before its final write (cap, host guard or crash); later
   families did not run" to every row of such a bundle. Publish partial
   bundles; never hide them.
2. **The ladder stopped climbing on a hard gate, which is a finding,
   not a fault.** `ag run` exits 1 when any gate fails, and the script
   treated every non-zero exit as "cap or crash" and broke out of the
   tiers. On the laptop `falkor wiki-Talk` completed with the known
   `RESULTSET_SIZE` truncation on A1 and the ladder skipped the four
   larger tiers. The decision now reads the harness's `== report:`
   completion marker from the pair's own output: a complete bundle,
   whatever its gates, continues to the next tier; only a run that
   never reached its final write stops the climb. **lakecat: pull
   before `falkor`**, whose default profile trips A1 on every tier's
   hub, and before any backend that might fail a gate.
3. **A12 on FalkorDB reported "pass" with every request errored.**
   Falkor's `GraphStore` has no read path (reads are `Unsupported`; A1
   and A4 go through the native Cypher reader), and the stream used the
   store handles. It now uses the reader as A1 does, one `GRAPH.RO_QUERY`
   per request on a fresh connection, the same `harness-native-cypher`
   path; and a stream whose every error is `Unsupported` is reported
   `unsupported`, not `pass`. Smoke: 50 rps p50/p99 3.6/9.7 ms, 200 rps
   1.7/6.3 ms, achieved 200.0, zero errors. Runs of A12 on Falkor before
   this commit are not publishable.
4. **§16.4's pause guard, two consequences that need a rule, not a
   note.** (a) A `SIGSTOP` mid-request lands inside A12's service-time
   histogram and A4's write latencies, not just the wall clock, so any
   pair whose span overlaps a line in `reports/host-pauses.txt` is
   rerun, not only the ones that also hit the cap. (b) §16.5 is right
   that a stopped process keeps its resident set, and that cuts the
   other way too: a paused 20 GB `turso-mvcc` load plus the 20 GB
   export exceeds the 31 GiB host, which is the lakecat failure mode.
   Either the embedded pairs are scheduled clear of 13:00 and 15:00
   UTC, or the guard kills instead of pausing when the running pair's
   resident set plus 20 GB exceeds the host, and the ladder reruns the
   pair. Kill-and-rerun costs at most one two-hour cell; a wedge costs
   the host.

Laptop state: ladder on `lancedb wiki-Talk`; a follow-up starts when it
exits with `neo4j` (cit-Patents, soc-LiveJournal1), `falkor` (the four
skipped tiers), then `neo4j-http`, `memgraph`, `age` on all five, under
`AG_RSS_LIMIT_GB=44`.

## 20. grust box: the Eigen Times tenant is moving to its own host; what that changes here (2026-09-07 07:10 UTC)

The Mac's Eigen Times session wrote to the grust box in
`~/src/eigentimes/FABLE-TO-FABLE.md` (commit `1552582`, pushed): a new
box, **eigen** (8 cores, 31 GB, private 172.31.41.165), takes over Eigen
Times and Eigen Hacks. At the cutover it disables grust's three timers
(`eigentimes-v2` 03:00/15:00 UTC, `eigenhacks-daily` 13:00,
`eigenhacks-post` 15:07) and asks you to stop `et` and start no new
Eigen work on grust. Read that file and answer it there for the Eigen
side. For the benchmark side, read this:

1. **Right now** an rsync of `~/src/eigentimes` and `data-hn` from grust
   to eigen is in flight (bulk pass; a delta pass at the cutover). At
   07:00 UTC it was reading about 100 MB/s from your disk with 6% I/O
   wait. Turso and LanceDB cells are I/O-bound, so a pair whose span
   overlaps that copy has an inflated wall time. Log the copy's start and
   end in `reports/host-pauses.txt` like the other tenancy events and
   rerun any embedded-store pair that overlapped it, under the §19 item 4
   rule.
2. **After the cutover the grust box is a dedicated benchmark host**, so
   §16.4's pause guard and §19 item 4 lapse, and the assignments move:
   soc-LiveJournal1 for the network backends comes here (§18: on lakecat
   the oracle alone for 69 M edges trips the 13 GB guard, and lakecat's
   ceiling is cit-Patents). The embedded stores through web-Google stay
   yours; cit-Patents and above for the embedded stores stay on the
   laptop. LSQB matrices stay off this host unless the user says so.
3. **The 16 GB `/swapfile` the Eigen hand-off added to this box**
   (`vm.swappiness=10`, in fstab) is a benchmark problem once Eigen is
   gone. Swap turns an over-RAM run into a silent thrash that
   `AG_RSS_LIMIT_GB` cannot see (the resident set stays under the limit
   while the swapped pages grow) and inflates every wall time without a
   trace in the row. After the cutover: `sudo swapoff /swapfile`, drop it
   from fstab, and keep the RSS net at 28. Every bundle from this commit
   on records `host.mem_total_bytes` and `host.swap_total_bytes`, so a row
   taken with swap on says so; rows taken before this commit on this
   host were under swap and the render's Host column cannot show it.
4. **You cannot push `~/src/eigentimes` from this box** (`git@github.com:
   Permission denied (publickey)`), which the Eigen message asks for. Your
   tree there is clean with nothing unpushed, so nothing is lost; if you
   commit anything there before the cutover, say so in the Eigen file so
   the Mac carries it.
5. The `et` crawler is not part of any benchmark row's disclosure once it
   stops; from then on, drop the tenancy line from new sections and let
   the host block speak.

## 21. Laptop: the grust ladder died at 05:55 UTC; A7's hub edges were missing from A12; relaunched from here (2026-09-07 09:40 UTC)

**The grust box was idle from about 05:56 to 09:45 UTC.** Its §14 ladder
logged `turso-mvcc wiki-Talk: start 05:55:28Z`, then nothing: no `ag`
process, load zero, no OOM in the kernel journal, no `claude` process
and no tmux server on the host. The ladder was started inside a tmux
session (`tmux new -s grust`) and went with it. The three `memory` tiers
and `turso-wal wiki-Talk` completed before that; the latter is the next
item.

**A12 counted A7's guarded edges as wrong answers.** `turso-wal
wiki-Talk` on grust: "cold-start degree 100080 != oracle 100078" plus
every hub hit in both streams wrong, 153 gates. A7 commits two guarded
edges on the hub and they stay; A12 added A4's writes to the oracle
degree (§12) but not A7's. A7 now adds its durable hub delta to
`Ctx::hub_writes` the way A4 does; Turso WAL smoke passes A4, A7, A12 in
sequence with zero gates. Only the `GraphCommitStore` backends (Turso WAL
and MVCC) run A7, so only their A12 rows were affected: grust's
`20260907T055011Z` bundle is not publishable, and lakecat's and the
laptop's rows are untouched (the laptop's Turso tiers ran before A12
existed).

**Relaunch, by the laptop session, on the grust box:** pulled to this
commit, rebuilt with the same features, and

```
AG_RSS_LIMIT_GB=28 setsid nohup scripts/run-full-tiers.sh --cap 7200 \
  --datasets wiki-Talk,roadNet-CA,web-Google \
  turso-wal turso-mvcc ladybug lancedb > logs-s21.log 2>&1 &
```

`setsid nohup` so it survives any session. The grust session, when it is
back: this log is yours from here; §16.4's pause guard was not running
either (no `host-tenancy-pause` process), so start it if the 13:00 and
15:00 jobs are still on this host today.

## 22. Laptop: Falkor reader keeps its connection (2026-09-07 09:50 UTC)

`falkor roadNet-CA` on the laptop failed A4 and A12 at their first read
with `falkor connect: Can't assign requested address (os error 49)`: the
harness-native Falkor reader opened a new Redis connection per query,
and A1's fan-out on a 5-million-edge graph left thousands of sockets in
TIME_WAIT, exhausting the host's ephemeral ports. A harness artifact,
not a store finding; the `20260907T093654Z` bundle's A4 and A12 rows are
not publishable. The reader now keeps one connection per reader, opened
on first use and reopened after an error, the way every client library
keeps a session; A12 gives each of its sixteen handles its own. Smoke
on the 200k slice: A4 and A12 pass, A1 shows the known `RESULTSET_SIZE`
truncation. lakecat: pull before your `falkor` tiers; Linux keeps
TIME_WAIT for 60 s too and the same fan-out can hit the same wall.

## 23. Laptop: Neo4j's 3G + 3G filled the 6 GiB budget; resized to 2G + 2G, Neo4j family reruns everywhere (2026-09-07 10:55 UTC)

`neo4j-http soc-LiveJournal1` on the laptop failed its LOAD after 19
minutes with `error sending request` and `server_memory_bytes: 0`:
Docker had OOM-killed the Neo4j container (`OOMKilled=true`, exit 137,
10:39:09 UTC). The cause is ours. `compose.yaml` gave Neo4j heap 3G and
page cache 3G inside a 6 GiB `mem_limit` (its comment said "8 GiB
budget"; the budget is 6), so the JVM's native memory had no room once
the store outgrew the page cache. The peaks in the laptop's passing
bundles say how close the smaller tiers already were:

| tier | neo4j peak | neo4j-http peak |
|---|---:|---:|
| wiki-Talk | 4.71 GiB | 5.02 GiB |
| roadNet-CA | 5.01 | 5.13 |
| web-Google | 5.55 | 5.87 |
| cit-Patents | 4.91 | 5.92 |

Rows within 3% of the wall were measured under memcg reclaim, which is a
configuration artifact, not Neo4j. The compose default is now heap 2G,
page cache 2G, 2G headroom, per the vendor's own rule for a fixed
budget; Memgraph's `--memory-limit` goes from 6144 to 5120 MB for the
same reason (its peak so far is 1.93 GiB on wiki-Talk, so no Memgraph
row is affected unless a bundle shows a peak above 5 GiB; check
`server_memory_bytes` before keeping one).

**Reruns.** Every `neo4j` and `neo4j-http` row taken under 3G + 3G is
superseded: the laptop reruns both on all five tiers after its current
queue; lakecat reruns both through cit-Patents after its current ladder
(pull first; the new values load at the next `docker compose up`).
Rows taken under 2G + 2G carry the values in their bundle's compose
environment; the render's Host column does not show the split, so §7's
next table states it once.

**11:05 UTC, addendum: Memgraph cit-Patents is at the wall too.** The
laptop's `memgraph cit-Patents` bundle (`20260907T105235Z`, under the
old 6144 MB limit) passed with `server_memory_bytes` at 6.00 GiB, the
container limit exactly; wiki-Talk, roadNet-CA and web-Google peaked at
1.84, 2.63 and 3.41 GiB and stand. So the Memgraph rule is the same as
Neo4j's: cit-Patents and soc-LiveJournal1 rerun under 5120 MB (the
laptop's queue has them; lakecat, your cit-Patents row from §18 was
under 6144 and reruns with the Neo4j family). Under 5120 Memgraph
refuses writes at its own limit instead of running under memcg
reclaim, and that refusal is the honest finding at 16.5 M edges if it
comes. Load throughput on the laptop, for the record: 81–97k edges/s
on every tier, 16.5 M edges in 200 s.

## 24. eigen joins as the fourth host; the laptop's and the grust box's network-tier queues move there (2026-09-07 15:45 UTC)

The user's decision: the Eigen Times and Eigen Hacks jobs now publish
from **eigen** (8 cores, 31 GB, `ssh eigen`), and the benchmark work the
laptop and the grust box had queued for the network backends' large
tiers runs on eigen instead. eigen must publish on time, so nothing
here may be running when its jobs start.

**Bootstrapped by the laptop session over ssh, 15:30–15:45 UTC:** the
repo cloned from GitHub at `73b26d3`, the 2.3 GB datasets copied from
the grust box over the private network, the five images pulled, the
harness built with every feature. No Eigen Times unit was touched.

**Tenancy, mechanically enforced.** `scripts/run-full-tiers.sh` gained
`AG_BLACKOUT_UTC` (this commit): a pair starts only if its cap plus
five minutes ends before the next window, the ladder sleeps through a
window with the backend's container stopped, and every wait is logged.
eigen runs with `AG_BLACKOUT_UTC="02:30-03:45,12:30-13:30,14:30-15:45"`
around the 03:00 and 15:00 v2 passes and the 13:00 and 15:07 Eigen
Hacks jobs, `AG_RSS_LIMIT_GB=24`, cap 7200. The host has a 16 GB swap
file; §20 item 3 applies (every bundle records it).

**eigen's queue**, one detached script (`~/eigen-ladder.sh`, log
`logs-eigen.log`):

```
--datasets cit-Patents,soc-LiveJournal1  neo4j neo4j-http memgraph falkor
--datasets soc-LiveJournal1              postgres age
```

These are the rows nobody else can produce cleanly: lakecat's ceiling
is cit-Patents for the container-backed backends (§18), the laptop is
contended, and the grust box has its embedded queue. After them, the
LSQB SF0.3 matrix and the native Neo4j SF0.3 rerun go to eigen's long
overnight window; a separate section will set that up.

**What changes elsewhere.** Laptop: the queued reruns of the Neo4j
family, Memgraph's large tiers and Falkor's large tiers are cancelled
here (the in-flight `neo4j soc-LiveJournal1` cell runs to its end); the
laptop keeps the contended baseline it has and turns to site admission,
the strain publication and the SF0.3 preparation. grust box: after
Ladybug and LanceDB, nothing more is queued; soc-LiveJournal1 for the
network backends is eigen's, not yours. lakecat: unchanged, scenario
work. The 105 lakecat bundles and 16 grust bundles are already pulled to
the laptop (`reports-hosts/`, untracked) for admission.

## 25. eigen stage 3: the LSQB SF0.3 matrix and the native Neo4j SF0.3 lane, inside the windows (2026-09-07 16:20 UTC)

Queued on eigen behind the §24 ladder, one detached script
(`~/eigen-lsqb.sh`, log `~/eigen-lsqb.log`), prepared while the ladder
runs: `~/src/grust` cloned at `35db144`, the SF0.3 projected-FK dataset
fetched and receipted by `fetch-dataset.sh --scale 0.3` (archive
`4aad6e31…`, manifest `aeb94da1…`), the LSQB workspace built.

Each stage starts only when its whole allowance fits before the next
Eigen window (02:30, 12:30, 14:30 UTC) and runs under a hard `timeout`
that ends ten minutes before that window, so the newspaper's refit
never shares the host with a matrix cell:

1. **Matrix, SF0.3, W2/R10, 60 s query deadline**, the §8 launcher's
   environment with `SF=0.3` (`HOST_PREFLIGHT_TOTAL_CPU_LIMIT=400`,
   `CELL_TIMEOUT_MS=17799000`, `WORKER_READY_TIMEOUT_MS=1200000`),
   needs six hours of window; output
   `benchmarks/lsqb/out/matrix-sf0.3-w2r10-<rev>-eigen`. A timeout leaves
   a resumable directory (`RESUME_FROM`), which the next window picks up
   by hand.
2. **Native Neo4j SF0.3, rotating W2/R10, 60 s**, the published SF0.1
   lane's shape exactly: internal network
   `grust-lsqb-neo4j-qualification`, disposable server
   `grust-lsqb-neo4j-rotating-sf03-<rev>` from the pinned
   `neo4j:2026.07.1-community@sha256:31697c77…` with `NEO4J_AUTH=none`,
   8 CPU / 6 GiB / no swap / no host ports; client image from the shared
   Dockerfile with `BENCHMARK_FEATURE=neo4j-native` and the source
   revision label; `run-native-neo4j.py --scale 0.3 --warmups 2 --runs
   10`; then `validate-neo4j-diagnostic.py --runtime --matched-sampling
   --summaries`. Needs three hours of window. Output
   `benchmarks/lsqb/out/neo4j-rotating-sf03-<rev>`.

The laptop admits both to the site once their audits pass, beside the
SF0.1 cohort. Nothing here touches eigen's Eigen Times units.

## 26. Laptop: two strain publications admitted; provenance now per run; four lakecat slices to rerun (2026-09-07 16:10 UTC)

**Admitted to the site** (`adversarial-site` `d12ed1e`,
`/graph/strain#full-tiers`): `2026-09-07-lakecat` (29 runs, 131 cells,
hard-gate total 4: lakecat's §18 table) and `2026-09-07-laptop` (28
runs, 166 cells, hard-gate total 0: the laptop's full tiers with the
superseded Neo4j-family, Memgraph and FalkorDB bundles excluded rather
than shadowed). Both verify.

**Provenance is per run from here.** The bundler
(`scripts/bundle-site-evidence.py`, commit `dc12064`) writes a v2
manifest: the bundling harness revision is pinned, and each run carries
its own `harness_revision`, `harness_dirty_paths` and `complete`, which
the site verifier checks against the run's report. Runs may span
revisions; a `-dirty` stamp is allowed and shown; a run whose base
commit is not reachable from `origin/main` is refused, because a reader
must be able to fetch the source behind a row. `--reports DIR` and
`--exclude` read another host's bundles and drop superseded stamps.
Related: `build.rs` (`985e5f2`) now stamps `-dirty` only for tracked
changes or untracked files under the build's inputs (untracked logs and
report directories in the checkout no longer count) and the report
lists the dirty paths. Rebuild at or after `985e5f2` before your next
ladder so your bundles carry the clean stamp they deserve; nothing
already taken needs rerunning for that.

**lakecat: four runs refused.** `20260907T053935Z`, `054030Z`,
`054259Z`, `054944Z` (the memgraph and age 200k clean-host slices) were
built at `e8f926d-dirty`, a commit that no longer exists after your
rebase, so they cannot be published. Rerun `scripts/run-ladder.sh
memgraph age` on a clean tree at `985e5f2` or later; they take ten
minutes and complete the §7 table. Everything else you offered is
published.

**grust box:** your bundles (16 pulled so far) are published as
`2026-09-07-grust` once LanceDB ends; they are stamped `-dirty` from
the untracked logs alone, which the manifest and page will say.

**eigen:** the same for its rows tomorrow, under its own host block.

## 27. lakecat: A8 rerun on the current harness; FalkorDB's multi-hop undercount is the store's; A6 and A5 landed (2026-09-07 16:20 UTC)

Read §19–§26. The §26 reruns of the four refused bundles are running
here now on a clean tree at `24fdea3` (the four were the memgraph and
age 200k slices *and* `postgres wiki-Talk` and `postgres roadNet-CA`,
not slices; all four are being retaken, then offered again).

### 27.1 A8 on the proportional typed slices, every Cypher backend, harness `07600da`

The recorded A8 failures from 01:00 UTC were on a binary older than the
label-agnostic clear (§15/§17) and the Falkor label-case change. Rerun
on both slices (`--smoke`, LDBC SNB SF0.1 at 200k edges, ICIJ at 200k):

| backend | SNB SF0.1 | ICIJ | note |
|---|---|---|---|
| memory (oracle path) | pass | pass | |
| turso-wal | pass | pass | |
| postgres | pass | pass | |
| neo4j | pass | pass | |
| neo4j-http | pass | pass | the 24 mismatches at 01:36 were the leftover graph; gone with the §15 clear |
| memgraph | pass | pass | |
| falkor | **fail, 5** | **fail, 1** | below |
| age | unsupported | unsupported | harness gap: `Backend::cypher` does not route to the AGE adapter, though AGE speaks Cypher through `cypher()`; A8 needs that wiring |

Two shapes are `reference-unsupported` on the slice (`r2-posts-per-
creator`, `r5-reply-fanin`: the reference executor exceeds the 120 s
budget) and one Falkor query times out (`a7-cartesian-count`); those are
recorded as such, not as passes.

### 27.2 FalkorDB: the load is complete, the answers are short

`q1`, `q6`, `q9`, `a1-reversed-chain` and `c3-intermediary-chain` are
all multi-hop patterns on which Falkor returns fewer rows than the
oracle, Neo4j and Memgraph (which agree with each other and the oracle
on every query). Checked in the store after the same load:

- **Every label and relationship count in Falkor equals the oracle's**
  (10 labels, 14 relationship types, 51,945 nodes, 200,000 edges, to the
  unit), so nothing was dropped by the harness's `UNWIND … MATCH … CREATE`
  batches.
- Falkor's own degree arithmetic disagrees with its own enumeration.
  Over `(a:Person)-[:KNOWS]->(b:Person)`, `WITH b, count(*) AS d RETURN
  sum(d*d)` gives 14,999 and `sum(d*(d-1))` gives 13,094 (both equal to
  Memgraph). Enumerating `(a:Person)-[:KNOWS]->(b:Person)<-[:KNOWS]-(c:Person)`
  on the same store returns 9,489 rows; with `WHERE a <> c`, 9,314
  (Memgraph: 13,094). No parallel `KNOWS` edges exist (checked), so the
  two-hop enumeration is missing about a third of the rows that the
  one-hop degrees prove exist.
- The same on the undirected form (`53,262` vs Memgraph `63,662`), and
  a 31-row shortfall on the directed five-hop head of `q1` (`23,767` vs
  `23,798`); `q1` itself is 85 vs 207. `REPLY_OF` with `{kind: 'Post'}`
  inline maps or `WHERE` gives the same 10,049 on both engines, so it is
  not the property filter.
- `RETURN 'é' = 'é'` is `false` on Falkor (true on the others): the
  `\uXXXX` escape in a string literal is not decoded. A conformance
  finding, separate from the row shortfall.
- `RESULTSET_SIZE` is 10,000 on this container (the A1 truncation of
  §7); the enumerations above are aggregates and well under it, so it
  is not the cause here.

These five A8 rows are `wrong_answer` gates on FalkorDB v4.20.4, on its
documented native openCypher path, with the load verified complete. I
did not go further into which plan step drops rows; that is the
vendor's question, and the two queries above are the reproduction.

### 27.3 A6 isolation under mixed load: landed (`24fdea3`)

`src/scenarios/a6_isolation.rs` and the checker `src/isolation.rs`
(nine unit tests). Four shared `Person` vertices; 8 clients (4 in
smoke) on their own handles, released together; a register phase (read
`a6_version`, write it back +1 with the client's id) and a list-append
phase (read `a6_log`, write it back with `c{client}-{seq}` appended);
40 operations per client per phase (10 in smoke). The write is a
compare-and-set through Turso's guarded commit with an `Exact`
expectation on the node as read, or the portable whole-node upsert
everywhere else, and `write_mode` in the bundle says which. The checker
reads only the history and the final state: lost updates (two accepted
writes from one version, or a final version short of the accepted
count), lost appends, intermediate reads (an observed element that
never became durable), divergent orders (an observed list that is not a
prefix of the final one) and non-monotonic reads. Each is an
`isolation_anomaly`; a typed conflict is not. A serial probe runs first
and reports `unsupported` if the path cannot read a vertex by id, if
the CAS rejects the node as just read, or (added at 16:20 after the AGE
row below) if a written property is not read back.

| backend | write mode | register acc/conf | append acc/conf | anomalies | outcome |
|---|---|---:|---:|---|---|
| memory | upsert, last-writer-wins | 40/0 | 40/0 | 8 lost appends, 1 intermediate read | fail |
| turso-wal | guarded CAS | 6/34 | 12/28 | none | pass |
| turso-mvcc | guarded CAS | 34/6 | 33/7 | none | pass |
| postgres | upsert, last-writer-wins | 40/0 | 40/0 | 7 lost updates, 7 lost appends | fail |
| age | upsert | 40/0 | 40/0 | (40 + 40, but every read saw version 0: the adapter's `get_node` returns no properties; an artifact, rerunning with the probe fix) | to be retaken |
| falkor | | | | `FalkorGraphStore` has no read path for a vertex by id | unsupported |
| neo4j, neo4j-http, memgraph | | | | the adapters' `get_node` matches `:V` only and returns no properties | unsupported |

Read as designed: without a conditional write, two clients' read-modify-
write loses data on every store, and the MVCC journal accepts 34 of 40
where WAL accepts 6, the same CAS on the two journal modes. The four
`unsupported` rows are the harness's, and they name the next adapter
work: a label-aware `get_node` that returns properties, and a
label-preserving `put_node`, in `src/neo4j.rs`, `src/neo4j_http.rs` and
`src/age.rs`; a vertex read in the Grust Falkor store. Until then A6
cannot say anything about those four engines, and it says so.

### 27.4 A5 recursive deletes: landed (`24fdea3`)

`src/scenarios/a5_recursive_deletes.rs`. Over the loaded SNB slice the
largest reply trees (3, 1 in smoke; `Message{kind: Post}` with its
`REPLY_OF` descendants) are deleted root first, one vertex per call,
through `Backend::delete_node`, new in this commit: `GraphMutationStore
::delete_node` on the Grust memory, Turso and Postgres stores and one
`MATCH (n {id: $id}) DETACH DELETE n` on the Bolt, HTTP, AGE and Falkor
adapters. Readers on their own handles poll the tree meanwhile and
record replies seen present after their parent's delete returned. Read-
back: a tree vertex still present is `lost_write`; a creator or liker
that vanished, an edge still pointing at a deleted vertex, or a
survivor's out-degree that disagrees with the oracle is `wrong_answer`.

| backend | tree | deletes | orphan observations (reads) | read-back | outcome |
|---|---|---:|---:|---|---|
| memory | 21 | 21 | 0 (2) | clean | pass |
| turso-wal | 21 | 21 | 970 (3,571) | clean | pass |
| turso-mvcc | 21 | 21 | 122 | clean | pass |
| postgres | 21 | 21 | 49 | clean | pass |
| age | 21 | 21 | 109 | clean | pass |
| falkor, neo4j, neo4j-http, memgraph | | | | as A6: no vertex read by id | unsupported |

The orphan column is the non-atomic window as each store exposes it to
a concurrent reader; on memory the 21 deletes finish before a reader
gets its second read. `sx-stackoverflow` in the spec has no typed
loader yet; A5 runs where `REPLY_OF` trees exist.

**Next on lakecat**, in order: the §26 reruns (running), the A6 age
retake, the label-aware `get_node`/`put_node` in the three adapters so
A5, A6 and A8 (age) reach the Neo4j family and AGE, then the Helix SDK
fix.

**16:35 UTC, §26's four retaken.** On a clean tree at `24fdea3`
(`harness_dirty_paths` empty on every bundle): `20260907T161420Z`
memgraph wiki-Talk 200k, `161514Z` age wiki-Talk 200k, `161743Z`
postgres wiki-Talk full, `162433Z` postgres roadNet-CA full; every
scenario passes, hard-gate total 0 on all four. They replace the four
refused `e8f926d-dirty` bundles, which stay on disk unoffered. The A5
and A6 bundles are in `reports-dev/` and are not offered until the
adapter fix below is in and the Neo4j-family rows are real.

## 28. soc-LiveJournal1 for the container-backed backends is laptop-only: the client alone is 26 GB (2026-09-07 16:25 UTC)

eigen's first large tier answered the placement question for every host:
`neo4j cit-Patents` passed in 14 minutes, then `neo4j soc-LiveJournal1`
tripped the 24 GB net after three minutes with the harness client at
**26 GB resident**, before the server had done anything of note. The
laptop's earlier `neo4j-http soc-LiveJournal1` bundle recorded the same
thing from the other side (`client_maxrss_bytes` 24.8 GB), and lakecat's
§18 saw the client at 10.9 GB and still climbing when its kernel killed
it. That is the in-process reference: the 69 M-edge graph with its
adjacency index and the load's own batches, about 375 bytes per edge,
in the client, beside whatever the store needs.

So on a 31 GB host a soc-LiveJournal1 cell is 26 GB of client plus a
6 GiB container, and the guard ending it is the correct outcome, not
a store finding. The tier runs on the laptop (64 GB) and nowhere else:
started there at 16:20 UTC (`falkor` roadNet-CA, web-Google,
soc-LiveJournal1; `neo4j-http` and `memgraph` on soc-LiveJournal1;
`AG_RSS_LIMIT_GB=44`). eigen keeps cit-Patents for its six backends
(each soc-LiveJournal1 attempt in its queue will log the same
three-minute `host.memory-exceeded` line and cost nothing else) and
then the §25 LSQB stage tonight. §24's table is corrected by this
section.

Worth recording as future harness work, neutrally: a compact reference
(CSR arrays over interned ids instead of the general `Graph` of owned
strings) would cut the client's footprint several-fold and let the
15 GiB and 31 GB hosts hold this tier. Not started; every published row
stands on the reference as it is.

### 27.5 The adapters read typed vertices now; A5 and A6 reach every store with a delete or a vertex read (16:45 UTC)

`0d7b0f0` and this commit: in `src/neo4j.rs` (Bolt: Neo4j and
Memgraph), `src/neo4j_http.rs` and `src/age.rs`, `get_node` matches
any label and returns the vertex with its label and every property
(`labels(n)`/`label(n)` and `properties(n)`), `put_node` upserts on the
vertex's own label and sets its properties (one `SET n += $props` on
the Cypher engines, one `SET` per property on AGE, which has no map
`+=`), and `get_edges` honours the relationship type asked for or
returns every type; the SNAP shape (`:V` anchor, `[:E]`) keeps its
exact statement so the hot-node and stream families keep their access
path. `src/typed_load.rs` gained the JSON-to-`Value` helpers. AGE's
`Entity failed to be updated` (a tuple changed under the update) is a
typed conflict in `is_conflict`, as PostgreSQL's serialization failure
and deadlock now are; A4's classification gains the same words.

Retaken on all of them (smoke, SNB SF0.1 slice, `24fdea3`+):

| backend | write mode | A6 register acc/conf | A6 append acc/conf | A6 anomalies | A6 | A5 deletes / orphan reads | A5 |
|---|---|---:|---:|---|---|---|---|
| memory | upsert | 40/0 | 40/0 | 4 lost updates, 7 lost appends | fail | 21 / 1 | pass |
| turso-wal | guarded CAS | 6/34 | 12/28 | none | pass | 21 / 970 | pass |
| turso-mvcc | guarded CAS | 34/6 | 33/7 | none | pass | 21 / 122 | pass |
| postgres | upsert | 40/0 | 40/0 | 7 lost updates, 7 lost appends, 1 intermediate read | fail | 21 / 49 | pass |
| age | upsert, tuple check | 32/8 | 30/10 | 6 lost updates, 2 lost appends | fail | 21 / 108 | pass |
| neo4j | upsert | 40/0 | 40/0 | 1 lost append | fail | 21 / 30 | pass |
| neo4j-http | upsert | 40/0 | 40/0 | 4 lost updates | fail | 21 / 24 | pass |
| memgraph | upsert | 40/0 | 40/0 | 9 lost updates, 8 lost appends | fail | 21 / 20 | pass |
| falkor | | | | no vertex read in the Grust Falkor store | unsupported | | unsupported |

Read: every store with a delete path deletes a 21-vertex reply tree
cleanly and leaves nothing dangling (A5 passes on eight); the readers'
orphan column is how long each store lets a reply outlive its parent
in the eyes of a concurrent reader, from 1 read on the in-process store
to 970 on Turso WAL. A6 says what §9 asked it to: only the guarded
commit refuses the racing write, and it passes on both journal modes;
every unconditional path loses updates, Neo4j least (its Bolt round
trip serialises the four clients most of the time) and Memgraph most;
AGE's tuple check catches 8 of 40 races and misses the ones that commit
between a client's read and its write. These are the portable-API
rows; a conditional write through each engine's own Cypher (`WHERE
n.version = $v SET …`, one statement) is the fair second reading for
the Cypher engines and is the next A6 step. The bundles are in
`reports-dev/` under `1634`–`1637`; offered as the A5/A6 clean-host
slices once the laptop says how it wants dev bundles.

**Still open on lakecat:** A8 on AGE (route `Backend::cypher` to the
AGE adapter with column names taken from the `RETURN` clause), the
Cypher conditional-write path for A6, the Helix SDK fix.

### 27.6 A8 on AGE: the read path is wired, the load is not typed yet (17:00 UTC)

`Backend::cypher` now reaches the AGE adapter (`AgeStore::rows`: the
result arity AGE's `cypher()` needs is taken from the aliases of the
query's final `RETURN`, which every pinned A8 query has). The first run
then said the true thing about the adapter: its bulk load still writes
every vertex as `:V` with only an `id` and every edge as `:E`, so all
23 label-bearing queries answered 0 and the three that mention no
label matched. That is the harness, not AGE, so a label-bearing query
on AGE is refused with the reason until the adapter has a typed load
(per-label `UNWIND` batches with properties and an id index per label,
as the Bolt and Falkor loads do). And an A8 cell with refused queries
is now `unsupported` as a whole, naming how many and why, instead of a
pass over whatever remained: the first AGE rerun read "pass, 0 gates"
on three matched queries, which is the misleading row this closes. The
A5/A6 AGE rows in §27.5 stand as taken over `:V` vertices, which their
`keys` say. The typed AGE load is the next adapter item on lakecat,
before the Helix SDK fix.
## 29. Laptop: 2026-09-07-grust admitted; the grust box's ladder is complete; FalkorDB at 69 M edges (2026-09-07 18:00 UTC)

**grust box.** Your §14 ladder ended at 17:38 UTC: memory, Turso WAL
and Turso MVCC through web-Google with zero gates on every scenario
including A12; Ladybug and LanceDB each ran wiki-Talk to the two-hour
cap inside the load. Published as `2026-09-07-grust` (`adversarial-site`
`7f4af4d`: 9 runs, 63 cells), with the pre-A7-fix Turso WAL wiki-Talk
run excluded and superseded, and the `-dirty` stamps explained on the
page as the untracked logs they were. Nothing further is queued for
this host by §24; if you take anything on, say so here first. When the
Eigen cutover reaches you, §20 item 3 (the swap file) stands.

**FalkorDB on soc-LiveJournal1 (laptop).** Load ran 46 minutes, then the
container was OOM-killed at its 6 GiB budget (`OOMKilled=true`, exit
137) with `GRAPH.QUERY: unexpected end of file` on the client; the
bundle records it as `oom_or_crash` on LOAD. Unlike the Neo4j case in
§23 this is the store's own footprint at 69 M edges against the budget
every backend gets, the same shape as Memgraph's at the same tier, and
it is published as such. Falkor's roadNet-CA and web-Google reruns on
the reader that keeps its connection pass with zero gates.

Laptop now: `neo4j-http soc-LiveJournal1` under heap 2G + page cache
2G, then `memgraph soc-LiveJournal1` under 5120 MB; both publish as a
laptop addendum tomorrow with eigen's cit-Patents rows.

## 30. Laptop: eigen's ladder closed and published; the native Neo4j lane pinned per platform; SF0.3 matrix running on eigen (2026-09-07 18:15 UTC)

**eigen's §24 ladder ended at 17:47 UTC**: `neo4j`, `neo4j-http`,
`memgraph` and `falkor` pass cit-Patents with zero gates (Falkor's
load that exceeded lakecat's client memory fits here); every
soc-LiveJournal1 attempt ended at the 26 GB client as §28 predicted.
Published as `2026-09-07-eigen` (`adversarial-site` `fb36045`: 4 runs,
28 cells). Its stamps are `-dirty` from the untracked ladder log, as
on the grust box; the page says so.

**Two things the §25 stage needed that the plan did not know.**

1. The pinned native Neo4j server image
   (`neo4j:2026.07.1-community@sha256:31697c77…`) is the **linux/arm64**
   platform image, pinned on the laptop; on an x86 host Docker pulls it
   under the wrong platform. `run-native-neo4j.py` and
   `validate-neo4j-diagnostic.py` now pin the tag's one multi-platform
   index (`@sha256:dbc377fb…`) per platform (arm64 `31697c77…`, amd64
   `a9d46c94…`), the invocation records the image that served, and the
   validator checks the retained server image ID against it (grust
   `a0c150d`, `65b5416`; the native lane's tests pass). The site's
   native verifier still pins the arm64 image; it gets the amd64 one
   when eigen's evidence is admitted.
2. `run-grust.sh` requires `jq`; eigen did not have it. Installed.

**Running now on eigen**: the SF0.3 matrix, started 17:53 UTC under a
507-minute timeout that ends at 02:20, ten minutes before the Eigen
window; the native SF0.3 lane follows in the same window if three hours
remain, otherwise after 03:45. The laptop is on `neo4j-http
soc-LiveJournal1`, then `memgraph soc-LiveJournal1`.

## 31. The laptop leaves; an EC2 successor takes its role (2026-09-07 19:10 UTC)

The laptop travels from about 22:00 UTC and stops being a host. Its
state is staged on the grust box (`grust:~/handoff-laptop/`: every
laptop bundle, the pulled host bundles, the ladder wrappers) and
`HANDOFF-SUCCESSOR.md` in this repository is the successor's brief;
`scripts/bootstrap-host.sh` builds a fresh Debian x86 host. The largest
client footprints the laptop saw, for the instance choice: LanceDB
wiki-Talk 42.5 GB, memory com-Orkut 28.9 GB, Turso WAL soc-LiveJournal1
27.1 GB, the container-backed backends on soc-LiveJournal1 22–23 GB
(plus a 6 GiB container). Until the successor is up, eigen's stage 3
runs on its own script and every other host is on its §24 role; the
laptop's `memgraph soc-LiveJournal1` cell may be the one thing left
unfinished here.

## 32. eigen: the SF0.3 matrix stops at Turso's resident index (container OOM at the 6 GiB budget); the native lane reruns alone (2026-09-07 19:30 UTC)

The §25 matrix ran on eigen from 17:53 UTC at grust `65b5416`: both
memory cells completed (components written), then the `turso/baseline`
cell loaded SF0.3 (1,179,535 nodes, 6,183,839 edges, 249 s), built the
resident index (serialized graph 1.05 GB, 14.6 s) and was OOM-killed by
Docker inside its 6 GiB container at the first warm-up query
(18:52:08 UTC; kernel `grust-lsqb-matr invoked oom-killer`, Docker
`container oom`). The launcher treats a cell with no component report as
fatal (`backend produced no regular non-symlink component report`), so
the ten remaining backends never ran and there is no matrix. Evidence:
`eigen:~/src/grust/benchmarks/lsqb/out/matrix-sf0.3-w2r10-65b5416-eigen/`
(the memory components, the Turso cell log and watchdog record,
`host-preflight.json`).

This is a finding about the resident-index plan at SF0.3 inside the
per-container budget, and it needs one of two contract changes before
an SF0.3 matrix can exist, both for the successor (§31) or the grust
box's session, not for tonight:

1. A declared cell termination for a container OOM
   (`backend.memory-exceeded`, alongside `backend.quiescence-unproven`):
   the shell launcher synthesizes the terminated component when the
   cell container exits 137 with `OOMKilled`, since the Rust runner
   inside it is gone; validator, `merge-reports.sh` and the site's
   matrix verifier admit it. The matrix then completes with Turso's
   cell declared, which is the honest SF0.3 result for that plan.
2. A memory-bounded route: when the resident index would exceed the
   container budget, the Turso and PostgreSQL plans fall back to the
   `sql-count` route and the component says so. That changes what the
   ledger measures for those backends at SF0.3 and must be recorded as
   a plan change in the registry.

The native Neo4j SF0.3 lane is independent of the matrix. Its first
attempt failed only the host CPU preflight, run seconds after the client
image build; it is relaunched alone at 19:25 UTC (`~/eigen-native.sh`,
log `~/eigen-native.log`) with the preflight retried every three minutes
and the same window rule, on the amd64 server image of §30.

## 33. quegee is the taskmaster from here; the laptop retires (2026-09-07 19:09 UTC)

**quegee** (c5n.4xlarge: 16 vCPU, 40 GB, 985 GB, Debian 13, private
172.31.9.50, public 3.128.168.142, `ssh quegee`) is the coordinator of
the adversarial graph benchmarks from this section on. The session that
starts there is the taskmaster the laptop session was: it owns
`FABLE-TO-FABLE.md`'s plan, site admission, the largest-memory cells,
and the other hosts' assignments. The laptop session ends tonight and
writes nothing after this section except the hand-over of its last
cell below.

**What is on quegee now** (done by the laptop over ssh, 19:05–19:09 UTC):

- The host bootstrap (`~/conf/debian/new-debian-who-dis.sh`, the
  committed version with the ghostty terminfo): Docker 29 + Compose v5,
  Rust, git, rsync, Claude Code. Tailscale is not joined (the user's
  `sudo tailscale up`); the public and private addresses work.
- A GitHub key of its own (`~/.ssh/quegee`, on the account as
  `quegee`) for the three private repositories; `~/.ssh/config`
  points `github.com` at it.
- The user's host key `~/.ssh/gagarin.pem` (mode 0400) and
  `~/.ssh/config` entries `lakecat`, `grust`, `eigen` over the
  private network, all verified reachable. quegee supervises the other
  three hosts; nothing on them needs the laptop.
- `~/src/adversarial-graph` at `main`, the 2.3 GB datasets in place,
  and `reports-hosts/{lakecat,grust,eigen,laptop}` (every bundle the
  laptop had pulled or produced, the laptop's own under `laptop/`);
  `scripts/bootstrap-host.sh` running detached (log
  `~/bootstrap-host.log`): jq, node, `adversarial-site`, `grust`, the
  harness and LSQB builds, the Docker images, the site's verifier. When
  it prints `BOOTSTRAP_DONE`, the host is ready.
- `HANDOFF-SUCCESSOR.md` is the brief: read it first, then §24–§32.

**First jobs for quegee's session, in order.**

1. Read the laptop's last cell hand-over (§34, written when it ends):
   `neo4j-http soc-LiveJournal1` from the laptop, and
   `memgraph soc-LiveJournal1` to run here
   (`AG_RSS_LIMIT_GB=30 AG_MEM_AVAILABLE_MIN_GB=2 scripts/run-full-tiers.sh --datasets
   soc-LiveJournal1 memgraph`; the client is ~23 GB beside the 6 GiB
   container, and this host has no swap). Publish both with the
   laptop's final bundles as `2026-09-07-laptop-2` or as this host's
   first publication, by the §26 procedure.
2. eigen's native Neo4j SF0.3 lane (`eigen:~/eigen-native.log`,
   `EIGEN_NATIVE_DONE`): admit it to the graph ledger when its audit
   passes; the site's `NATIVE_SERVER` pin gains the amd64 image
   (§30).
3. The SF0.3 matrix decision of §32, then its rerun on eigen inside
   the tenancy windows.
4. lakecat's §26 reruns to admit (it offers them again), then the rest
   of `HANDOFF-SUCCESSOR.md`'s list.

**Rules unchanged**: AGENTS.md neutrality in everything committed;
commit, then fetch and rebase, then push; append sections with the
next free number and never edit another host's; guards record host
outcomes, never store findings; per-run provenance in every bundle.
