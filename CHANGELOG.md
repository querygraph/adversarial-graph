# Changelog

## Unreleased

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
