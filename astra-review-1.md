# Astra review 1 — handoff for Fable

2026-09-08. The user requested a review of benchmark reliability, more useful
workloads and resource envelopes, and tested Grust improvements before further
long runs. This document is in the active checkout at the user's request;
implementation work remains on the isolated branches below.

The recommendation is to fix inexpensive correctness and orchestration checks
first, separate capacity testing from performance measurement, and profile
setup memory before another broad matrix. The changes already implemented are
useful but do not solve SF0.3's resident-memory requirements.

## Delivered work

| Repository | Branch | Commit | Contents |
|---|---|---|---|
| adversarial-graph | `review/adapter-reliability` | `982288e` | A8/report outcome fixes, regression tests, full benchmark review |
| grust | `perf/resident-index-build` | `93b6155` | Typed-index allocation improvements, PostgreSQL streaming/borrowed decoding, tests and diagnostic evidence |

Neither implementation branch has been merged, published, or used to advance
the harness's dependency pin. The original review examined adversarial-graph
`9ee4d47` and Grust `29fd384`. At this handoff, the active harness checkout is
`7f74fa4`, containing FABLE-TO-FABLE section 42; that new audit was read while
writing this document.

Full documents and evidence are available in the isolated worktrees:

- `/tmp/adversarial-graph-review/docs/notes/benchmark-review-2026-09-08.md`
- `/tmp/adversarial-graph-review/review-evidence/harness-tests.log`
- `/tmp/grust-resident-index-review/docs/RESIDENT_INDEX_REVIEW_2026-09-08.md`
- `/tmp/grust-resident-index-review/review-evidence/`, including source/evidence
  checksums in `manifest.json`, earlier attempts, and final diagnostics.

The commits preserve these files even if the temporary worktrees are removed.
For example, from the respective repository:

```sh
git show 982288e:docs/notes/benchmark-review-2026-09-08.md
git show 93b6155:docs/RESIDENT_INDEX_REVIEW_2026-09-08.md
```

## Implemented fixes and measured effects

**A8 and report outcomes.** Backend errors and timeouts now increment hard
gates. Missing reference coverage or an empty query set cannot produce a pass.
An observed hard failure takes precedence over an unsupported operation in the
cell headline, while refusal notes and individual query records remain visible.
Tests cover failed comparisons, partial reference coverage, and a mismatch
alongside a refusal. Actual cancellation of timed-out work remains outstanding.

**TypedGraphIndex construction.** Sparse adjacency reuses its temporary edge
triples for both directional sorts instead of allocating another relation-sized
buffer for each direction. Label grouping borrows keys from the immutable graph
and clones only distinct output keys. Physical edge identity, multiplicity,
snapshot ownership, and the public API are unchanged.

**PostgreSQL reads.** Node and edge reads decode protocol rows as they arrive
instead of first collecting all wire rows. Decoding borrows text from each row,
avoiding temporary owned copies of identifiers and JSON property payloads.
Returned graph values remain owned. This reduces intermediate allocation; it
does not provide a public streaming graph API or a bounded-memory snapshot.

| Small diagnostic | Before | Candidate | Interpretation |
|---|---:|---:|---|
| Sparse index cumulative requested allocation, bytes | 94,705,040 | 70,701,124 | About 25% less allocator traffic; peak memory changed little |
| Sparse index median client CPU ticks | 47 | 47 | No demonstrated CPU improvement |
| PostgreSQL read/decode median client CPU ticks | 39 | 34 | About 13% less client CPU in this diagnostic |
| PostgreSQL read/decode median client peak RSS, KiB | 237,344 | 229,788 | About 3% less peak RSS in this diagnostic |

The index fixture has 100,000 vertices, 1,000,000 physical edges, and 128 sparse
types; a one-type dense control also showed no demonstrated CPU improvement.
The PostgreSQL fixture has 100,000 generated relationship rows with 1,024-byte
string properties. Both diagnostics use three paired comparisons, alternating
order, with exact result checks. PostgreSQL uses fresh processes and the same
optimized binary for the retained old algorithm and candidate implementation.

CPU ticks are process user plus system time; the host reports 100 ticks per
second. PostgreSQL's release diagnostics recorded one-minute host load of
3.40–3.61; each index diagnostic records its own load. **Wall times on this
shared host are upper bounds: compare CPU columns and recorded load alongside
them.** Server CPU was not measured in the decoder diagnostic. These are not
SF0.3 results, whole-system throughput measurements, or cross-engine comparisons.
Earlier debug-profile attempts without a CPU improvement remain in the evidence.

Validation: 201 Grust tests/doc tests across core, Memory, PostgreSQL, and Turso;
eight live PostgreSQL regression tests; and 24 harness tests passed. One
existing harness test was ignored. PostgreSQL's 21 offline tests were repeated
after the borrowed-text change; do not count those again as additional distinct
tests. Modified Rust files passed formatting checks. No full all-feature
workspace release or packaging gate was run.

## Remaining findings to fix before broad reruns

These findings refer to the reviewed source snapshot; check subsequent changes
before implementing a second fix.

1. **A8 timeout does not stop the reference executor.** The timeout wraps a
   `spawn_blocking` join handle. Started blocking work can continue after the
   handle is dropped, retain the graph, consume CPU during later queries, and
   delay shutdown. Use owned workers with termination/reap/quiescence checks
   or proven cooperative cancellation. Test a deliberately stuck worker.
2. **The full-tier memory guard can target another run.**
   `scripts/run-full-tiers.sh` uses a host-wide `pgrep` pattern for `ag run`.
   Its supplied PID bounds the guard's lifetime, not the processes it kills.
   Own a process group or cgroup per invocation. Test two disposable workers
   and prove that the unrelated worker survives. Worktrees alone do not
   isolate process discovery, Compose services, ports, or volumes.
3. **A12's producer waits for a worker permit.** Under saturation this slows
   offered arrivals and misses some scheduling delay. Use intended absolute
   arrival timestamps, a separate producer and bounded queue, explicit offered/
   admitted/dropped/completed/error counts, and bounded draining. Ordinary
   request errors currently can leave a passing cell; the unsupported condition
   examines errors rather than all requests. Add a slow-service regression.
4. **Requested work can disappear from a complete report.** Unknown IDs,
   dataset read failures, and oracle construction failures can be skipped in
   `src/main.rs` before the report is marked complete. Validate IDs first and
   persist an expected-cell manifest with explicit setup outcomes.
5. **Launcher lifecycle gaps.** Add readiness deadlines and reliable cleanup
   on every exit. Fix zero-prefixed UTC arithmetic and blackout windows across
   midnight. Exercise real shell orchestration with fake services/workers for
   failure, refusal, declared termination, partial output, and all-declared
   suites. Replay existing bundles through validators before remeasuring.
6. **A6 write contracts differ.** Turso's guarded CAS and other paths' plain
   read-then-upsert are distinct workloads. Keep portable upsert and conditional/
   transactional lanes separate; unsupported capabilities remain unsupported.
7. **Release provenance needs alignment.** The README/AGENTS release-tag
   description and the manifest's raw commit/patch configuration differ.
   Resolve this through the owning release workflow. Do not use local Grust
   path dependencies or silently advance the harness pin for these candidates.

## Resource policy and workload proposal

Keep the existing 6 GiB cells as visible capacity evidence. Stop repeating an
unchanged setup failure merely because reporting or launcher code changed.

- **Capacity lane:** small setup/read canaries across a declared memory ladder,
  recording the lowest tested successful budget, peak charges, and repeatability.
- **Performance lane:** a common total client-plus-server envelope chosen from
  measured setup peaks and headroom, subject to actual host/tenant capacity.
  The recorded 12 GiB completion plus roughly one-third headroom suggests
  16 GiB as a canary candidate, not a proven fit or a new universal default.
  Earlier container/client envelopes were not equivalent. Verify equivalent
  accounting before freezing a cohort.

Measure separate useful outcomes: first exact answer including setup; warm
query CPU and tail latency; ingestion/maintenance cost; successful throughput
under writes; and recovery with exact post-recovery state. Use selective lookup,
multihop expansion, aggregation, returned rows, and mutation workloads. Disclose
portable GraphStore, native query, and resident-index execution routes separately.

Backend emphasis, applied with common semantic checks wherever supported:

| Backend/path | Next diagnostic emphasis |
|---|---|
| Memory, Turso, PostgreSQL resident indexes | Setup peak ownership, retained representations, rebuild after every mutation kind, skew and multiplicity |
| PostgreSQL native SQL / PGQ / pggraph | Selectivity, join skew, plan and statistics changes, transferred rows, client plus server cost |
| LanceDB | Fragment growth, compaction correctness, and measured table-handle caching with correct freshness/invalidation |
| Surreal and Helix HTTP/SDK | Request count, batch boundaries, indexed endpoint lookup, serialization and typed property parity |
| Ladybug | Public bulk calls reaching bulk ingestion, typed values, copy/buffer peaks, reopen correctness |
| FalkorDB | Minimized multihop discrepancy fixtures, verified stored edges, loops/parallel edges, and quiescence |
| Neo4j HTTP/Bolt and Memgraph Bolt | Equivalent query/transport semantics, online indexes, skewed expansion, transaction correctness, cancellation |
| AGE | Typed labels/properties, RETURN aliases, value decoding, deletes and typed conflicts |
| Other feature-gated paths | Tiny capability/read-back checks before large loads; missing operations explicitly unsupported |

Section 42's PostgreSQL batch-size audit concerns ingestion; the candidate here
concerns read-back/decoding. They test different hypotheses. Do not reopen the
commit-frequency hypothesis without new evidence. Its LanceDB handle-caching
and HTTP batching proposals are sensible next experiments, with measured
before/after controls and semantic regression tests.

## Coordination and suggested next actions

1. Review the two commits and integrate deliberately after checking concurrent
   changes. Rerun affected gates after integration; historical passing tests do
   not certify a new merge or release.
2. Address the remaining outcome, ownership, timeout, and launcher problems
   with small fixtures. This is the highest priority before more long runs.
3. Profile one persistent-backend setup cell: durable scan, decoded graph,
   temporary structures, resident index, and oracle footprint separately.
   Fewer allocations in these diagnostics do not establish a large capacity gain.
4. Use short correctness/setup canaries to establish the performance envelope,
   then run only cells that answer a new scale, variance, lifecycle, or
   before/after question. Follow the release workflow before repinning adapters.
5. Generate results with the existing scripts, freeze evidence, independently
   verify it, and retain every earlier failed or superseded cell.

Fable's original checkouts and services were not edited or stopped during the
review. **The builds and diagnostics did consume shared-host resources while
Fable continued running.** Treat overlapping observations as co-tenanted;
do not assume those intervals were quiet. The disposable PostgreSQL test
container was stopped and removed. No global cache drop or full matrix restart
was performed. No messages were sent to external services on Fable's behalf.
