# Changelog

## Unreleased

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
