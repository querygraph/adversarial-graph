# Benchmark reliability and backend coverage review

Reviewed snapshot: `9ee4d47`, 2026-09-08. Branch:
`review/adapter-reliability`; isolated worktree: `/tmp/adversarial-graph-review`.
This is a source review and proposal, not a new benchmark run. Fable's
checkout, services, reports, and running jobs were not changed. Historical
observations below come from the handoffs; their underlying remote receipts
were not independently revalidated here.

The highest-value next step is to make small tests detect adapter and
orchestration defects before authorizing expensive measurements. Large
datasets remain necessary for capacity findings, but not for proving that a
decoder, cleanup branch, or report classifier works.

## Findings, in priority order

1. **P1: A8 can pass without successful comparison.** In
   `src/scenarios/a8_differential.rs:167-177`, backend errors and timeouts
   increment observations only. If there are no mismatches or refusals,
   `ScenarioResult::finish` (`src/report.rs:123`) converts zero gates into
   `Pass`. All oracle failures also leave a zero-gate, zero-attempt cell
   eligible to pass. Mixed refusals and mismatches can instead produce an
   `Unsupported` headline with nonzero failure gates: failure evidence is
   retained but the headline is misleading. Define outcome precedence:
   observed correctness/runtime failures first; unsupported capabilities
   next; missing oracle coverage as explicitly incomplete; pass only for
   completed, required comparisons. Keep each query's outcome. Test all-error,
   all-timeout, zero-reference, mismatch-plus-refusal, and partial coverage.

2. **P1: A8's oracle timeout does not stop its work.** The timeout at
   `src/scenarios/a8_differential.rs:65-79` wraps `spawn_blocking`. Once started,
   that work continues after the join handle is dropped, potentially consuming
   CPU and retaining the cloned graph/index while subsequent queries run.
   Runtime shutdown can wait for it. This follows Tokio's documented
   [blocking task behavior](https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html).
   Run expensive oracle and backend computations in owned worker processes
   with deadline, termination, reap, and quiescence checks; use cooperative
   cancellation where the executor supports it. Test a deliberately stuck
   worker and prove no child or server query survives before the next cell.

3. **P1: the host guard is not scoped to its run.**
   `scripts/run-full-tiers.sh:37-58` discovers every process matching
   `./target/release/ag run`, then may kill any of them. The supplied PID only
   controls the watchdog's lifetime. A second checkout does not isolate this
   behavior. Own a process group or cgroup per invocation and terminate only
   that group. Use a unique Compose project, ports, volumes, database namespace,
   and report root as well. Test two disposable fake workers and verify only
   the owned worker is stopped. Do not exercise this guard against live jobs.

4. **P1: A12 undercounts overload and can pass request failures.** In
   `src/scenarios/a12_cold_start.rs`, the producer awaits a semaphore permit
   before scheduling its next request, and sets `scheduled` to the current
   instant rather than the intended tick. Under saturation it reduces offered
   work and omits some scheduler delay from response latency. Use a separate
   producer with absolute arrival deadlines and a bounded queue; record
   offered, admitted, dropped, completed, errored, and timed-out requests.
   Bound drain time. Compute latency from intended arrival; report success
   latency separately from failed-request latency. Ordinary stream errors
   currently produce notes without a failure gate. The final unsupported
   condition also tests whether every *error* is unsupported, not whether
   every *request* was refused. Test with a slow fake service and mixed outcomes.

5. **P1: requested work can disappear from a complete report.**
   `src/main.rs` continues on unknown datasets/backends, dataset read errors,
   and oracle construction errors, but eventually marks the report complete
   and exits according to hard gates alone. Validate requested IDs up front;
   persist dataset/setup failures and an expected-cell manifest before work
   begins. Completion should mean every requested cell is accounted for,
   with unsupported and failed setup distinct from success.

6. **P2: launcher boundaries remain brittle.** Both ladder scripts have
   readiness loops without a deadline or guaranteed cleanup traps. Full-tier
   readiness also lacks Surreal/Helix checks. In
   `scripts/run-full-tiers.sh:75`, zero-prefixed UTC fields enter Bash
   arithmetic without decimal conversion; `08` and `09` are invalid octal
   values. The blackout comparison also misses tomorrow's window when a cap
   crosses midnight. Use epoch intervals, explicit readiness deadlines, and
   cleanup tested for every exit path. Preserve the original exit cause and
   partial evidence instead of grouping cap, guard, and crash in one message.

7. **P2: A6 compares different write contracts.**
   `src/scenarios/a6_isolation.rs::Client` uses guarded CAS on Turso and plain
   read-then-upsert elsewhere. `write_mode` discloses this, but lost updates
   cannot establish an engine isolation defect when no atomic transaction was
   requested. Keep a portable upsert lane and add a separate conditional-write
   or transaction lane, marking absent capabilities unsupported. Serial
   read-back probe failures should distinguish declared missing capability
   from a broken implementation of a supported operation.

8. **P2: dependency documentation does not match the manifest.** README and
   AGENTS describe release-tag dependencies; Cargo.toml pins a raw commit and
   patches several public crates, with a comment naming a development branch.
   Verify release provenance, publish/tag the intended adapter version, and
   align the lockfile and documentation. Do not silently repin this review or
   use a local Grust checkout as a harness dependency.

## Replace full-matrix debugging with staged admission

FABLE-TO-FABLE sections 40–41 explicitly record launcher failures in declared
cell handling, cleanup, empty merges, and exit/OOM classification. They also
report that replaying validators against retained evidence exposed defects
without another measurement. Treat these as required fixtures.

1. **Offline orchestration gate:** fake worker/service commands produce pass,
   hard failure, unsupported, declared resource termination, partial output,
   invalid receipt, missing image row, and all-declared suites. Exercise the
   actual launcher, merger, bundler, and validator together. Assert cleanup
   and exact accounting, including when a worker is killed but its parent
   exits with a different status. Do not hard-code exit 137 as OOM proof.
2. **Tiny adapter contract gate:** a deterministic typed multigraph includes
   isolated vertices, a loop, parallel edges, multiple labels, null/missing
   properties, Unicode, arrays, and identifiers needing escaping. Check actual
   read-back, update, delete, reopen, and query decoding over every enabled
   transport. Include bulk batches just below, at, and above their boundaries.
   A successful load response is insufficient: verify stored cardinalities,
   edge identities, labels, and property values independently.
3. **Scale diagnostic gate:** geometric sizes on one affected backend, with
   bounds on time and memory. Record rows/request, request count, bytes moved,
   allocations, fragments, and peak memory. Detect per-edge round trips,
   repeated scans, and superlinear bulk behavior before full tiers.
4. **One real lifecycle canary:** exercise startup, declared termination,
   teardown, receipt creation, and offline admission using a small explicit
   resource cap. Retain the existing real declared-cell check as an integration
   gate; a production-size OOM should not be the first test of shell control flow.
5. **Measurement cohort:** only after the earlier gates pass. Reuse verified
   cells at identical source, dataset, query, image, lifecycle, and resource
   identities. Cross-revision reuse is a separately disclosed historical
   comparison, not a fresh same-revision cohort. Record every reused source
   receipt and retain superseded failures.

The orchestration and LSQB validators described in the handoffs belong to
other repositories; their current implementations were not reviewed here.

## Backend workload matrix

Apply common semantic cases to every backend that supports them. The
backend-specific emphasis below is a diagnostic hypothesis, not an assumed
defect or performance result.

| Backend/path | Cases to add or emphasize | Evidence and improvement to pursue |
|---|---|---|
| Memory / Rust indexed reads | Hubs versus uniform degree, triangles, duplicate relationships, zero-hop paths, OPTIONAL null extension, count overflow, concurrent write followed by indexed read | Compare exact multisets with an independent small-graph enumerator; measure adjacency visits and allocation; test snapshot invalidation and compact adjacency layouts |
| Turso WAL/MVCC | Read-heavy workloads interrupted by mutations, reopen, snapshot rebuild near memory limits, competing conditional writes | Separate durable state, resident index, oracle, and temporary buffers; stream index construction, share immutable data, and prove freshness after every mutation kind |
| PostgreSQL / PGQ / pggraph | Selectivity changes, skewed multiway joins, plan changes after statistics refresh, server SQL versus resident Rust plans | Record actual execution route, plans, rows transferred and client/server CPU; choose plans by proven shape and cost, retaining fallback and correctness checks |
| LanceDB | Batch-size sweep, many small appends, compaction followed by scans and updates/deletes, reads during compaction | The handoff reports a bulk-load repair; guard fragment growth, bounded buffers, and post-compaction exactness with small regressions |
| Surreal HTTP/SDK | Endpoint lookup under growing vertex counts, batching, typed property round-trips, partial batch failure | Confirm index use and request count; batch writes and remove repeated endpoint scans where profiles establish them |
| Helix HTTP/SDK | Request enum serialization, typed ID lookup, fresh server bootstrap, batch boundaries | Make the historical request-casing defect a wire fixture; verify index readiness and identical transport semantics |
| Ladybug | Generic put_graph versus bulk path, typed values, batch boundary errors, reopen after load | Assert that public bulk calls reach bulk ingestion; quantify copies and peak buffers, preserve exact read-back |
| FalkorDB | Directed/undirected multihop patterns, loops and duplicate edges, concurrent mutation/quiescence, graph deletion/reload | Minimize the handoff's multihop undercount into an independent fixture; verify actual loaded edges before attributing a query discrepancy; disclose native Cypher versus GraphStore reads |
| Neo4j Bolt/HTTP and Memgraph Bolt | Same graph and queries, selective lookup, skewed expansion, count versus returned rows, concurrent transactional writes, connection failure | Verify indexes online, identical semantics across transports, server-side cancellation, and batch/pool effects; retain native execution as its own disclosed route |
| AGE | Typed labels/properties, RETURN aliases, null/scalar/list decoding, delete and transaction conflicts | Extend fixtures for the historical adapter read-path repairs; distinguish unsupported typed loading from query or decoding failures |
| Sail and other feature-gated paths | Capability discovery, supported read/write round-trips, startup failure | Establish the capability contract before expensive dataset loading; missing operations remain unsupported |

## Grust engineering priorities

**First: prevent adapter regressions at the shared API boundary.** Put the
typed fixture suite beside GraphStore implementations and run it against
published candidate artifacts. Add a capability contract for bulk ingestion,
typed read-back, conditional writes, cancellation, and indexed snapshots.
Capabilities describe semantics, not backend names. Keep harness-native paths
visible until adapters implement equivalent operations.

**Second: reduce resident-index construction peak memory.** The handoffs report
budget-sensitive SF0.3 setup. Investigate graph cloning, string interning,
temporary edge collections, and simultaneous graph/index ownership. Prototype
streamed construction of compact typed adjacency with bounded scratch space.
Measure setup CPU, peak charged memory, serialized size, and query CPU
separately. A serialized index's size does not establish its construction peak.

**Third: amortize setup with explicit lifecycle semantics.** Validate immutable,
hash-bound database/index artifacts for private worker copies. Account for
artifact creation, copy/reopen, and warm-up separately. Test writes, rollback,
delete, and restart against cached snapshots before considering incremental
maintenance. Keep cold-process, fresh-connection, and warm-index results in
separate lanes; current A12's reopen is not proof of a cold server or OS cache.

**Fourth: optimize general operator shapes.** Profile typed adjacency
intersection, semijoins, factorized counts, selective expansion, and bounded
top-k. Validate multiplicity, nulls, relationship uniqueness, and overflow
before enabling any optimized path. Route by recognized semantics and measured
cost, never query IDs. Include row-returning and mutation workloads so count
plans do not stand in for the whole database workload.

**Fifth: compare equivalent work with complete cost accounting.** Use separate
portable GraphStore, native query, and Grust resident-index lanes. Match
dataset digest and slice, result semantics, durability, concurrency, warm-up,
cache state, and declared resource envelopes. For server systems account for
both client and server; separate oracle overhead where possible. Process-wide
`ru_maxrss` in `src/probe.rs` is not a per-cell peak; endpoint server memory
samples are not server peak memory. Use owned per-cell accounting and record
memory events, anonymous/file charges, throttling, and host co-tenancy.

On shared hosts, wall times are upper bounds: compare client/server CPU columns
and recorded load average alongside them. This review makes no new timing or
speedup claim. The page-cache explanation in section 40 is a hypothesis, not
proven causality; section 41 records another memory failure after cache cleanup.
Do not globally drop caches on a shared host as a routine diagnostic. Use
controlled, declared cache-state experiments on an allocated measurement host.

## Suggested implementation sequence and completion checks

### Resource envelopes that answer useful questions

The next performance cohort should not inherit the earlier 6 GiB cap by
default. Preserve those rows as capacity evidence. Use two declared lanes:

- **Capacity lane:** a small number of setup/read canaries across a predeclared
  memory ladder. Stop repeating an unchanged failure. Record the lowest tested
  successful budget, peak memory, and repeatability; do not infer an exact
  minimum or a failure probability from a handful of historical runs.
- **Performance lane:** choose one common total client-plus-server envelope
  from measured setup peaks with headroom and the host's available capacity.
  The recorded 12 GiB completion plus roughly one-third headroom makes 16 GiB
  a candidate to check, not an asserted
  sufficient limit: the earlier Turso 12 GiB and PostgreSQL 8 GiB cells did not
  necessarily include equivalent client/server charges. Validate a short
  canary for every admitted backend before fixing the cohort envelope. If
  the common budget does not fit the host alongside its tenants, schedule an
  allocated host rather than silently raising a limit during a cohort.

Publish separate answers to: time and memory to first exact answer; warm-query
CPU and tail latency; ingestion and maintenance cost; sustained successful
throughput under writes; and recovery time with exact post-recovery state.
Include a small query mix spanning selective lookup, multihop expansion,
aggregation, returned rows, and mutations. Report unsupported coverage beside
performance. Run short diagnostic repeats first, then allocate longer sampling
only where variance or a specific correctness question requires it.

Large runs should have an explicit information target: a new scale boundary,
a matched before/after adapter comparison, a cold-versus-warm distinction, or
a confidence interval that is still too wide. Repeating a known setup failure
after only report or shell changes is not such a target; replay fixtures first.

### Implementation order

1. Fix outcome classification and expected-cell accounting; regression fixtures
   must prove that failures and missing work cannot produce pass.
2. Scope process/service ownership and hard cancellation; fake concurrent jobs
   must remain untouched, and timed-out work must be reaped.
3. Test launcher exit paths offline and repair readiness/blackout handling;
   replay retained bundles before another matrix.
4. Add typed adapter conformance and correct A12 arrival accounting; verify
   overload with a controlled slow service.
5. Profile one resident-index build and one bulk-ingestion path; implement the
   highest measured allocation or I/O improvement in the owning Grust branch,
   publish the candidate, and repeat affected cells under a matched envelope.

This branch also implements finding 1: A8 errors/timeouts fire gates, absent
reference coverage prevents a pass, and hard failures take precedence over
unsupported in the headline. The tests exercise failed comparisons, partial
reference coverage, and a mismatch alongside a refusal. Other findings remain
explicit follow-up work; this is not authorization to resume an uncorrected
full matrix.

Companion implementation: `/tmp/grust-resident-index-review`, branch
`perf/resident-index-build`, contains index-construction and PostgreSQL decoding
changes with diagnostics and regression evidence in `review-evidence/`. These
are source candidates; this harness still resolves its original pinned Grust
dependencies. Follow the owning repository's release workflow before advancing
the harness to released artifacts.

Review validation: source paths and control flow inspected; remote publication
bundles were not rerun. Existing long workloads were not restarted. Changes to
published results require new generated reports and frozen, independently
verified evidence bundles; earlier failed cells remain visible.
