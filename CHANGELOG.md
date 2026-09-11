# Changelog

## Unreleased

- Grust repinned to 2d447ff for three adapter defects the full tiers
  exposed on every host (2026-09-10): the Surreal relate path had no index
  over a relation's `(in, out)`, so its idempotent delete-then-RELATE was a
  table scan per edge and every full-tier surreal-http load died in the
  client's minute-long request timeout after ~300 s of O(E²) work; Surreal
  node reads by ID were an OR-chain that SurrealDB's parser refuses past a
  few hundred terms ("Exceeded expression recursion depth limit"), which
  took surreal-sdk's A1 and A2 down at 4,039 nodes; and the Helix SDK
  adapter's errors hid the server's text. The harness sets the Surreal
  request timeout to ten minutes (a bound on a stalled server, not on a
  batch).

- The helix-sdk backend gets its own server. `helix-db` 3.0.0 posts a
  nested query AST to `/v2/query`; the enterprise-dev image the `helix`
  service runs serves `/v1/query` only and answered every SDK request with
  400 "missing field `queries`", which was the whole of "Helix SDK
  replace/drop failed" on every host. Compose service `helix-sdk` (port
  18083) runs the standalone HelixDB server at the revision the SDK grew up
  with, built from source on the host by scripts/build-helix-sdk-server.sh
  (the sibling LSQB harness's qualified recipe, on amd64); the ladder maps
  helix-sdk to it and probes /healthz and /readyz; AG_HELIX_SDK_URL
  overrides the address; the backend's container record is its own.

- The families run only after a load that passed: a load that was refused
  or not attempted skipped nothing before, and PostgreSQL's unsupported
  sx-stackoverflow load was followed by every family against an empty
  store (6,444 gates, 2026-09-11).

- The load budget can end a load whose adapter never yields: the load runs
  on its own task and the budget's timer races it. Wrapped around the
  adapter's own future it could not fire (the in-process store's load is
  synchronous; a Bolt batch under memory pressure holds for hours), and on
  2026-09-11 Neo4j at com-Orkut ran 6 h 20 min past a six-hour budget. A
  task the timer beats is aborted; the run ends with an explicit exit so a
  thread still blocked in the adapter cannot hold the process. Checked: a
  24 s in-process load under a 2 s budget is a LOAD row with the gate at
  2,001 ms and the process ends in 21 s.

- A multigraph refused by a structural edge key is `unsupported`, not a
  crash: the Grust SQL adapters upsert an edge on (from, label, to), and
  PostgreSQL rejects a batch that repeats the key, so sx-stackoverflow's
  parallel edges are not preserved on that route. The load row says so.
  A load-failure note keeps the message's head and tail rather than the
  adapter's whole statement (20 KB of VALUES on that row).

- The load has its own budget (`AG_LOAD_BUDGET_S`, the ladder passes its
  `--cap`), separate from the families' time: a load that does not finish
  inside it is a LOAD row with the `hang_or_timeout_without_refusal` gate
  and the families do not run; a store that loads inside it gets the
  families under a pair timeout of twice the cap. Before, one cap covered
  both, and at com-Orkut it ended Turso WAL's and LanceDB's first
  traversal after their loads had finished.
- A store whose measured rate on this host projects the tier past the load
  budget is not sent to spend it (`budget::measured_rate`, from the host's
  own passing LOAD rows, the largest tier first): a `not-tested` LOAD row
  with the projection, the rate and the run it came from, and no larger
  tier after it. `AG_PREDICT_LOAD=0` sends the store anyway, for the
  deliberate measurement of a load past the budget.

- A load failure or a crash gate on a containerized backend records the
  container's own state in the row's notes (`GET /containers/{name}/json`):
  "container adversarial-graph-neo4j-1: exited, exit 137, OOMKilled". The
  kernel taking a store at its memory limit read as a transport error
  before (neo4j-http at soc-LiveJournal1 on eigen, 2026-09-10).

- The compact reference now loads its edges into the Bolt, HTTP and
  FalkorDB stores. Their loaders group edges by the labels of the vertices
  in the same batch and skipped an edge whose endpoint the batch did not
  carry, which is every edge of the compact loader's edge-only chunks: on
  2026-09-10 com-Orkut, soc-Pokec and soc-LiveJournal1 loaded every vertex
  and no edge into Neo4j while LOAD read "pass" and A1 found zero
  neighbours. An absent endpoint now resolves to the untyped label `V`
  (`typed_load::LoadPlan`). A load the store reports short of the loader's
  node or edge count is a `lost_write` gate, and the conformance probe
  "compact edge chunks are shaped for this adapter" fails on "accepted but
  read back 0 edges" instead of passing it. The bundles from that window
  are set aside as `reports-void-20260910/` on each host.
- Two more untyped formats through one edge-pair source
  (`dataset::pairs`): a Matrix Market coordinate file inside a SuiteSparse
  tarball (GAP-road; `symmetric` expands to both directions, as SNAP's road
  networks list theirs), and SNAP temporal edge lists (`sx-*`: `from to
  timestamp`), whose repeated pairs are parallel edges kept as such and
  counted in the new `parallel_edges` load stat -- the multigraph is the
  tier's pathology. Both loaders, materialized and compact, read the same
  source, so a row's load stats do not depend on the path.

- A8 records a store's own declared resource cap as a refusal, not a
  crash: Memgraph's `--memory-limit`, Neo4j's transaction memory pool,
  FalkorDB's `QUERY_MEM_CAPACITY`, matched on the store's typed message
  (`differential::is_declared_limit`). The store stopped the query, said
  so, and is still up, exactly as Grust's bounded-read cap is recorded on
  the in-process and SQL routes; the cell stays unsupported past it. A load
  that ends at a store's limit is still a failing row.

- A8 never issues a query to a store that is still executing the last one
  (Astra finding 1). FalkorDB is handed the store budget as its own
  `TIMEOUT` and stops the query itself; after any query the harness stopped
  waiting for, every store gets a trivial probe, waited for up to the
  reference budget, and if it does not answer the remaining queries are
  recorded as `not-attempted` with the hung query named, one hang gate for
  the hang and none for them. A store's own deadline error is a timeout,
  not a crash. Falkor at LDBC sf1 had recorded 28 hang gates behind one
  slow count; Memgraph at sf0.1 four behind two.

- A8 runs in a fraction of the memory: the oracle index is built over the
  loaded graph itself instead of a copy of it (`LoadedGraph::Full` is
  shared); on Turso and PostgreSQL the one pinned shape the SQL planners
  refuse (ICIJ c4, `WHERE o <> p`) runs the reference executor over the
  store's resident snapshot under the store budget's policy (110 s, 2 GiB
  of intermediates) instead of the adapters' own fallback, which read the
  whole graph out of the store again for every such query and ran the
  executor unbounded and synchronously (a 2 h hang on one host, a 32 GB
  kernel kill on another); and ICIJ r2, r3 and r4 join the native answer
  key, each of which had exceeded 2 GiB of intermediates and taken 13-23 s
  in the executor. The route each store took is recorded as before.
- The five remaining LDBC row shapes (r1 knows pairs, r3 tag popularity, r4
  female persons, r6 countries distinct, r7 knows unordered) join the native
  answer key: at sf1 the executor on r3 took the client past a 22 GB host
  guard on top of a 14 GB graph. Every row shape of both schemas is held
  against the executor on a small graph with null properties, duplicate
  group names and edges of the right type from the wrong label.

- A8 backend errors and timeouts now fire hard gates, and missing reference
  coverage prevents a pass. A failed gate takes precedence over an unsupported
  operation in the cell headline; refusal details remain in the notes.

- Two external adapters: `memgraph` (Memgraph 3.12 over Bolt through the
  Neo4j store with a `BoltDialect` for the session database and index DDL)
  and `age` (Apache AGE 1.8 on PostgreSQL 18.6 through the `cypher()` table
  function, `src/age.rs`, feature `age`), both in the external compose
  profile and both ladders.
- A12 cold start and footprint: time to the first correct hub degree on a
  fresh handle, then an open-loop stream of one-hop reads at 50 and 200
  requests per second over sixteen handles with service and response time
  histograms (p50/p99/p99.9/max), late arrivals and wrong answers. The
  edges A4 appends to the hub are carried into A12's expected degree
  (`Ctx::hub_writes`), since both families run against one load.
- `scripts/run-full-tiers.sh`: one (backend, dataset) pair per `ag run`
  under a wall-clock cap, smallest dataset first; a backend that hits the
  cap is not tried on larger tiers.
- First clean-host results (dedicated 4-vCPU EC2 host, load average ≈1)
  for all thirteen backends; ADVERSARIAL-GRAPH.md §7 gains the clean-host
  eight-way table, the HTTP-versus-SDK transport pairs, and four new
  findings: SurrealDB's `get_nodes` OR-chain exceeds the parser's expression
  recursion limit on wiki-Talk's hub (both transports), Ladybug loads at
  ≈11 edges/s through per-row statements, HelixDB's adapter writes each edge
  as two `NWhere id = …` node scans so a 500-edge batch outruns the gateway's
  30 s request timeout (408) on the 200k slice, and the Helix SDK read path
  rejects the server's response envelope (`unknown variant \`Read\``).
- Record the server profile a row was taken under (`profile` observation,
  FalkorDB `resultset_size=…`) and the edge slice, and key `RESULTS.md` on
  both, so the truncating and tuned FalkorDB runs and the 10k and 200k Helix
  runs no longer overwrite each other.
- Create a Helix runtime equality index on `(V, id)` at bootstrap, as the
  harness does for FalkorDB and Neo4j. The server accepts it, but the 200k
  load still times out, so whether `NWhere` uses runtime indexes is open.
- Link on Linux with the `ladybug` feature: `build.rs` passes
  `--allow-multiple-definition` for the binary only, because the prebuilt
  `liblbug.a` bundles zstd (and simsimd) objects that the Lance crates also
  link through `zstd-sys`; macOS ld64 silently took the first copy, GNU ld
  and lld refuse. Pin the `helix` service to the image's multi-arch index
  digest instead of its arm64 manifest, so the same build resolves on x86.
  `scripts/render-results.py` defines its backend order before use and adds
  a `Host` (arch/vCPUs) column so contended-laptop and dedicated-host rows
  are distinguishable.
- Add LadybugDB (embedded, Grust's `grust-ladybug` adapter over `lbug`
  0.20.2) and HelixDB (Grust's `grust-helix` adapter, HTTP and `helix-db`
  SDK) as backends; both adapters are `publish = false` in Grust and are
  pinned to the `v0.13.0` tag by a `git` dependency, with `grust-core`
  patched to the same tag so registry and tagged crates share one
  `GraphStore`. `compose.yaml` gains a digest-pinned `helix` service.
- Run every system that offers both an HTTP API and a Rust client as two
  backends: `surreal-http`/`surreal-sdk`, `helix-http`/`helix-sdk`,
  `neo4j` (Bolt)/`neo4j-http` (HTTP Query API v2, `src/neo4j_http.rs`).
  LOAD rows record `transport`; `ag backends` prints it.
- Add `ADVERSARIAL-GRAPH.md`: research findings on the Graph Data Council
  (LDBC) benchmarks and audits, GAP/Graph500, the graph-database testing and
  Jepsen literature, and the tail-latency methodology; the pathology-driven
  dataset ladder; twelve scenario families mapped to stack layers, choke
  points, and hard gates; and the harness milestones.
- Add `scripts/fetch-datasets.sh` with a SHA-256 manifest for the S/M tiers
  (15 datasets, 2.2 GB) and an opt-in L tier (twitter-2010, Friendster, GAP).
- Add the `ag` harness (M1): SNAP edge-list loader, `GraphIndex` oracle,
  memory and Turso (WAL/MVCC) backends through `grust-graph` 0.13.0, scenarios
  A1 fan-out, A2 deep paths, A3 policy bounds, A4 hot-node contention, and A7
  guarded-commit replay, and the hard-gate report contract. Smoke run on
  wiki-Talk, roadNet-CA, and web-Google: 36 pass, 9 unsupported, 0 gates.
