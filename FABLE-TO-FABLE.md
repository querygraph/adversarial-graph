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
