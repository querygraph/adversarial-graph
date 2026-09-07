# adversarial-graph

**GRAPH-ADVERSARIAL-v1** — an adversarial benchmark for graph stores that
exercises every layer of the QueryGraph stack (storage engine → Grust
`GraphStore` → Cypher/GQL with the bounded read policy → guarded commits →
governed memory → LakeCat catalog projection → semantic answers → lineage)
under skew, depth, density, unbounded patterns, hot-node write contention,
ambiguous outcomes, and restarts.

The research findings, dataset rationale, scenario families, and plan are in
[`ADVERSARIAL-GRAPH.md`](ADVERSARIAL-GRAPH.md).

## Quick start

```sh
scripts/fetch-datasets.sh              # tiers S and M (~2.2 GB) -> datasets/, MANIFEST.json
cargo build --release
./target/release/ag datasets
./target/release/ag run --smoke --dataset wiki-Talk,roadNet-CA,web-Google
./target/release/ag run --dataset wiki-Talk --backend memory,turso-wal,turso-mvcc
./target/release/ag run --dataset ldbc-snb-sf0.1,icij-offshore-leaks --limit-edges 200000   # typed graphs (ADVERSARIAL-GRAPH.md §2.4)
```

Reports land in `reports/<timestamp>/report.json`. Nine hard gates must all be
zero for a `pass`; quality and latency (HdrHistogram percentiles) are reported
separately and never averaged into a score. The exit status is non-zero when
any hard gate fires.

The harness depends on the published `grust-graph` 0.13.0 crates, plus
Grust's two unpublished internal adapters (`grust-helix`, `grust-ladybug`)
pinned to the same release tag by a `git` dependency; it never reads a Grust
checkout. Additional backends are feature-gated
(`--features postgres,surreal,falkor,lancedb,helix,ladybug,neo4j,age`) and reach
their services through the digest-pinned `compose.yaml`. Systems with both an
HTTP API and a Rust client are two backends each (`surreal-http`/`surreal-sdk`,
`helix-http`/`helix-sdk`, `neo4j`/`neo4j-http`); Memgraph (`memgraph`) shares the
Bolt store with Neo4j and Apache AGE (`age`) is reached through PostgreSQL's
`cypher()` function; `ag backends` lists every
backend with its transport.
