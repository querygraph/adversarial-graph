# Report to the Turso team: MVCC bulk-load throughput, 0.7.2 versus main

Status: draft, 2026-09-17, not yet sent. Written as the bug-report-style
contribution Turso's CONTRIBUTING asks for in the MVCC layer: reproducer and
measurements, not patches. The measurements and their provenance are in
`turso-graph-journal.md` (§3, E1/E2/E3/E8, PR #8385) and the Grust book
chapter "Turso under strain"; the reproducer's Grust revision `abea518` is
the head of PR #6 at the time of measurement (docs-only commits since).

---

Not single-threaded: we load with 4-8 concurrent writers under `BEGIN CONCURRENT`
(separate connections on one database via the shared `Database`), and we
measured 1 writer as a control. Real multi-core scaling on 0.7.2: 1/2/4/8
writers = 326/220/171/142 s on an 8-vCPU host.

What we see on `main` (`19710d58d`, 0.8.0-pre.11) vs the pinned v0.7.2, same
host, same binary, engine version asserted in every probe, alternating A/B
pairs, CPU steal recorded per run (5.1 M-edge SNAP web-Google, MVCC,
`synchronous=NORMAL` during load, upsert with ON CONFLICT, batches of 500,
mimalloc on):

- **WAL bulk load: `main` is +22% faster** (93.4k vs 76.7k edges/s). The B-tree
  improvements show clearly.
- **Hot-node concurrent single-statement writes (16 x 200, FULL): `main` is
  much better** -- engine group commit alone gives 2.62 s where our best
  client-side batching on 0.7.2 gives 3.2 s.
- **MVCC bulk load: `main` is slower, and the gap grows with the graph** --
  -5% at 1 writer and -15/-16% at 4 writers on a 5.1 M-edge graph (35.5k vs
  41.7k edges/s), replicated on two hosts; **-26% at 4 writers on a 16.5 M-edge
  graph** (25.7k vs 35.0k). That size dependence is what you'd expect from a
  per-comparison cost in the index skiplist. Same host, same config, zero
  steal, 4 writers, 16 cores: **-20%** on load at 63.5 M edges (SNAP
  sx-stackoverflow, 9.9k vs 12.3k edges/s) and **-21%** at 69 M (SNAP
  soc-LiveJournal1, 10.0k vs 12.7k), with reads (1-hop fan-out, deep
  traversal) faster on `main` in both. At 117 M edges (SNAP com-Orkut, same
  host and config): **-23%** on load (8.7k vs 11.2k edges/s) while reads are
  19-24% *faster* on `main` (A1 120.6 s vs 159.4 s; a depth-7 traversal
  reaching 3.07 M nodes 1,197 s vs 1,478 s). Peak RSS is 16-31% higher on
  `main` at these sizes. So this is confined to the MVCC write/index path;
  serving got better.
  Ruled out by A/B: `PRAGMA mvcc_group_commit` on/off (identical);
  `mvcc_checkpoint_threshold = -1` during the load (identical, and zero Busy
  retries); allocator (mimalloc lifts both versions ~15%, gap unchanged).
- `perf` (4 writers): `SortableIndexKey::compare` is 20.8% on `main` vs 19.6%
  on 0.7.2, but its callees changed -- `types::cmp_in_column` 6.2% (new) and
  `sqlite3_ondisk::read_value_serial_type` 4.3% vs 1.7%. The index-key
  comparison path is ~31% of the load on `main` vs ~25.5% on 0.7.2; B-tree
  seeks are cheaper on `main` (3.5% vs 5.8%).
- **PR #8385 fixes it.** Against its own base `bad083faf`: 23.9k/24.2k ->
  31.0k/31.4k edges/s, **+29-30%**, two alternating rounds. Profiled under the
  PR, the ~31% index-key path collapses into `types::compare_serialized_records`
  at 12.2% and `cmp_in_column` / `read_value_serial_type` drop out of the top
  symbols entirely; `from_utf8` is still ~1.3% (non-byte collations, presumably). We tried to
  cherry-pick the five commits onto `19710d58d` to measure them on current
  `main`; they conflict in `core/mvcc/cursor.rs` and
  `core/mvcc/database/mod.rs`, so we stopped there. If the arithmetic
  composes, `main`+#8385 would be ~10% ahead of 0.7.2 on this load.

For what it's worth, our workaround on `main` is to fill MVCC stores through
WAL and switch journal mode after (79.5k edges/s on the 16.5 M-edge graph,
3.1x the 4-writer MVCC load, and +19% over 0.7.2 on the same path) -- so the
B-tree work is clearly paying off; it's only the MVCC per-row path that
regressed.

Reproducer: `cargo run --release -p grust-turso --example bulk_load --features mimalloc -- --mode mvcc --snap web-Google.txt --edges 5105039 --load-threads 4`
(Grust `abea518`, https://github.com/querygraph/grust/pull/6; web-Google is the SNAP edge list, gunzipped; `--features mimalloc` is the `grust-turso` feature, or `grust-graph/turso-mimalloc` from the facade).

Happy to run anything else on this workload, or to test a rebased #8385.
