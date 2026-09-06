# Ladybug notes from two Grust-side benchmarks

For Arun, from Alexy's benchmark sessions of 2026-09-04/05. Everything below
is measured; where I infer a cause I say so. Two harnesses produced these
numbers, and it matters which one:

- **LSQB, unmodified upstream scripts** (`ldbc/lsqb` at `242cb2f`, "Kuzu ->
  Ladybug", Python `ladybug==0.19.0`, 8 threads, in a container capped at 8
  CPUs and 6 GiB, five repetitions per scale, each in a fresh copy of the
  checkout). This is Ladybug itself, no Rust involved.
- **Grust's `grust-ladybug` adapter over the `lbug` 0.20.2 Rust crate**,
  driven by two harnesses: the LSQB matrix at example scale, and a strain
  harness (`querygraph/adversarial-graph`) on a dedicated 4-vCPU x86_64 host
  loading a 200,000-edge slice of SNAP wiki-Talk (max out-degree 12,215).
  These numbers measure the adapter's statement-at-a-time use of the binding
  as much as the engine; I separate the two below.

## 1. Native Ladybug on LSQB is fast; the loader is the headline

Whole five-repetition run, including `init-and-load.sh` each time, from the
supervisor's wall clock:

| Scale | Nodes / edges | Five loads + 45 queries |
|---|---|---|
| example | 28 / 72 | 0.1 min |
| SF0.1 | 432,235 / 2,080,404 | 0.3 min |
| SF0.3 | 1,179,535 / 6,183,839 | 0.9 min |

So a COPY-based load of 6.2 million edges plus nine queries takes well under
15 seconds per repetition. Per-query medians (ms, 8 threads):

| | q1 | q2 | q3 | q4 | q5 | q6 | q7 | q8 | q9 |
|---|---|---|---|---|---|---|---|---|---|
| example | 22 | 5 | 96 | 6 | 8 | 3 | 2 | 12 | 6 |
| SF0.1 | 667 | 36 | 229 | 79 | 84 | 253 | 284 | 174 | 404 |
| SF0.3 | 1807 | 72 | 734 | 37 | 371 | 1183 | 1130 | 494 | 1748 |

Things in that table worth a look:

- **q3 at example scale costs 96 ms on a 28-node graph** while its neighbours
  cost 2 to 22 ms, and at SF0.1 it is 229 ms. q3 is the five-`MATCH` triangle
  with a shared country. About 90 ms of that looks like planning, not
  execution; if the planner enumerates join orders for the five-clause form,
  that is where it goes.
- **q4 is faster at SF0.3 (37 ms) than at SF0.1 (79 ms)** across five runs
  each. Either a plan choice flips between the scales or thread scheduling
  dominates at this size; either way it is a reproducible oddity.
- q1 (nine-hop chain, 8.7 million matches at SF0.1) and q9 (two-hop `knows`
  plus anti-join) scale roughly linearly with data; q6 and q7 grow about 4.5×
  for a 3× graph.
- For scale, the LSQB repository's own `expected-output.csv` carries a
  reference timing column of 42 ms for q1 and 23 ms for q6 at SF0.1.

Also from the upstream track: **`init-and-load.sh` at this revision cannot
safely reuse a database path for a second initialization**, so the harness
has to copy the pristine checkout for every repetition. A loader that either
refuses cleanly or truncates would remove that workaround.

## 2. Through the Rust binding, statement at a time

Grust's adapter maps each label to a node table (`id STRING PRIMARY KEY,
props STRING`) and each edge label to a rel table, and writes with
`MERGE (n:T {id: $id}) SET n.props = $props` and
`MATCH (a:T1), (b:T2) WHERE a.id = $from AND b.id = $to MERGE (a)-[r:E]->(b)
SET r.id = $id, r.props = $props`, one statement per element, via
`Connection::prepare` + `execute`. That design is Grust's, not yours, but the
costs it exposes are in the binding and engine:

| Measurement | Result | Per element |
|---|---|---|
| Load 28 nodes + 72 edges (example, LSQB matrix) | 1.6 to 2.5 s | 16 to 25 ms |
| Load 145,172 nodes + 200,000 edges (wiki-Talk slice, dedicated host) | 5.1 hours, 10.9 edges/s, peak RSS 6.46 GB | ~90 ms |
| 100 single-edge MERGEs onto the 12,215-degree hub from 4 concurrent handles | 3.86 s; p50 37 ms, p99 974 ms, max 1.95 s; all 100 accepted, degree exactly 12,315 after | |
| Read back 100 elements by table (`MATCH (a:T)-[r:E]->(b:T) RETURN a.id, b.id, r.id, r.props`) | 150 to 270 ms | 1.5 to 2.7 ms |
| 2-hop neighbourhood of the hub, one `MATCH … WHERE a.id = $id` per frontier vertex | 19.5 s for ~12,000 statements | 1.6 ms |
| BFS to depth 8 from a fixed vertex | 13 ms | |

For comparison on the same host and slice, Turso (embedded SQLite-family)
loaded in 18 to 25 s and did the 100 hub writes at p50 24 ms / p99 61 ms;
LanceDB loaded in 12 to 15 s; the in-process reference took under a second.

What I read from this, as hypotheses for you to confirm or reject:

1. **A floor of roughly 16 to 40 ms per autocommitted statement**, visible
   even on an empty database (the example-scale load). That looks like a
   WAL flush per statement. If that is by design, the binding's docs should
   push people hard toward explicit `BEGIN TRANSACTION … COMMIT` batching or
   the Arrow/COPY path; the adapter has an Arrow IPC registration path
   (`register_arrow_ipc_node_table`) that this harness did not use, which is
   exactly the trap a new user falls into.
2. **Inserting an edge on a high-degree vertex costs O(degree).** The hub
   writes have a 26× spread between p50 and p99 and a 1.95 s maximum, and the
   5.1-hour load is dominated by wiki-Talk's hubs. Either the `MERGE`
   existence check walks the adjacency list, or the adjacency chunk is
   rewritten on each insert. A hub-heavy insert workload is a common shape
   (social and citation graphs) and is what turned a 200k-edge load into
   hours.
3. **Per-row result extraction in the binding costs about 1.5 to 2.7 ms per
   row** at small result sizes, which is far above the engine's per-row cost
   in the native numbers. Whether that is `Value` conversion, per-call
   locking, or query re-preparation, a batched or Arrow-shaped result path
   would help every embedding.
4. **6.46 GB resident for a 200k-edge graph** is presumably the buffer pool
   sizing itself to the host (15 GiB) rather than data, but it surprised the
   harness's memory probe; a note on how the default is chosen, and how to cap
   it from Rust, would save others the same surprise.

## 3. Packaging of the `lbug` crate

- `liblbug.a` ships its own zstd and simsimd with exported, un-namespaced
  symbols. Linking it into a binary that also has `zstd-sys` and `simsimd`
  (LanceDB does) produces duplicate-symbol warnings for `_ZSTD_createCCtx`,
  `_ZSTD_compress_usingDict`, `_ZSTD_CCtx_setParameter`, `_simsimd_dot_f32`,
  `_simsimd_l2sq_f32`, and about a dozen more. The link succeeded and the
  binary ran, but which copy wins is up to link order. Hidden visibility or a
  prefix on the bundled copies, or a feature to link the system libraries,
  would close it.
- The crate is 125 MB of source with a prebuilt-library download cache; that
  worked here on macOS arm64 and Linux x86_64. Note that the two tracks pin
  different versions: Python `ladybug==0.19.0` (from LSQB) and Rust `lbug`
  0.20.2 (from Grust). If 0.20 changed anything in the areas above, the
  native numbers in §1 predate it.

## 4. What is not in these numbers

- No native-Ladybug run of the adversarial LSQB attacks or of the strain
  scenarios exists yet; every non-upstream number went through the Grust
  adapter. A native `lbug` harness using COPY and Cypher directly would
  isolate the engine from the adapter and is the obvious next measurement.
- The laptop runs were on a shared, heavily loaded host and are excluded
  here; the strain numbers above are from the dedicated host at load
  average 1.
- Evidence: upstream LSQB bundles and the Grust example-scale Ladybug cells
  are published under adversari.al/graph (evidence directories
  `2026-09-04/upstream/*` and `2026-09-05/grust/sfexample/components/
  *-ladybug-*`); the strain run is `reports/20260905T093649Z` in this
  repository.
