# Task for the EC2 session: a resident typed index for durable Grust stores

Written 2026-09-06 03:50 UTC by the laptop session. Scope: `~/src/grust`
(pull `origin/main`, currently `af2efa8`; history was rewritten on
2026-09-05, so use `git fetch origin && git reset --hard origin/main` on an
existing clone). The laptop must not touch its grust worktree until the
running SF0.1 matrix finishes, so this work belongs on the EC2 host.
Neutral framing throughout, per `AGENTS.md`.

## Why

The SF0.1 matrix run of 2026-09-06 (`benchmarks/lsqb/out/matrix-sf0.1-w2r10-af2efa8-c1`)
shows the Memory cell running all 22 LSQB cases through the new count plans
(medians 6 to 152 ms), while the durable backends route those counts to SQL:
Turso q4 10.4 s, PostgreSQL q4 13.6 s, q1 a 60-second timeout on both, and
seven of nine baseline queries `unsupported` because whole-store
materialization per query is refused at downloaded scales. SQL planners do
not do a nine-way join count inside the envelope. Embedded engines solve this
with a resident in-memory adjacency beside the durable store, built once and
kept warm. `MemoryGraphStore` already has that mechanism; the durable stores
do not.

## What exists

- `crates/grust-core/src/typed_graph_index.rs`: the immutable typed
  adjacency (`TypedGraphIndex`), dense/sparse per relationship type.
- `crates/grust-memory/src/indexed_snapshot.rs`: the cached snapshot on
  `MemoryGraphStore`, built lazily, shared through `Arc`, invalidated on
  every write; exact serialized-size caching for the bounded API.
- `crates/grust-cypher/src/read/indexed.rs` and `read_policy/`: the indexed
  read entrypoint that runs the count plans over a snapshot with the policy
  budgets charged.
- `benchmarks/lsqb/src/backend/{mod.rs,execution_plan.rs}`,
  `src/matrix_catalog.rs`, `src/report/execution_plan.rs`,
  `evidence-manifest-v2.json`, `EXECUTION-PLANS.md`: how a backend declares
  its execution class per query, how the worker reports the plan it ran, and
  the hash-bound plan registry that admits non-materializing plans past the
  1,000,000-row gate.
- `docs/INDEXED_READS.md`: the API contract for indexed reads.

## What to build

1. **`TursoGraphStore` resident snapshot.** Add the same cached,
   write-invalidated `IndexedSnapshot` that Memory has: built once from a
   full typed read of the store (nodes by label, edges by type) after
   `bootstrap`/load or on first indexed read, shared through `Arc`, dropped
   on any mutation. Expose it through the same indexed entrypoint so the
   existing count plans run unchanged. Measure and record build time and
   resident bytes for SF0.1 and SF0.3; that cost belongs in the load
   interval or worker setup, never in query time.
2. **Same for `PostgresGraphStore`** once Turso works; the read-all path
   differs, the snapshot and plans do not.
3. **A new execution class in the harness**, declared per backend and per
   query, for "durable store with a resident index built outside the query
   boundary". Suggested id: `backend-resident-index-rust-count`. It must be
   added to the class enum, the backend declarations, the plan registry, the
   Python validator (`validate-matrix-publication.py`) and the site verifier
   (`~/src/adversarial-site/scripts/verify-graph-matrix.mjs`, `allowedClasses`
   per backend), with tests in each. It is a distinct class, disclosed as
   such; it is not a relabeling of the Memory reference and not
   "backend-native".
4. **Row-limit exemption** applies to admitted non-materializing plans in
   the new class exactly as it does for Memory, through the plan registry,
   never by query name.
5. **Worker lifecycle.** Turso is process-owned, so each observation worker
   reloads before READY. The snapshot build therefore happens in
   `setup_ns` per observation; the coordinator's `WORKER_READY_TIMEOUT_MS`
   for SF0.1 needs the large-tier value (1,200,000 ms). If the build makes
   per-observation setup impractical at SF0.3, note it in the ledger; do
   not move the build inside the timed boundary.
6. **Differential validation.** Every count from the Turso resident-index
   plans must equal the Memory cell's count and the upstream oracle on
   `sfexample`, SF0.1 and SF0.3 before any timing is taken. Add the
   Turso cases to the existing count-plan integration tests.
7. **Resume mode for `run-grust.sh`.** A publication currently requires all
   twelve cells in one supervised run, so a change to one backend costs a
   full matrix. Add `RESUME_FROM=<prior OUTPUT_DIR>`: for every cell whose
   component, log and watchdog record exist in the prior directory, were
   produced at the same source revision and image identities, and verify
   against their recorded hashes, copy them in and skip execution; the
   receipt lists reused cells with the prior directory's identity. Anything
   that fails verification runs fresh. Tests for the validator and for the
   site verifier so reused cells are distinguishable in the receipt.

## Measurement plan

- Diagnostic first, on the EC2 host: one Turso cell at SF0.1 with
  `DISCOVERY=1` to see setup and query times; not publishable.
- Then one full SF0.1 matrix on the laptop, or with resume mode, only the
  Turso cells plus any others that changed, producing a new receipt at the
  new revision. The Memory, Neo4j, upstream and Falkor numbers do not change.
- Record in `docs/GRUST_SPEED_PROGRESS.md`; the site gets a new dated
  publication, not an edit of the existing one.

## Coordination

- Commit to `grust` `main` and push as you go; the laptop is not committing
  to grust until its matrix run ends and will pull after.
- The laptop matrix run's Turso and PostgreSQL cells at `af2efa8` are the
  pre-change baseline; keep that directory.

## Follow-ups from the first SF0.1 matrix with the resident index (2026-09-06, laptop, revision 68d1b09)

Observed in the running `baseline-turso` cell (measurement iteration 3):

| Turso at SF0.1 | q1 | q2 | q3 | q4 | q5 | q6 | q7 | q8 | q9 |
|---|---|---|---|---|---|---|---|---|---|
| route | sql-count | resident | resident | sql-count | resident | resident | resident | resident | resident |
| result | timeout 60 s | 176 ms | 9 ms | 10.5 s | 127 ms | 5 ms | 132 ms | 125 ms | 14 ms |

1. **Route preference.** When both a `sql-count` registry entry and a proven
   `count-factorized` plan exist, the worker takes the SQL count. q1 and q4
   are the two slowest Turso queries as a result, while the resident plan
   would run them in well under a second (Memory: 52 ms and 124 ms). Prefer
   the resident plan whenever it is proven, and keep `sql-count` as its own
   declared class for the cases without one. The same applies to PostgreSQL.
2. **Per-observation setup cost.** Turso is process-owned, so every
   observation worker reloads the store from the CSVs and rebuilds the index
   before READY: 59 s per sample at SF0.1. Twelve samples for each of 22
   admitted cases makes the two Turso cells about five hours and the
   PostgreSQL cells similar, so a full SF0.1 matrix is now a twelve-hour run.
   A prebuilt database file copied into each worker's private directory keeps
   the fresh-process recovery proof and turns that setup into seconds; the
   copy, like the load, stays outside the query boundary. Record whichever
   choice is made in the lifecycle so the receipt says how the worker got
   its state.
3. Until both land, `RESUME_FROM` (at the same revision) is the way to avoid
   paying the twelve hours again for a one-backend change.
