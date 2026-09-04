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
```

Reports land in `reports/<timestamp>/report.json`. Nine hard gates must all be
zero for a `pass`; quality and latency (HdrHistogram percentiles) are reported
separately and never averaged into a score. The exit status is non-zero when
any hard gate fires.

The harness depends only on the published `grust-graph` 0.13.0 crates; it
never reads a Grust checkout. Additional backends are feature-gated
(`--features postgres,surreal,falkor,…`) and reach their services through
the digest-pinned `compose.yaml` (milestone M2).
