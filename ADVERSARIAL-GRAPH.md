# ADVERSARIAL-GRAPH

**GRAPH-ADVERSARIAL-v1 — an adversarial benchmark that exercises every layer of
the QueryGraph stack.** Research findings, first-principles design, and the
plan for the harness in this repository.

Research date: 2026-09-04. Every external claim below carries its source; the
stack facts come from the sibling checkouts at the revisions in
`~/src/querygraph/QUERYGRAPH.md`. This document is written de novo from the
datasets and the literature; it is not derived from the in-tree Grust LSQB
harness, which is being developed separately and will be compared with this
one afterwards.

---

## 1. What the industry benchmarks measure, and what they do not

### 1.1 The Graph Data Council (formerly LDBC)

The body usually referred to as the "graph benchmark council" is one
organization: the **Linked Data Benchmark Council rebranded to the Graph Data
Council (GDC) in 2025** "to better reflect our broad focus on graph data
management" ([history](https://ldbcouncil.org/post/the-history-of-the-graph-data-council/),
[introduction](https://ldbcouncil.org/introduction/)). It began as an EU FP7
project in 2012 (UPC and CWI coordinating) and is now a member-funded
non-profit with a Board of Directors and a Members Policy Council
([TPCTC 2023 organization paper](https://arxiv.org/abs/2307.04350)).

Its benchmarks, and the single most useful idea they contribute:

| Benchmark | Measures | Does not measure |
|---|---|---|
| **SNB Interactive** v1/v2 ([spec](https://arxiv.org/pdf/2001.02299), [v2 paper](https://arxiv.org/abs/2307.04820)) | Transactional throughput (ops/s) under a scheduled-arrival driver: 14 complex neighbourhood reads, 7 short reads, 8 inserts; v2 adds 8 **deletes** including recursive cascades and a cheapest-path query. Mix ≈ 8% complex / 72% short / 20% insert / 0.2% delete. | Whole-graph analytics; multi-client isolation anomalies; result correctness beyond a validation set; resource footprint. |
| **SNB Business Intelligence** ([PVLDB 16](https://www.vldb.org/pvldb/vol16/p877-szarnyas.pdf)) | 20 join/aggregation-heavy queries over most of the graph with delete-aware microbatches; power@SF and throughput@SF plus 3-year TCO. | Low-latency transactional ops, concurrency safety, streaming ingest. |
| **Graphalytics** ([spec](https://arxiv.org/pdf/2011.15028)) | Six global algorithms (BFS, PageRank, WCC, CDLP, LCC, SSSP) on analytics platforms. | Query languages, transactions, updates, storage-engine behaviour. |
| **FinBench** ([paper](https://arxiv.org/pdf/2306.15975)) | Financial anti-fraud OLTP: neighbourhood reads plus continuous inserts/deletes on distributed transactional systems. No audited results yet. | Analytics; any comparative record. |
| **LSQB** ([repo](https://github.com/ldbc/lsqb)) | 9 subgraph-pattern queries that stress join ordering, worst-case-optimal joins, and cardinality estimation. A microbenchmark, explicitly not audited. | Filters, aggregation, updates, transactions. |
| **Graph500** ([spec](https://graph500.org/?page_id=12)) | Distributed BFS/SSSP TEPS on Kronecker graphs (A=0.57, B=C=0.19, D=0.05, edge factor 16). | Databases entirely: no queries, storage, transactions, or real topology. |
| **GAP** ([Beamer et al.](https://arxiv.org/abs/1508.03619)) | Six single-node kernels (BFS, SSSP, PR, CC, BC, TC) on five fixed graphs (Twitter 61.6M/1.47B, Web 50.6M/1.95B, Road 23.9M/58.3M, Kron, Urand). | Persistence, query languages, concurrency, updates. |

**Auditing.** Only GDC members can commission an audit (£3,000); a certified
auditor re-executes the benchmark, cross-validates query results, runs ACID
compliance tests, and publishes a Full Disclosure Report
([process](https://ldbcouncil.org/docs/ldbc-snb-auditing-process.pdf)). The
audited Interactive record progressed from TuGraph (~13.5k ops/s, 2023) to
GraphScope Flex (130,098 ops/s at SF100 and the first SF1000 completion, May
2024, [FDR](https://ldbcouncil.org/docs/audits/snb/LDBC_SNB_I_20240514_SF100-300-1000_graphscope.pdf))
to Huawei GES (125.9–139.4k ops/s at SF100–1000, December 2025). Audited BI
results exist only for TigerGraph and TuGraph. **No audited LDBC result for
Neo4j was found**, and the vendor comparisons that do exist — TigerGraph vs
Neo4j ([report](https://www.tigergraph.com/benchmark/), rebutted by Redis for
single-request methodology), Memgraph "120× Neo4j at ¼ memory"
([blog](https://memgraph.com/blog/memgraph-vs-neo4j-performance-benchmark-comparison),
criticized in ["Bullshit Graph Database Performance Benchmarks"](https://maxdemarzi.com/2023/01/11/bullshit-graph-database-performance-benchmarks/)),
FalkorDB "p99 136 ms vs 46.9 s"
([blog](https://www.falkordb.com/blog/graph-database-performance-benchmarks-falkordb-vs-neo4j/)),
ArcadeDB "473× on 2-hop" ([page](https://arcadedb.com/benchmarks.html)) — all
show the tuning-asymmetry pattern that Raasveldt et al. catalogue in *Fair
Benchmarking Considered Difficult*
([DBTest'18](https://dl.acm.org/doi/10.1145/3209950.3209955)).

**Choke points.** LDBC designs workloads around "well-chosen technical
difficulties that are challenging for the present generation of data
processing systems" ([choke-points.tex](https://github.com/ldbc/ldbc_snb_docs/blob/main/choke-points.tex)).
Nine families matter here: CP-1 aggregation, CP-2 join (incl. 2.5
worst-case-optimal joins, 2.6 factorized execution), CP-3 data-access
locality, CP-4 expression evaluation, CP-5 correlated sub-queries, CP-6
parallelism and concurrency (6.1 inter-query result reuse), **CP-7 graph
specifics** (7.1 incremental path computation, 7.2 cardinality estimation of
transitive paths, 7.3 executing a transitive step, 7.4 termination criteria,
7.5 unweighted shortest paths, 7.6 cheapest paths, 7.7 composition of graph
queries, 7.8 reachability across disconnected components), CP-8 language
features, and **CP-9 update operations** (9.1–9.5 insert node/edge, delete
node/edge, delete recursively). This benchmark adopts the vocabulary and adds
the families the council does not cover (§3).

### 1.2 The third-party literature

- *Beyond Macrobenchmarks* (Lissandrini et al., [VLDB 2019](https://www.vldb.org/pvldb/vol12/p390-lissandrini.pdf)):
  containerized microbenchmarks across Neo4j, OrientDB, JanusGraph, ArangoDB;
  motivated by the fact that earlier studies contradicted each other; Neo4j
  strongest on traversals, JanusGraph weakest, large variance by operation
  class.
- Kuzu ([CIDR 2023](https://www.cidrdb.org/cidr2023/papers/p48-jin.pdf)):
  factorized, vectorized execution with worst-case-optimal joins; third-party
  [kuzudb-study](https://github.com/prrao87/kuzudb-study) reports ~18× faster
  ingestion and up to two orders of magnitude on multi-hop paths vs Neo4j on a
  100k-node graph. Kuzu was archived in October 2025; LadybugDB is the
  community continuation.
- DuckPGQ ([CIDR 2023](https://www.cidrdb.org/cidr2023/papers/p66-wolde.pdf)):
  a columnar RDBMS with SQL/PGQ matches or beats native graph databases on
  pattern matching — the null hypothesis any "native graph" claim must beat.
- Surveys: Sahu et al. ([VLDB 2018](https://vldb.org/pvldb/vol11/p420-sahu.pdf))
  find scalability and super-nodes are practitioners' top pain points; Besta
  et al., *Demystifying Graph Databases* ([arXiv](https://arxiv.org/pdf/1910.09017)).

### 1.3 Robustness testing is a separate literature — and it never meets load

The correctness-testing work on graph databases is single-connection and
logic-bug oriented: **Grand** (Gremlin differential testing, [ISSTA 2022](https://dl.acm.org/doi/10.1145/3533767.3534409), 21 bugs),
**GDsmith** (Cypher differential testing, [ISSTA 2023](https://dl.acm.org/doi/10.1145/3597926.3598046), 27 bugs, one crashing RedisGraph),
**GDBMeter** (query partitioning, [ISSTA 2023](https://dl.acm.org/doi/10.1145/3597926.3598044)),
**Gamera** (graph-aware metamorphic relations, [VLDB 2024](https://www.vldb.org/pvldb/vol17/p836-zhuang.pdf), 39 bugs),
**GraphGenie** and **GRev** ([ICSE 2024](https://dl.acm.org/doi/10.1145/3597503.3623307)),
**Dinkel** (state-aware Cypher generation, 60 bugs, [arXiv](https://arxiv.org/abs/2408.07525)).
Concurrency safety has only been examined by Jepsen, and only for Dgraph
([2018](https://jepsen.io/analyses/dgraph-1-0-2): snapshot-isolation
violations and lost inserts even without faults; [2020](https://jepsen.io/analyses/dgraph-1.1.1):
five safety issues). **No Jepsen analysis of Neo4j, Memgraph, TigerGraph, or
any embedded graph database exists.** Jepsen's [Elle](https://github.com/jepsen-io/elle)
checker ([VLDB 2021](https://people.ucsc.edu/~palvaro/elle_vldb21.pdf)) is
directly reusable for graph mutations.

The methodological gap is precise: *no public benchmark combines sustained
adversarial load (super-node fan-out, deep deletes, concurrent hot-node
writers, unbounded results) with correctness and isolation checking and with
resource accounting.* That is the gap GRAPH-ADVERSARIAL-v1 fills.

### 1.4 Measuring the tail honestly

Closed-loop load generators hide server stalls because a blocked worker stops
issuing requests — **coordinated omission** (Tene; [wrk2](https://github.com/giltene/wrk2),
[ScyllaDB explainer](https://www.scylladb.com/2021/04/22/on-coordinated-omission/)).
The harness therefore drives **open-loop** load from a fixed-rate or Poisson
schedule, measures from the *intended* send time, records into an
HdrHistogram, and reports p50/p99/p99.9 with ≥10⁴ samples per percentile
decade. Memory is reported as peak RSS (`ru_maxrss`), for server systems
inclusive of every process in the container, at a fixed total memory budget.
Cold start is measured as process-exec-to-first-correct-query with the page
cache dropped, and separately at steady state.

---

## 2. Datasets, from first principles

A dataset earns a place in this benchmark because it exhibits a **named
structural pathology** that stresses a specific mechanism, not because it is
popular. Every dataset below is freely downloadable without registration;
sizes were verified by HTTP HEAD on 2026-09-04. Synthetic inputs are labelled
as such.

### 2.1 Pathology classes

| Pathology | Mechanism it stresses | Dataset(s) | Published numbers |
|---|---|---|---|
| **Degree skew / super-nodes** | Adjacency storage of a single vertex; relationship-chain locking; k-hop frontier explosion; page-cache residency | twitter-2010; wiki-Talk; soc-LiveJournal1 | twitter-2010 max out-degree **2,997,469**, max in-degree 770,155, avg 35 (five orders of magnitude of skew, [LAW](https://law.di.unimi.it/webdata/twitter-2010/)); wiki-Talk: SCC covers only 4.7% of nodes |
| **High diameter** | Variable-length path termination (CP-7.4), BFS frontier depth, recursive CTE depth limits | roadNet-CA (diameter **849**), GAP-road | [SNAP](https://snap.stanford.edu/data/roadNet-CA.html) |
| **Dense overlapping communities** | Triangle/clique enumeration, intermediate-result size (CP-2.6 factorization) | com-Orkut (avg degree 76, diameter 9), com-Friendster | [SNAP](https://snap.stanford.edu/data/com-Orkut.html) |
| **Many components / dangling nodes** | Reachability across disconnected components (CP-7.8); unbounded MATCH without anchors | web-Google; twitter-2010 (3.72% dangling) | [SNAP](https://snap.stanford.edu/data/web-Google.html) |
| **Temporal multi-edges** | Parallel-edge preservation vs structural edge keys; deletes of repeated interactions | sx-stackoverflow (63.5M temporal edges over ~36M distinct pairs); wiki-talk-temporal; Reddit hyperlinks (86 edge properties) | [SNAP](https://snap.stanford.edu/data/sx-stackoverflow.html) |
| **Typed property graph** | Label/property indexes, schema enforcement, Cypher over real types | LDBC SNB SF0.1/SF1 (8 node labels, 15+ relationship types, dates); soc-Pokec (59 profile columns); ICIJ Offshore Leaks (~2M typed nodes) | [LDBC datasets](https://datasets.ldbcouncil.org/), [Pokec](https://snap.stanford.edu/data/soc-Pokec.html), [ICIJ](https://offshoreleaks-data.icij.org/offshoreleaks/csv/full-oldb.LATEST.zip) |
| **Write streams with typed deletes** | CP-9 inserts/deletes, recursive delete cascades | LDBC SNB update streams (`-updates` bundles) | [LDBC](https://datasets.ldbcouncil.org/snb-interactive-v1-updates/) |
| **Uniform degree (no locality)** | Defeats degree-aware optimizations | GAP-urand (synthetic, 134M/4.3B) | [SuiteSparse](https://sparse.tamu.edu/MM/GAP/) |
| **Skew at scale (synthetic)** | Kronecker generators with max degree in the millions by construction | Graph500 / GAP-kron (synthetic) | [Graph500](https://graph500.org) |

### 2.2 The dataset ladder

Tiers are chosen so that every scenario has a run that fits a laptop CI
budget and a run that does not.

| Tier | Dataset | Nodes / edges | Download | Size | License |
|---|---|---|---|---|---|
| S | email-Eu-core | 1,005 / 25,571 | `https://snap.stanford.edu/data/email-Eu-core.txt.gz` (+ department labels) | 78 KB | SNAP (cite) |
| S | ego-Facebook | 4,039 / 88,234 | `https://snap.stanford.edu/data/facebook_combined.txt.gz` | 214 KB | SNAP |
| S | LDBC SNB SF0.1 | ~1.5k persons, typed | `https://datasets.ldbcouncil.org/snb-interactive-v1/social_network-sf0.1-CsvBasic-LongDateFormatter.tar.zst` | 17.5 MB | LDBC (CC-BY, verify) |
| S | wiki-Talk | 2,394,385 / 5,021,410 | `https://snap.stanford.edu/data/wiki-Talk.txt.gz` | 16 MB | SNAP |
| S | roadNet-CA | 1,965,206 / 2,766,607 | `https://snap.stanford.edu/data/roadNet-CA.txt.gz` | 17 MB | SNAP |
| S | web-Google | 875,713 / 5,105,039 | `https://snap.stanford.edu/data/web-Google.txt.gz` | 20 MB | SNAP |
| S | ICIJ Offshore Leaks | ~2M typed nodes | `https://offshoreleaks-data.icij.org/offshoreleaks/csv/full-oldb.LATEST.zip` | 72 MB | ODbL / CC-BY-SA (verify) |
| M | cit-Patents | 3,774,768 / 16,518,948 | `https://snap.stanford.edu/data/cit-Patents.txt.gz` | 81 MB | SNAP |
| M | LDBC SNB SF1 (+ updates) | ~3M / 17M typed | `https://datasets.ldbcouncil.org/snb-interactive-v1/social_network-sf1-CsvBasic-LongDateFormatter.tar.zst` | 220 MB | LDBC |
| M | soc-Pokec (+ profiles) | 1,632,803 / 30,622,564 | `https://snap.stanford.edu/data/soc-pokec-relationships.txt.gz`, `soc-pokec-profiles.txt.gz` | 126 + 415 MB | SNAP |
| M | soc-LiveJournal1 | 4,847,571 / 68,993,773 | `https://snap.stanford.edu/data/soc-LiveJournal1.txt.gz` | 248 MB | SNAP |
| M | com-Orkut | 3,072,441 / 117,185,083 | `https://snap.stanford.edu/data/bigdata/communities/com-orkut.ungraph.txt.gz` | 427 MB | SNAP |
| M | sx-stackoverflow (temporal) | 2,601,977 / 63,497,050 | `https://snap.stanford.edu/data/sx-stackoverflow.txt.gz` | 530 MB | SNAP |
| L | twitter-2010 | 41,652,230 / 1,468,365,182 | `https://snap.stanford.edu/data/twitter-2010.txt.gz` | 5.5 GB | cite Kwak et al. / LAW |
| L | com-Friendster | 65,608,366 / 1,806,067,135 | `https://snap.stanford.edu/data/bigdata/communities/com-friendster.ungraph.txt.gz` | 9.4 GB | SNAP |
| L | GAP road / web / kron / urand | up to 134M / 4.3B | `https://sparse.tamu.edu/MM/GAP/GAP-{road,web,kron,urand}.tar.gz` | 0.2–17 GB | CC-BY 4.0 |

`scripts/fetch-datasets.sh` downloads tiers S and M by default (~2.2 GB) and
verifies size and SHA-256 into `datasets/MANIFEST.json`; `--large` adds tier
L. Datasets that require registration (Yelp, Semantic Scholar) or whose terms
now require a permission form (MovieLens) are deliberately excluded.

### 2.3 Generators, for the parts real data cannot give

Two synthetic families are used *alongside*, never instead of, real graphs:
Graph500-parameter R-MAT for scale sweeps at controlled skew, and a
deterministic **temporal update stream** derived from a real graph (edges
sorted by timestamp, with a configurable delete ratio) so that CP-9 delete
choke points can be exercised on sx-stackoverflow and wiki-talk-temporal, not
only on LDBC.

---

## 3. Design principles

1. **Adversarial means the input and the request path are trying to mislead
   the system.** Skew, depth, density, unbounded patterns, hot-node write
   contention, ambiguous outcomes, restarts at inconvenient moments, and
   requests that try to widen a governed answer. Steady-state throughput on a
   friendly workload is reported but is not what the benchmark is for.
2. **Every layer of the stack, not just the storage engine.** The QueryGraph
   stack is layered — storage engine, Grust `GraphStore`, Cypher/GQL with the
   bounded read policy, guarded commits, TypeSec-governed memory, LakeCat's
   catalog projection through the transactional outbox, QueryGraph's semantic
   answers with their proof bases, and OpenLineage — and a graph that is
   correct at layer 1 can still leak at layer 5. Each scenario names the layer
   it exercises (§4).
3. **Hard gates are zero-tolerance and never averaged.** Following
   `adversarial-cognition`, safety and correctness failures are counted in
   named gates that must all be zero; quality and performance are separate
   sections of the report.
4. **Ground truth is computed, not trusted.** For structural queries the
   oracle is Grust's in-memory `GraphIndex` over the same loaded graph
   (degree, neighbourhood sets, BFS layers, component ids); for Cypher the
   oracle is `grust-cypher`'s reference executor over the materialized graph;
   for isolation the oracle is an Elle-style history checker over
   register/list operations on nodes.
5. **Unranked, source-pinned, immutable evidence** — the `catalog-bench`
   discipline: every result is `pass`, `fail`, `unsupported`, or `not-tested`;
   performance eligibility is derived; bundles record engine images by digest,
   dataset hashes, configuration, sanitized transcripts, and cleanup.
6. **Fairness is mechanical.** Same container CPU and memory limits, same
   dataset bytes, same open-loop schedule, same warm-up, same host; every
   external system runs its released image with vendor-documented memory
   tuning (documented per profile), never defaults-by-accident.

---

## 4. Scenario families and the layers they exercise

Layer key: **L0** storage engine (Turso/Postgres/RocksDB/LMDB/…) · **L1** Grust
`GraphStore` (put/get/traverse, batching, edge keys) · **L2** Cypher/GQL +
`ReadQueryPolicy` · **L3** transactions and guarded commits · **L4**
TypeSec-governed memory (`querygraph-memory`, labels, capabilities) · **L5**
LakeCat catalog projection + outbox replay · **L6** QueryGraph semantic
answers and proof bases · **L7** OpenLineage.

| ID | Family | What is adversarial | Layers | Datasets | Hard gate(s) | Choke points |
|---|---|---|---|---|---|---|
| **A1** | Super-node fan-out | k-hop (k=1..3) from the maximum-degree vertex; neighbourhood counts must match the `GraphIndex` oracle; memory recorded | L0–L2 | twitter-2010, wiki-Talk, soc-LiveJournal1 | `wrong_answer`, `oom_or_crash` | CP-3.3, 7.3 |
| **A2** | Deep paths | Variable-length paths on road graphs to depth 50/200/849; shortest path between far vertices; policy-bounded systems must *refuse* beyond their declared hop limit, not hang | L1–L2 | roadNet-CA, GAP-road | `hang_or_timeout_without_refusal`, `wrong_answer` | CP-7.1, 7.4, 7.5 |
| **A3** | Unbounded results | `MATCH (n) RETURN n`, cartesian products, range bombs, deep UNION arms; the bounded read policy must reject; unbounded systems are measured to first byte, RSS, and completion | L2 | com-Orkut, web-Google | `policy_bypass`, `oom_or_crash` | CP-1.3, 2.3 |
| **A4** | Hot-node write contention | N writers (8, 32, 128) attaching edges to one node under open-loop load; every write must be either durably applied or rejected with a typed conflict; no silent loss | L0, L1, L3 | wiki-Talk hub, synthetic star | `lost_write`, `duplicate_durable_mutation`, `non_conflict_error` | CP-6, 9.2 |
| **A5** | Recursive deletes | SNB v2-style delete cascades and temporal-stream deletes of repeated edges; post-state must equal the oracle; parallel edges must survive when the model declares them | L1, L3 | LDBC SNB SF1 updates, sx-stackoverflow | `wrong_answer`, `lost_write` | CP-9.3–9.5 |
| **A6** | Isolation under mixed load | Elle-style register and list-append histories on node properties and adjacency under concurrent readers/writers; anomalies classified (G0, G1a/b/c, G-single, lost update, dirty traversal) | L3 | LDBC SNB SF0.1 | `isolation_anomaly` | — |
| **A7** | Ambiguous commit and restart | Kill the store mid-transaction; kill the process between COMMIT and ack; restart; replay the guarded commit with the same idempotency key; exactly one durable effect | L0, L3 | any M-tier | `duplicate_durable_mutation`, `lost_write`, `non_deterministic_receipt` | — |
| **A8** | Differential Cypher | GDsmith/Gamera-style generated queries (metamorphic: add-then-remove, projection narrowing, reversed patterns) run on every backend and the reference executor; disagreement is a logic bug | L2 | LDBC SF0.1, ICIJ | `wrong_answer` | CP-8 |
| **A9** | Governed memory under strain | Recall with typed clearance while an adversary inserts confusables, oversized records, forged provenance, cross-tenant ids, and replayed proposals at high rate; rank-only paths must never widen authority | L4 | ICIJ (as a memory corpus), synthetic tenants | `unauthorized_disclosure`, `cross_scope_leakage`, `residual_recall_after_forget` | — |
| **A10** | Catalog projection replay | LakeCat emits catalog events for thousands of tables under commit contention while the graph sink is periodically unavailable; outbox replay must produce exactly the oracle graph with stable event ids | L5, L1 | synthetic catalog (10k tables) | `duplicate_durable_mutation`, `lost_write`, `wrong_answer` | — |
| **A11** | Semantic answer drift | Publish an Ossie model over LDBC SF1 tables, answer five metrics, then perturb artifact, model, policy, plan, graph, lineage under concurrent load; every drift must be rejected | L5–L7 | LDBC SNB SF1 | `drift_accepted`, `non_deterministic_receipt` | — |
| **A12** | Cold start and footprint | Time to first correct query from a cold page cache; peak RSS for the S and M ladders at a fixed memory budget; steady-state p99.9 under open-loop mixed load | L0–L2 | ladder | none (performance section) | — |

Families A1–A8 apply to every graph store, internal and external. A9–A11
apply to the QueryGraph stack only, because no other system has those layers;
they are reported as stack-integrity evidence, not as a comparison.

### 4.1 Systems under test

| System | Access path | Notes |
|---|---|---|
| Grust `MemoryGraphStore` | in-process | reference; oracle host |
| Grust `TursoGraphStore` (WAL and MVCC) | embedded | only `GraphCommitStore`; `BEGIN CONCURRENT` under MVCC |
| Grust `PostgresGraphStore`, `PostgresPgqStore`, `PgGraphStore` | Postgres 18.6 / 19β PGQ / pgGraph 1.2 | digest-pinned images |
| Grust `SurrealHttpGraphStore` / SDK | SurrealDB 3.2.4 | |
| Grust `FalkorGraphStore` | FalkorDB 4.20.4 | write path only through `GraphStore` (reads return `Unsupported`); native Cypher for reads |
| Grust `LanceDbGraphStore` | embedded | load/read only |
| Grust `SailGraphStore` | Sail Spark Connect, pinned rev | Cypher pushdown |
| Grust `HelixHttpGraphStore`, `LadybugGraphStore` | unpublished crates | via `git` dependency on the Grust tag, feature-gated |
| Neo4j 5.26 Community (Bolt, `neo4rs`) | `src/neo4j.rs`, a harness-side `GraphStore` | heap 3G / page cache 3G / `db.memory.transaction.total.max` 1G in an 8 GiB container (`compose.yaml`); `UNWIND`-batched loads, `(:V {id})` index; `read_path = harness-native-cypher` |
| Memgraph, Kuzu/LadybugDB native, Apache AGE | external adapters (phase 2) | |

### 4.2 Report contract

`reports/<run>/report.json`: run provenance (dataset hashes, image digests,
crate versions, host, limits), per-system × per-scenario outcome
(`pass|fail|unsupported|not-tested`), the nine hard-gate counters, quality
(oracle agreement rate by family), and performance (HdrHistogram percentiles,
throughput, peak RSS, cold start). `RESULTS.md` is generated from the JSON;
there is no hand-maintained table.

Hard gates (all must be zero for a `pass`): `wrong_answer`, `lost_write`,
`duplicate_durable_mutation`, `isolation_anomaly`, `policy_bypass`,
`hang_or_timeout_without_refusal`, `oom_or_crash`, `unauthorized_disclosure`
(incl. `cross_scope_leakage`, `residual_recall_after_forget`),
`non_deterministic_receipt` (incl. `drift_accepted`).

---

## 5. Why these dimensions separate architectures

The dimensions in §4 are chosen because the literature shows they are where
graph-database architectures actually diverge, not where they all look alike.
Reported factually:

- **Memory model.** Systems that keep query intermediates and transaction
  state on a managed heap document that an unbounded `MATCH` can exhaust it
  (Neo4j: `MemoryPoolOutOfMemoryError`, [issue #13008](https://github.com/neo4j/neo4j/issues/13008),
  [#12850](https://github.com/neo4j/neo4j/issues/12850)) and that collection
  pauses stall every thread, with performance dropping "as much as two orders
  of magnitude when GC-trashing happens" ([GC tuning docs](https://neo4j.com/docs/operations-manual/current/performance/gc-tuning/));
  a KB article records pauses "of the order of minutes" triggering cluster
  re-elections ([KB](https://neo4j.com/developer/kb/mitigating-causal-cluster-re-elections-caused-by-high-gcs/)).
  Systems with native memory and a bounded read policy (Grust's
  `ReadQueryPolicy`: 100k nodes, 500k edges, 1M candidate work, 256 MiB
  intermediates, 4 hops, 2 s, by default) fail closed by construction. A3 and
  A12 make both behaviours measurable rather than argued.
- **Hot-node writes.** Neo4j marks a node dense at 50 relationships and
  redesigned relationship-chain locking in 4.3 because contention on shared
  nodes was serious ([blog](https://neo4j.com/blog/developer/relationship-chain-locks-dont-block-the-rock/));
  `DeadlockDetectedException` under concurrent writes is documented behaviour
  with retry guidance ([docs](https://neo4j.com/docs/operations-manual/current/performance/locks-deadlocks/)).
  JanusGraph answers super-nodes with vertex-centric indexes that require a
  reindex to add ([docs](https://docs.janusgraph.org/v0.2/basics/index-performance/)).
  Turso resolves the same race with MVCC write-write conflict detection at
  commit and a bounded retry; Kuzu/Ladybug serialize on a single writer. A4
  measures accepted-vs-conflict-vs-error under identical open-loop pressure.
- **Isolation.** Neo4j documents read-committed with unprotected traversal
  paths ([docs](https://neo4j.com/docs/operations-manual/current/database-internals/concurrent-data-access/));
  Memgraph advertises MVCC snapshot isolation; Dgraph claimed snapshot
  isolation and Jepsen found violations. A6 classifies what each system
  actually provides.
- **Planner regressions.** A 20× slowdown of a variable-length path query
  between Neo4j 5.18 and 5.26.2 ([#13585](https://github.com/neo4j/neo4j/issues/13585))
  and an eagerness-analysis regression in 5.19 where a benign write query
  "would either hang or lead to an out-of-memory error during planning"
  ([changelog](https://github.com/neo4j/neo4j/wiki/Neo4j-5-changelog)) are
  the reason A2 and A8 pin versions and run the same queries across releases.
- **Rust-native systems have their own weak spots**, and the benchmark must
  find them: SurrealDB's long-criticized performance and TiKV overhead
  ([discussion](https://github.com/orgs/surrealdb/discussions/3413)), Kuzu's
  single writer, FalkorDB's per-graph serialization, LanceDB's non-transactional
  graph, Turso's stringly-typed conflict errors (the LakeCat spike showed the
  typed `transaction()` API stays single-writer under `journal_mode=mvcc`
  until `BEGIN CONCURRENT` is issued explicitly), and Grust's structural edge
  keys collapsing parallel edges on some backends. A4, A5, and A7 are aimed at
  exactly these.

The benchmark does not predict a winner. It states where each architecture's
documented failure modes lie and measures whether they occur under the same
strain; what the numbers say is what the report says.

---

## 6. Harness plan

```
adversarial-graph/
  ADVERSARIAL-GRAPH.md        this document
  Cargo.toml                  workspace: grust-graph 0.13.0 + backend features
  crates/ag-core/             dataset loaders (SNAP edge lists, LDBC CSV, ICIJ),
                              oracle (GraphIndex), scenario engine, HdrHistogram,
                              open-loop scheduler, RSS/cold-start probes, report
  crates/ag-backends/         one adapter per Grust store + external adapters
  crates/ag-cli/              `ag fetch | load | run | report`
  scripts/fetch-datasets.sh   tiers S/M (default) and L (--large), SHA-256 manifest
  compose.yaml                digest-pinned services (Postgres, Falkor, Surreal,
                              pgGraph, Neo4j, Memgraph) with CPU/memory limits
  scenarios/v1/*.json         A1–A12 definitions (dataset, parameters, gates)
  reports/                    immutable run bundles
```

Milestones:

1. **M1 (this session):** workspace, dataset fetcher and manifest, SNAP and
   LDBC loaders, oracle, scenarios A1 (fan-out), A2 (deep paths), A3 (policy
   bounds), A4 (hot-node contention), A7 (guarded-commit replay) for the
   memory and Turso backends, JSON report with hard gates.
2. **M2:** Postgres, PGQ, pgGraph, Surreal, Falkor, LanceDB, Sail adapters
   through the compose stack; A5, A6 (Elle-style checker), A8 (differential
   Cypher), A12 (cold start / RSS / open-loop tail).
3. **M3:** external adapters (Neo4j, Memgraph, Ladybug native, AGE) with
   vendor-documented tuning profiles; A9–A11 stack-integrity scenarios wired
   to `querygraph-memory`, LakeCat's outbox, and QueryGraph's proof bases.
4. **M4:** immutable evidence bundles, `RESULTS.md` generation, public review
   packet — the same posture as `catalog-bench` 2026-Q3.

Comparison with the in-tree Grust LSQB harness happens after M2; the two
were designed independently on purpose.

---

## 7. First results (M1, 2026-09-04)

Host: Apple Silicon laptop, `cargo build --release`, `grust-graph` 0.13.0,
Turso 0.7.2. These are correctness observations with latency context, not a
ranking; the Turso rows are single-process embedded stores, the memory row is
the reference.

**Smoke ladder** (wiki-Talk, roadNet-CA, web-Google truncated to 200k edges;
memory, turso-wal, turso-mvcc): 36 `pass`, 9 `unsupported` (A3 needs the
reference executor's materialized graph, A7 needs a `GraphCommitStore`),
**0 hard-gate failures**. A4 attached 4×25 edges to the hub from four handles
on every store with no lost or duplicated write; A7 replayed the same
idempotency key from eight concurrent Turso handles and produced exactly one
commit id on both WAL and MVCC.

**Full size, memory backend:**

| Dataset | Scenario | Observation | p50 |
|---|---|---|---|
| wiki-Talk (2.39M / 5.02M) | A1 fan-out | hub out-degree **100,022**; 1-hop layer matches oracle | 75 ms |
| wiki-Talk | A3 policy | 7/7 attacks refused (graph exceeds `max_graph_nodes`, refused before execution) | 11 µs |
| wiki-Talk | A4 hot node | 16 writers × 200 edges: 3,200 accepted, 0 conflicts, 0 lost | 39 µs |
| roadNet-CA (1.97M / 5.53M) | A2 deep paths | depth-50 BFS from vertex 0 reaches 20,242; 200-hop Cypher pattern refused by `max_path_length=4` | 205 ms |
| roadNet-CA | A3 policy | 7/7 refused | 4 µs |

**Resource accounting.** Every scenario and every load records client user
and system CPU and peak RSS (`getrusage`), the host 1-minute load average at
start and end, and — for containerized backends — the server container's
cumulative CPU (`cpu.stat usage_usec`) and current memory from its cgroup.
A wall-clock number is therefore always accompanied by the CPU actually
spent on it and by the contention it ran under; the M1 runs above were taken
on a host whose load average reached 30 (a concurrent benchmark container
and this harness's own Surreal load), so their wall times are upper bounds
and the CPU ratios are the comparable figures.

**Backend findings from the network smoke (2026-09-04/05).** Postgres (Grust
`PostgresGraphStore`, native `tokio-postgres`): wiki-Talk 200k-edge slice
loaded in 7.4 s; A1/A2/A4 pass with 0 gates. SurrealDB 3.2.4 through the
Grust SDK store (native WebSocket, not HTTP `/sql`): the server pins one core
and ingests on the order of 10²–10³ edges/s and *slows as the table grows*.
The adapter, not only the engine, is implicated: `grust-surreal` makes each
edge write idempotent as `DELETE <E> WHERE in = … AND out = …; RELATE …`
inside 500-statement transactions, and without an index on `(in, out)` that
`DELETE` scans the edge table, so a bulk load is O(E²). The fix belongs in the
adapter (an `in,out` index at bootstrap, or deterministic edge record ids so
`RELATE` alone is idempotent); until then Surreal loads are recorded with
`edges_per_s` and are not comparable to the other stores' load figures.

FalkorDB is a second adapter finding: `grust-falkor` 0.13 is write-only —
`get_node`, `get_edges`, and `traverse` return `Unsupported("… does not
implement reads yet")`, its native-Cypher escape hatch discards results, its
Redis calls are synchronous inside `async fn`s, and `put_edge` is one round
trip per edge. FalkorDB itself answers all of these through `GRAPH.QUERY`.
The harness therefore reads Falkor back through its own openCypher path
(`read_path = "harness-native-cypher"` in the report, `src/falkor_reader.rs`)
so the engine is measured; the portable-API gap is charged to the adapter.
Two more `grust-falkor` facts the harness has to work around: node labels are
lowercased through `schema_identifier` (`V` is stored as `v`, relationship
types are kept as given), and the id index is created only inside
`apply_schema`, which `put_graph` never calls — without it every edge
`MATCH … {id}` scans all nodes (38k edges in eight minutes on a 73k-node
slice). The harness creates the index at bootstrap, as it does for Neo4j,
so the engine and not the missing index is what gets measured.

**Neo4j 5.26 Community** (roadNet-CA 200k-edge slice, heap 3G / page cache
3G, host load ≈300): A1 fan-out pass in 2.4 s wall / 0.57 s server CPU; A2
depth-8 BFS pass in 3.9 s / 2.0 s; A4 hot node 4×25 writes pass with no
lost write in 4.3 s wall and **4.5 s server CPU — about 45 ms of server CPU
per single-edge `CREATE`** through Bolt with the id index present. Client
CPU stayed at ≈1 %, i.e. the wall time is the server and the round trips.

Known M1 limits, to be closed in M2: A2 starts from the lowest id (on
wiki-Talk that vertex sits in a one-node component, so the deep-path run is
trivial there); A4 is closed-loop (open-loop scheduling with coordinated-
omission correction is M2); Turso full-size loads are slow through per-batch
upserts (≈20k edges/s) and are run separately from the smoke ladder; only
the memory backend can run A3 because the reference executor needs a
materialized `Graph`.
