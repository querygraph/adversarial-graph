# Reply to Arun Sharma (LadybugDB), 2026-09-18

Status: draft, for the user to send. Answers his two messages of 2026-09-17:
the 0.20.4 / 0.21.x recommendation with the cit-Patents question, and the
pointer to `ldbc_data_gen_columnar/lbug_datagen/bulk.py`. Also answers his
later question about where to find Icecat. Measurements are in
Grust PR #7 (https://github.com/querygraph/grust/pull/7) and
`crates/grust-ladybug/examples/copy_bench.rs`.

---

Thanks for both — the pointer to `bulk.py` was the right one, and it settled
it. Short version: cit-Patents wasn't LadybugDB's load speed, it was our
adapter's load path, and it's fixed and on 0.20.4 now.

**What was wrong on our side.** The strain harness loads every store through
the same property-graph API (`put_graph`, upsert semantics). Our Ladybug
adapter's bulk path registered each chunk's rows as an Arrow table and ran
`COPY t FROM (MATCH (a)-[r:scratch]->(b) RETURN …)` — the row-probing shape
your docstring warns about. On 0.20.4, measured on 1 M nodes / 5 M edges:
nodes go through that path at about a million rows a second, but the
relationship probe is superlinear — 4,100 edges/s at 500 k edges, and not
finished after 132 minutes at 5 M. That is why 16.5 M edges never landed
inside our two-hour budget. On top of that the adapter read back every
existing node id and (from, to) pair per chunk to keep upsert semantics,
which is where the 9.7 GB went.

**What we changed.** The `lbug` Rust crate has no equivalent of the Python
binding's `COPY t FROM $df`, and the binder rejects `COPY t FROM
<registered arrow table>` ("Variable s_nodes is not in scope"), so we now
write each batch to a temporary CSV and run `COPY t FROM 'file'`. With our
own schema unchanged — STRING primary keys and a JSON `props` string per
row — that is 2.2 M nodes/s and 1.28 M edges/s (5 M edges in 3.9 s); with
INT64 keys and no properties, your CSR-parquet shape, 7.7 M and 6.1 M rows/s
(5 M edges in 0.82 s), which is your five seconds. The CSR registration path
(`create_arrow_rel_table_csr`, UINT64 offsets) copies at 231 k edges/s
through MATCH and, like the node tables, can't be COPY'd from directly. An
empty target table now skips the read-back. Pinned `lbug` 0.20.4; the Rust
API is identical to 0.20.2, so the gains there are all engine.

**Two things you might want to look at.**

1. A `COPY … FROM` that scans a registered Arrow table (or a `$df`-style
   parameter) in the Rust API would remove our file hop and let a Rust
   caller do exactly what `bulk.py` does.
2. `COPY rel FROM (MATCH (a)-[r:arrow_rel]->(b) …)` being ~300× slower than
   file `COPY` on the same rows, and superlinear, looks like a planner case
   rather than an inherent cost — reproducer:
   `VARIANTS=A cargo run --release -p grust-ladybug --example copy_bench -- 100000 500000`
   on the PR branch (A = the old path, C/D = file COPY, E = CSR).

**What happens next on our side.** The Ladybug ladder re-runs on the
reference host on the fixed adapter (web-Google through com-Orkut, loads
uncapped, families capped at two hours) with 0.20.4. We'll also add a
separately tagged profile that imports your CSR parquet / icebug-disk
directly once 0.21.x lands — published as a different load path, the way we
publish Turso's fill-through-WAL, not as the standard row, since the
standard row is "through the property-graph API".

One earlier item still applies: the buffer pool auto-sizes to host RAM from
`SystemConfig::default()`, which is right for a dedicated process and heavy
for a library on a shared host; our harness caps it at 4 GiB, and a
documented default cap or env override in the crate would make co-tenancy
predictable for everyone else.

**On Icecat, since you asked where to look.** The branch you found,
`feat/rust-rewrite`, is gone — it was fast-forwarded into `main` and deleted, so
that URL now 404s. Everything is on the default branch:

  https://github.com/querygraph/icecat

The README there now names what the repository holds, which it previously did
not: **Icebug**, the Arrow update of the NetworKit C++ codebase; **Icecat**, the
Rust rewrite under `rust/` (`icebug-core`, `icebug-algorithms`, `icebug-io`,
`icebug-datafusion`, `icebug-python`); and **Grustcat** and **Grustcat Cypher**,
Rust adapters exposing those kernels through the Grust property-graph API. The
last two sit outside the Rust workspace on purpose, because they answer to
Grust's dependency graph rather than Icebug's.

If you want the exact sources behind the published algorithm benchmark rather
than current `main`, they are tagged `algorithms-benchmark-2026-09-13`. That
tag's message records how faithfully the published snapshot corresponds to the
commit: 1,085 of 1,087 tracked files match, two READMEs match no commit because
they were staged from a working tree, and 601 files under `extlibs/` are
submodule contents outside the commit tree.

One caveat if you are evaluating rather than browsing: `rust/README.md` still
describes the rewrite as experimental and points at a status and roadmap
document. That framing is from when the crates landed and has not been revised,
so treat the code as current and the maturity claims as dated.

**Why the Rust rewrite is faster than the C++ it came from, since it is the same
class of finding as your `COPY`.** In the algorithm benchmark the Rust
implementation beats the NetworKit-derived C++ one on full-path Dijkstra by
1.55x at 4,096 nodes and 2.8x at 65,536 — 10,888 ms against 30,454. The widening
is the clue: a language or codegen difference is a constant factor, and this one
grows with the work.

`perf` on both, same 65,536-node chain, same container limits. C++: 36.2% in
`NetworKit::SSSP::getPath`, 20.1% in the caller's loop, **15.2% in
`do_user_addr_fault`** — kernel page-fault handling — plus 7.7% in libc and 3.0%
in kernel memory locking. Rust: 66.7% in `dijkstra_impl`, 33.2% in `main`, and no
allocator or kernel frame above 2%. About a quarter of the C++ run is the
allocator and the kernel; none of the Rust run is.

The mechanism is in the data structures, not the compilers. NetworKit stores
predecessors as `std::vector<std::vector<node>>`, one heap block per node, so
walking a path chases pointers across n separate allocations — the price of
supporting multiple shortest paths through `getPaths`. Then `getPath` builds a
fresh `std::vector<node>` per target with `push_back` and no `reserve`, so each
path reallocates as it grows, and reverses it; the caller allocates a second
vector for costs per target. On a chain the paths sum to `n(n+1)/2` entries, so
at 65,536 that is 2.1 billion entries pushed through 131,072 allocations whose
sizes grow linearly with the target index. That churn is what surfaces as page
faults.

The Rust side keeps a flat `Vec<u64>` of parents and two reconstruction buffers
allocated once and `clear()`ed per target, so after the first few paths it never
allocates again.

None of that is a verdict on the languages. It is an API shape forcing an
allocation per item — `getPath` returning by value cannot reuse a caller buffer,
so the cost is structural rather than incidental, and the same code with an
out-parameter or a visitor would close most of the gap. It reads as the same
lesson as the row-probing `COPY`: the expensive thing was the shape of the
interface, not the engine underneath it.
