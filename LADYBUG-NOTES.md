# LadybugDB under the strain harness: notes for the maintainer

Written 2026-09-06 from the `adversarial-graph` strain ladder
(`querygraph/adversarial-graph`), which drives LadybugDB through Grust's
`grust-ladybug` adapter over the `lbug` 0.20.2 crate. Everything below is a
measurement or a code path you can look at; where a number could be the
adapter's fault rather than the engine's, it says so. The adapter is not
yours, but several of its costs land on the engine's per-statement path, and
that is what you can act on.

## What ran

| | |
|---|---|
| Engine | `lbug` 0.20.2 (prebuilt static `liblbug.a`, x86_64 Linux) |
| Adapter | `grust-ladybug` at `querygraph/grust` `fdc685ee`, untyped mode, on-disk database under the run's work directory, `SystemConfig::default()` |
| Host | dedicated 4-vCPU, 15 GiB EC2 instance, Debian 13, no swap, nothing else running (load average ≈1) |
| Graphs | 200,000-edge slices of SNAP wiki-Talk (145,172 nodes, one vertex with 12,215 out-edges) and roadNet-CA (72,767 nodes) |
| Scenarios | LOAD, A1 hub fan-out, A2 depth-8 BFS, A4 one hundred concurrent single-edge writes to the hub, then the degree read back |

How the adapter drives the engine, so the numbers can be attributed:

- **Load** is `BEGIN TRANSACTION;` then one prepared statement per node,
  `MERGE (n:V {id: $id}) SET n.props = $props;`, and one per edge,
  `MATCH (a:V), (b:V) WHERE a.id = $from_id AND b.id = $to_id MERGE (a)-[r:E]->(b) SET r.id = $id, r.props = $props;`,
  then `COMMIT`. Tables are `CREATE NODE TABLE V(id STRING, props STRING, PRIMARY KEY(id))`
  and `CREATE REL TABLE E(FROM V TO V, id STRING, props STRING)`; `props` is
  `{}` for every row here.
- **Reads** go through the adapter's `get_edges`, then one prepared point
  lookup per neighbour, `MATCH (n:V) WHERE n.id = $id RETURN n.id, n.props LIMIT 1;`,
  re-prepared on every call.
- **Single writes** (A4) are the same edge statement, one per call, auto-committed.

## 1. Load: 11 edges per second

The wiki-Talk slice took **5 h 06 min** to load: 345,172 statements
(145,172 node MERGEs, 200,000 edge MERGEs) in 18,381 s, or **53 ms per
statement**, with the harness process at 0.90 CPU (one core busy the whole
time, so this is compute, not I/O wait). The roadNet-CA slice (272,767
statements) is loading as this is written; if the cost is per statement it
will take about 4 hours, and that run will be in the published bundle.

Two things about that number are the adapter's: it uses one statement per
row instead of the crate's bulk path (`Connection::create_arrow_table` and
the relationship equivalent behind the `arrow` feature, then `COPY … FROM`),
and it re-does a `MATCH … WHERE a.id = … AND b.id = …` for every edge. But
the per-statement cost itself is the engine's, and it is consistent across
two very different contexts:

| Context | Statement | Cost |
|---|---|---|
| Inside one 345k-statement transaction (LOAD) | node or edge MERGE with `SET … props` | ≈53 ms average |
| Auto-committed, one at a time (A4, 100 writes to the 12,215-degree hub) | edge MERGE | **37.5 ms p50, 974 ms p99** |

So a single-row `MERGE` of a relationship with a `SET` costs about 40–50 ms
of CPU on this machine whether or not it commits, and does not obviously get
worse as the transaction grows. For comparison, on the same host and slice
through the same adapter contract, SQLite-family Turso does the equivalent
upsert in 0.1 ms and PostgreSQL over the wire in 1.3 ms of server CPU. Worth
profiling: the `MERGE` existence check on a rel table (does it scan the
source's adjacency? the hub has 12,215 out-edges and the A4 p99 is 26× its
p50), and the `SET r.props = $props` string update path.

What would help adapter authors, regardless: a documented, obvious
in-process bulk-ingest example for Rust (Arrow tables in, `COPY FROM`),
since the crate's fast path exists but nothing steers a `GraphStore`-style
adapter toward it.

## 2. Point lookups: about 1.6 ms each

The hub's 1-hop read (A1) took **19.5 s** for 12,215 neighbours: the adapter
fetched the edges, then ran 12,215 prepared primary-key lookups, so
**≈1.6 ms per `prepare` + `execute` of a one-row PK lookup**. The depth-8 BFS
(A2, from a low-degree vertex) is consistent at 14 ms for its handful of
statements. The adapter re-preparing the same statement every time is its
own waste, but 1.6 ms for a prepared single-key `MATCH … LIMIT 1` in-process
is the figure to check on your side: if most of it is planning and pipeline
setup rather than the hash-index probe, a prepared-statement cache or a
lighter path for single-key lookups would move every embedding client.

## 3. Memory: 6 GB resident for a 200k-edge graph

The harness process reached **6.3 GB peak RSS** (6.0 GB steady) while
loading and querying 145k nodes / 200k edges with empty property strings.
`SystemConfig::default()` leaves `buffer_pool_size` at 0, which the engine
sizes from host RAM; the adapter passes the default and exposes no knob, so
every embedding process on a 15 GiB host reserves several gigabytes for a
few megabytes of graph. On this host a supervisor's memory watchdog killed
the first attempt for exactly that reason. A documented default cap (or an
environment override the crate reads) would make co-tenancy predictable
without adapter changes; the auto-sizing is the right default for a
dedicated database process and the wrong one for a library.

## 4. Packaging of the Rust crate

- **Duplicate symbols on Linux.** The prebuilt `liblbug.a` bundles its own
  zstd (1.5.x) and simsimd objects. Any binary that also links the
  `zstd-sys` or `simsimd` crates (here through Lance and Arrow) fails to link
  with `rust-lld`: `duplicate symbol: ZSTD_compressBound` and nineteen more
  before the linker stops. macOS `ld64` silently takes the first definition,
  which is why this only shows up on Linux. We worked around it with
  `-Wl,--allow-multiple-definition` for the final binary, which is fragile.
  Localizing the bundled third-party symbols when the static library is
  produced (`objcopy --localize-symbols`, or a partial link with
  `-Wl,--exclude-libs,ALL` / hidden visibility) would fix it for everyone.
- **Prebuilt download location.** `build.rs` downloads the prebuilt library
  into `.cache/lbug-prebuilt` *inside the crate's own source directory*, which
  for a registry dependency is `~/.cargo/registry/src/…/lbug-0.20.2/`. That
  writes into a directory Cargo treats as immutable, and it breaks
  `cargo vendor`, read-only registries, and offline builds. `OUT_DIR`, or a
  path under `$CARGO_HOME` or the XDG cache directory, would be the
  conventional home.
- Both the prebuilt-versus-source switch and the cache location deserve a
  line in the crate README; we found them by reading `build.rs`.

## 5. What worked

Every scenario that ran passed with zero hard gates: the hub's 1-hop layer
matched the oracle exactly, the depth-8 BFS frontier sizes matched, and all
one hundred concurrent hub writes were accepted with no lost or duplicated
edge when the degree was read back. Correctness was not the issue anywhere;
per-statement cost was.

## Reproduce

```
git clone https://github.com/querygraph/adversarial-graph && cd adversarial-graph
scripts/fetch-datasets.sh
cargo build --release --features ladybug
./target/release/ag run --smoke --dataset wiki-Talk --backend ladybug --out reports
```

Run bundles: `20260905T093649Z` (wiki-Talk, harness `d351e0b`, Grust
v0.13.0 adapter), the pinned rerun at harness `1baddcd` / adapter
`fdc685ee` (kept beside the publication), and the 2026-09-06 strain
publication at adversari.al/graph/strain with the rewritten adapter, each
`report.json` recording the harness and adapter revisions, client CPU and
peak RSS, and the host load average.

## Addendum, 2026-09-06: what the adapter rewrite changed, and what it did not

Following your pointers (the columnar LDBC generator's load pattern, the
multi-writer knob, issue #885), the Grust adapter was rewritten on
2026-09-06 (`querygraph/grust` `3840d152`): every entity type is one
registered Arrow table and one `COPY … FROM (MATCH …)`; tables are
resolved once per label instead of once per row (the old path also ran a
`CREATE … TABLE` attempt and a metadata `MERGE` for every node and edge,
which was the other half of the hours); a traversal step is one
`MATCH (a)-[r]->(b) WHERE a.id = $id` per relationship table; and the
buffer pool is capped at 4 GiB. Same host, same slices, same harness
scenarios, every gate still zero:

| Cell | Adapter at v0.13.0 | Adapter at 3840d152 |
|---|---|---|
| wiki-Talk load, 145,172 nodes / 200,000 edges | 5 h 03 min (11 edges/s) | 23.3 s (8,600 edges/s) |
| roadNet-CA load, 72,767 nodes / 200,000 edges | not reached | 14.0 s (14,300 edges/s) |
| wiki-Talk hub 1-hop, 12,215 rows | 18.9 s | 39.5 ms |
| roadNet-CA depth-8 BFS | not reached | 461 ms |
| A4: 100 hub writes, one `MERGE … SET` each, p50 · p99 | 37.5 ms · 974 ms | 37.2 ms · 978 ms |
| peak RSS | 6.3 GB | 0.73 GB |

So the load and the reads were the adapter's to fix, and are fixed. What
remains yours: the single-statement write at about 37 ms of CPU, unchanged
by anything on the adapter side, and the 1.4 ms `prepare` cost per point
lookup (the rewrite avoids most of them but cannot avoid all). The
`enable_multi_writes` profile with four concurrent writers is being
measured as a separate row in the same publication.
