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

### 2.4 Loaders (added 2026-09-06)

Three loaders in `src/dataset/` produce the same `grust::Graph`, so a backend
and the in-process oracle always see one multiset of nodes and edges:

- **SNAP edge lists** (`snap-edge-list`): one node label `V`, one
  relationship type `E`, exact duplicate edges dropped and counted,
  self-loops kept and counted, `--limit-edges` truncates and keeps only the
  nodes a loaded edge references.
- **LDBC SNB Interactive v1, CsvBasic with the long date formatter**
  (`ldbc-snb-csvbasic`; `ldbc-snb-sf0.1`, `ldbc-snb-sf1`): the archive is
  extracted beside itself on first use (`tar --zstd`). Node ids are
  namespaced by label (`Person:933`) because SNB ids repeat across entity
  types; the raw id stays as the `id` property. `organisation` and `place`
  split into Company/University and Continent/Country/City by their `type`
  column, as the LSQB projected-FK layout does; relationship types are the
  file's verb in upper snake case (`KNOWS`, `HAS_CREATOR`, `REPLY_OF`);
  millisecond-epoch columns (`creationDate`, `birthday`, `joinDate`) become
  RFC 3339 `DateTime` values, integers `Int`, and the multi-valued attribute
  files become string arrays (`speaks`, `email`). SF0.1: 327,588 nodes in 11
  labels, 1,477,965 edges in 15 types, 4.7 s to load, no dangling edges.
- **ICIJ Offshore Leaks** (`icij-offshore-leaks`): the CSV zip read in
  place. Labels are the file's kind (Entity, Officer, Intermediary,
  Address, Other); `node_id` is unique across files and is both the node
  id and the `id` property; `rel_type` becomes the relationship type
  (`OFFICER_OF`, `REGISTERED_ADDRESS`, `SAME_AS`, …) with `link`, `status`,
  the dates and `sourceID` as properties; every other column is a string,
  empty cells absent. Full graph: 2,016,523 nodes in 5 labels, 2,901,722
  edges in 14 types, 17 s to parse; the in-process copy plus the Memory
  backend's copy exceed the 15 GiB EC2 host, so the full graph runs on the
  laptop and this host runs slices (200,000 edges: 209,019 nodes, 1.9 s).

Every load records its format, the duplicate, dangling and self-loop
counts, the truncation point, and the label and relationship-type counts
(`load` and `schema` in `report.json`). The oracle carries that schema and
can restrict its hub, k-hop and BFS answers to one relationship type
(`EdgeFilter::Label`), which is what the typed families use; the M1
families (A1–A7) are defined over the single-label SNAP shape and report a
typed dataset as `unsupported`, never as a pass.

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
| Grust `SurrealHttpGraphStore` and `SurrealSdkGraphStore` | SurrealDB 3.2.4 | two backends, `surreal-http` (SurrealQL over `/sql`) and `surreal-sdk` (native WebSocket through the `surrealdb` crate), so the transport is measured, not assumed |
| Grust `FalkorGraphStore` | FalkorDB 4.20.4 | write path only through `GraphStore` (reads return `Unsupported`); native Cypher for reads |
| Grust `LanceDbGraphStore` | embedded | load/read only |
| Grust `LadybugGraphStore` | embedded LadybugDB (the `lbug` 0.20.2 crate) | Grust's internal adapter, `publish = false`, reached through a `git` dependency on the same Grust release tag as the published crates; on-disk under the run's work directory, untyped mode |
| Grust `HelixHttpGraphStore` and `HelixSdkGraphStore` | HelixDB (`enterprise-dev`, digest-pinned) | same internal-adapter route as Ladybug; two backends, `helix-http` (dynamic queries posted to `/v1/query`) and `helix-sdk` (the `helix-db` client crate, pinned `=2.0.0` by the adapter) against one container |
| Grust `SailGraphStore` | Sail Spark Connect, pinned rev | Cypher pushdown |
| Neo4j 5.26 Community | Bolt (`neo4rs`, `src/neo4j.rs`) and the HTTP Query API (`POST /db/neo4j/query/v2`, `src/neo4j_http.rs`) | two backends, `neo4j` and `neo4j-http`, sharing Cypher, labels, batching and the `(:V {id})` index; heap 3G / page cache 3G / `db.memory.transaction.total.max` 1G in an 8 GiB container (`compose.yaml`); `read_path = harness-native-cypher` |
| Memgraph 3.12 | Bolt through the same harness-side store as Neo4j (`src/neo4j.rs`, `BoltDialect::Memgraph`) | backend `memgraph`; the session database is `memgraph`, the id index is `CREATE INDEX ON :V(id)`; `--memory-limit` 6 GiB in `compose.yaml`; `read_path = harness-native-cypher` |
| Apache AGE 1.8 on PostgreSQL 18.6 | PostgreSQL wire protocol through AGE's `cypher()` table function (`tokio-postgres`, `src/age.rs`) | backend `age`; parameters travel as an `agtype` map in text format and results are cast to `text` (AGE has no binary `agtype`); bootstrap creates the graph, the `V`/`E` labels, a GIN index on `V.properties` (what AGE's planner uses for `{id: $id}`) and B-tree indexes on `E.start_id`/`E.end_id` (AGE keeps edges in a plain heap, so without them a one-hop read is a sequential scan of the edge table); a 16-connection round-robin pool; `read_path = harness-native-cypher` |

### 4.2 Report contract

Where a system offers both an HTTP API and a Rust client, the harness runs
both as separate backends and records `transport` in every LOAD row
(`http-sql`, `rust-sdk-ws`, `http-json`, `rust-sdk-http`, `bolt`,
`http-query-api`, `pg-wire`, `resp`, `embedded`). Same engine, same adapter
logic, same container: a difference between the pair is the wire and the
client, nothing else.

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

**FalkorDB 4.20.4 through the harness-native load and read path** (200k-edge
slices, host load ≈640): roadNet-CA A1/A2/A4 pass with 95 / 192 / 1,509 ms of
server CPU; wiki-Talk A4 (4×25 hot-node writes) pass at 3.3 s server CPU. And
one **hard-gate failure that is the benchmark working as intended**: on
wiki-Talk's 12,215-neighbour hub, A1 returned exactly **10,000** rows. The
image ships `RESULTSET_SIZE 10000`, which silently truncates every result set
at 10,000 rows with no error and no warning — a `wrong_answer` at default
configuration. With the documented knob set to `-1` the full 12,215 come
back; the tuned-profile rerun (`reports/20260905T060047Z`) passes A1, A2, and
A4 with 0 gates. Both profiles are kept: the defaults row stays a failure in the report,
and the tuned profile (`FALKOR_RESULTSET_SIZE=-1` in `compose.yaml`) is the
one comparable with the other stores, exactly as Neo4j's memory tuning is.

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

### 7.1 Clean-host results (M1, 2026-09-05, dedicated EC2 host)

Host: `lakecat`, Debian 13, 4 vCPU, 15 GiB, no swap; one system under test
resident at a time (`scripts/run-ladder.sh`), every container capped at
4 CPUs and 6 GiB; 1-minute load average 1–2 throughout, against 300–700 for
the laptop rows in §7. Same binary features, `grust` 0.13.0. Rows are
200k-edge slices except where marked 10k; the full table is `RESULTS.md`
(`x86_64/4` rows). Every containerized row carries the server's cumulative
CPU from the Docker Engine API, so "server CPU" below is the engine's own
cost, not the harness's.

**Eight-way at 200k edges, wiki-Talk (hub out-degree 12,215 in the slice)
and roadNet-CA.** A1 is the hub's 1-hop layer, A2 the bounded deep
traversal, A4 4 writers × 25 edges on the hub (p50 per write).

| Backend | Load edges/s (wiki / road) | A1 p50 · server CPU | A2 p50 wiki / road | A4 p50 · p99 · server CPU | Gates |
|---|---|---|---|---|---|
| memory | 330k / 653k | 24 ms · — | 0.0 / 0.7 ms | 0.0 · 0.1 ms · — | 0 |
| turso-wal | 15.3k / 24.5k | 72 ms · — | 0.3 / 11 ms | 0.1 · 11 ms · — | 0 |
| turso-mvcc | 8.3k / 10.6k | 73 ms · — | 0.4 / 15 ms | 23 · 64 ms · — | 0 |
| postgres | 17.4k / 20.6k | 115 ms · 89 ms | 1.7 / 62 ms | 5.0 · 7.4 ms · 126 ms | 0 |
| falkor (`RESULTSET_SIZE -1`) | 26.7k / 30.4k | 54 ms · 57 ms | 2.8 / 98 ms | 87 · 125 ms · 3.19 s | 0 |
| falkor (image default 10,000) | 26.8k / 30.6k | **10,000 rows, wrong_answer** | 3.1 / 94 ms | 87 · 125 ms · 3.19 s | 1 |
| neo4j (Bolt) | 16.4k / 26.6k | 385 ms · 1.02 s | 72 / 246 ms | 17 · 126 ms · 2.37 s | 0 |
| neo4j-http (Query API v2) | 19.9k / 35.1k | 426 ms · 1.25 s | 86 / 523 ms | 28 · 173 ms · 3.88 s | 0 |
| lancedb | 12.9k / 16.4k | 2.44 s · — | 253 ms / 9.1 s | 251 · 290 ms · — | 0 |
| ladybug (adapter at v0.13.0; rewritten in §7.2) | **11** / not reached | 19.5 s · — | 14 ms / — | 38 · 974 ms · — | 0 |
| surreal-sdk (10k) | 20 / 23 | **parse error** (both) | 236 ms / 11.2 s | 92 · 123 ms · 9.42 s | 1 |
| surreal-http (10k) | 21 / 23 | **parse error** (both) | 407 ms / 20.5 s | 148 · 176 ms · 14.8 s | 1 |
| helix-http (10k; 200k **408**) | 33 / 26 | 54 ms · 43 ms | 30 ms / 1.94 s | 303 · 455 ms · 5.46 s | 1 (LOAD) |
| helix-sdk (10k; 200k **408**) | 33 / 27 | **SDK read rejected** | — | — | 4 |

The contended laptop rows were upper bounds; these replace them as the M1
baseline, and the ordering they suggested holds. The embedded stores answer
the hub in tens of milliseconds and the deep path in microseconds to
milliseconds; the servers pay one to two orders of magnitude for the round
trips, and the CPU column shows where that cost lands: Neo4j spends 1 s of
server CPU on a 12k-row 1-hop and 2.4 s on 100 single-edge Bolt `CREATE`s
(24 ms each), FalkorDB 3.2 s on the same 100 writes (32 ms each, with
`RESULTSET_SIZE` irrelevant to writes), Postgres 0.13 s (1.3 ms each). The
FalkorDB truncation from §7 reproduces exactly on the clean host — 10,000
rows returned for a 12,215-row layer, no error, no warning — and its rows
now carry `profile` so both configurations stay visible side by side.

**New findings, all in adapters or defaults rather than in the engines'
traversal code.**

*SurrealDB, both transports, wiki-Talk A1: `Exceeded expression recursion
depth limit`.* `grust-surreal`'s `get_nodes` fetches a batch of ids as one
`WHERE id = type::record(t, id) OR id = type::record(t', id) OR …` chain, two
terms per id (it tries both the untyped `record` table and the label table),
and SurrealDB 3.2.4's parser rejects the chain at the hub's neighbour count
(the error is at character 4,812 of the statement). roadNet-CA's small
neighbourhoods pass. The fix belongs in `surreal_get_nodes_query`: `WHERE id
IN [...]`, or selecting the record ids directly. The 10k-edge load figure
did not improve on the idle host — 20–23 edges/s, server pinned on one core
for eight minutes — which confirms the O(E²) `DELETE … WHERE in= AND out=`
diagnosis from §7 rather than contention.

*LadybugDB: 11 edges/s.* `grust-ladybug`'s `put_graph` opens one
transaction and then executes one prepared statement per node and per edge,
so the 200k-edge wiki-Talk slice took 5.1 hours to load, with the harness
process at 6.3 GB peak RSS (the engine's default buffer pool, which the
adapter does not let the caller size). Once loaded, A2 and A4 pass with 0
gates and A2 answers in 14 ms, but the hub's 1-hop takes 19.5 s. The engine
has a `COPY FROM` bulk path the adapter does not use; the load figure is
charged to the adapter, the 1-hop time to the engine-through-adapter read
path and is worth a native-Cypher comparison in M2.

*HelixDB: 408 on the 200k slice, both transports.* `grust-helix` writes a
batch of edges as, per edge, `N … NWhere id = <to>` and `N … NWhere id =
<from> · AddE`. Node batches of 500 commit in 0.3 s each (`hyperscale` logs
`vertices_added=500`), but each edge is two property filters over every
node, and the first 500-edge batch on a 145k-node slice never returns
before the gateway's `request_timeout=30s`, so the client gets `408 Request
Timeout` and the server never commits an edge. The harness now creates a
runtime `NodeEquality` index on `(V, id)` at bootstrap, as it does for
FalkorDB and Neo4j; the server accepts it and the outcome does not change,
so whether `NWhere` consults runtime indexes is an open question for the
Helix side. On a 10k-edge slice the load completes at 33 edges/s with the
server saturating all four cores, and `helix-http` then passes A1, A2, and
A4 with 0 gates and the lowest server CPU of any network store on the
1-hop (43 ms). `helix-sdk` loads identically but every read fails before
reaching the server: `invalid Helix SDK read: unknown variant `Read`,
expected `read` or `write`` — the adapter's SDK path and the `helix-db`
2.0.0 client disagree on the request-type enum's casing.

**HTTP versus SDK, same engine, same container, same slice.** Both
transports of each pair ran back to back with nothing else resident.

| Engine | Metric | HTTP | SDK / Bolt | Ratio |
|---|---|---|---|---|
| SurrealDB 3.2.4 (10k) | load edges/s (wiki / road) | 21 / 23 | 20 / 23 | 1.0 |
| | A2 p50 wiki / road | 407 ms / 20.5 s | 236 ms / 11.2 s | 1.7–1.8× slower over HTTP |
| | A4 p50 · p99 | 148 · 176 ms | 92 · 123 ms | 1.6× |
| | A4 server CPU (100 writes) | 14.8 s | 9.4 s | 1.6× |
| Neo4j 5.26 (200k) | load edges/s (wiki / road) | 19.9k / 35.1k | 16.4k / 26.6k | HTTP 1.2–1.3× faster |
| | A1 p50 · server CPU | 426 ms · 1.25 s | 385 ms · 1.02 s | 1.1–1.2× |
| | A2 p50 wiki / road | 86 / 523 ms | 72 / 246 ms | 1.2–2.1× |
| | A4 p50 · p99 · server CPU | 28 · 173 ms · 3.88 s | 17 · 126 ms · 2.37 s | 1.4–1.6× |
| HelixDB (10k) | load edges/s | 33 / 26 | 33 / 27 | 1.0 (same adapter statements) |
| | reads | pass | rejected by adapter | — |

Two things separate here. For SurrealDB the transport is a constant factor
on every read and write (the WebSocket SDK is 1.6–1.8× cheaper in both wall
time and server CPU), and it does not touch the adapter's load or the A1
parse failure, which are the same statements on both paths. For Neo4j the
answer depends on the operation: Bolt wins every read and the hot-node
writes by 1.2–2×, but the HTTP Query API v2 loads 20–30% faster because the
adapter's batched `UNWIND` Cypher is one HTTP body per batch while the Bolt
driver pays per-message framing. Helix's two transports share every
statement, so they load identically and differ only in the SDK's broken
read envelope.

**What the clean host changed.** No pass became a fail, and no fail became
a pass: the laptop's `wrong_answer` on FalkorDB and the Surreal load
pathology are exactly reproduced, and the five backends measured for the
first time (Ladybug, both Helix, Surreal HTTP, Neo4j HTTP) added four
adapter findings and no engine correctness failure. Wall times dropped as
expected — Postgres's roadNet-CA load from 16.9 s to 9.7 s, FalkorDB's
wiki-Talk A4 from 4.2 s to 3.0 s. Server CPU behaved differently per engine:
FalkorDB's A4 reproduced within 5% (3.3 s → 3.2 s on wiki-Talk, 1.5 s →
1.6 s on roadNet-CA, laptop arm64 image versus x86 here), while Neo4j's
roadNet-CA A4 fell from 4.5 s to 0.8 s, so a JVM store's CPU time is itself
inflated under contention (scheduling and cache pressure, not more work).
That is the argument from §1.4 for reporting CPU next to the load average,
not just next to wall time.

### 7.2 Grust store changes and their measured effect (2026-09-05, pinned run)

Section 7.1's map from scenario to Grust path (FABLE-TO-FABLE.md §4) says
where the embedded stores spend their time: `traverse` and `get_edges` for
A1/A2, `put_edge` for A4, `put_graph` for LOAD. Seven changes were made in
the Grust adapters (branch `fable/strain-adapter-reads`, merged to
`querygraph/grust` main, final revision `3840d152`; tests, clippy and fmt
green), and measured on this host against the 200k-edge slices with the
same harness build otherwise:

1. **Memory store: reads through the typed snapshot.** A load into an
   empty store builds the `TypedGraphIndex` that the indexed Cypher
   entrypoint already used; until the next write, `traverse` and
   endpoint-anchored `get_edges` walk its `u32` slot adjacency instead of
   the string-keyed B-tree edge maps. Any write invalidates the snapshot
   and reads fall back to the maps, so a workload that interleaves point
   writes and reads never rebuilds an index inside a read; the retired
   snapshot is freed on a detached thread so the write that invalidates it
   does not pay for the drop (the first attempt paid 53 ms on one A4
   write). Results and order are unchanged, with `Direction::Both`
   listing outgoing before incoming neighbours.
2. **Turso: one transaction per load, checkpoint after it.** `put_graph`
   ran one auto-committed statement per 500-row batch, about 700 durable
   commits for a 200k-edge slice; it now runs every batch inside one
   transaction. In WAL mode the load is followed by `PRAGMA
   wal_checkpoint(TRUNCATE)`: without it every read and write after the
   load paid to look through the log (A4 writes at 0.7–1 ms instead of
   0.1 ms), and a `PASSIVE` checkpoint, which leaves the log file in
   place, measured the same as none. MVCC mode gets no checkpoint (the
   engine gates it behind an experimental flag) and the load itself did not
   change there.
3. **`TypedGraphIndex`** measures its serialized size on first use rather
   than at construction (a full JSON encode of the graph that plain
   traversals never needed) and exposes `relationship_types()`.
4. **`GraphStore::traverse_ids`.** The harness's k-hop walk only ever used
   the ids of the nodes `traverse` returned, so the 12,215-neighbour hub
   read cloned 12,215 nodes it then discarded. The trait gains
   `traverse_ids`, defaulted through `traverse` so every adapter answers it
   identically, and the memory store serves it from the snapshot cloning
   ids only. The harness calls `traverse_ids` for every backend; the
   portable read path is unchanged for all of them.
5. **Ladybug: bulk load through registered Arrow tables.** `put_graph`
   wrote one `MERGE` per node and per edge, and before each of them ran a
   `CREATE … TABLE` attempt plus a metadata `MERGE` to resolve the row's
   table; four to five statements per edge at tens of milliseconds each is
   the five-hour load of §7.1. Tables are now resolved once per distinct
   label, and rows are grouped per table, registered as Arrow record
   batches (`create_arrow_table`, `create_arrow_rel_table`) and copied with
   one `COPY … FROM (MATCH …)` each, the pattern LadybugDB's own columnar
   LDBC generator uses. Ids and `(from, to)` pairs that already exist keep
   the per-row `MERGE`, so the upsert semantics are unchanged.
6. **Ladybug: one query per relationship table per traversal step**,
   `MATCH (a)-[r]->(b) WHERE a.id = $id RETURN b.id, b.props`, instead of
   one prepared point lookup per neighbour; `traverse_ids` asks for ids.
7. **Ladybug: buffer pool cap and multi-writer option.** The engine sizes
   its buffer pool from host RAM; the adapter now exposes the cap
   (`buffer_pool_bytes`, the harness sets 4 GiB and records it as the row's
   profile) and the engine's multi-writer mode (`concurrent_writes`, off by
   default as in the engine), under which writers use their own
   connections instead of queueing on the adapter's lock.

**Measured** with the harness pinned to that revision (`Cargo.toml` git
`rev` for the internal adapters and a `[patch.crates-io]` of `grust-core`,
`grust-memory`, `grust-turso` and `grust-ladybug` to the same revision; harness `6b4b08c`,
bundles `20260906T0626…` onwards, each report stamped with both
revisions). "Before" is the morning's clean-host run at v0.13.0 (§7.1);
the ten other backends were rerun under the same pin and every one of their
cells reproduced within a few percent, with no outcome changed and the
same hard-gate total.

| Backend | Cell | Before | After |
|---|---|---|---|
| memory | LOAD wiki-Talk / roadNet-CA | 606 / 306 ms | 833 / 396 ms (index build inside the load) |
| memory | A1 p50 wiki-Talk hub (12,215 rows) | 23.9 ms | 2.7 ms |
| memory | A1 p50 roadNet-CA | 35 µs | 10 µs |
| memory | A2 p50 roadNet-CA depth-8 / wiki-Talk | 682 / 10 µs | 114 / 6 µs |
| memory | A4 p50 · p99 wiki-Talk | 6 · 81 µs | 4 · 741 µs |
| turso-wal | LOAD wiki-Talk / roadNet-CA | 13.06 / 8.16 s | 6.38 / 5.22 s |
| turso-wal | A1 p50 wiki-Talk / roadNet-CA | 72.3 ms / 268 µs | 58.5 ms / 376 µs |
| turso-wal | A2 p50 wiki-Talk / roadNet-CA | 333 µs / 11.4 ms | 310 µs / 10.9 ms |
| turso-wal | A4 p50 wiki-Talk / roadNet-CA | 124 / 19 µs | 120 / 103 µs |
| turso-mvcc | LOAD wiki-Talk / roadNet-CA | 24.0 / 18.8 s | 24.8 / 18.3 s |
| turso-mvcc | A1 p50 wiki-Talk / roadNet-CA | 73.0 ms / 339 µs | 71.4 ms / 1.0 ms |
| turso-mvcc | A4 p50 wiki-Talk / roadNet-CA | 23.4 / 21.8 ms | 24.1 / 23.6 ms |
| ladybug | LOAD wiki-Talk / roadNet-CA | 5 h 03 min (11 edges/s) / not reached | 23.3 s (8.6k) / 14.0 s (14.3k edges/s) |
| ladybug | A1 p50 wiki-Talk hub / roadNet-CA | 18.9 s / not reached | 39.5 ms / 24.1 ms |
| ladybug | A2 p50 wiki-Talk / roadNet-CA | 13 ms / not reached | 9.2 ms / 461 ms |
| ladybug | A4 p50 · p99 wiki-Talk (100 hub writes) | 37.5 · 974 ms | 37.2 · 978 ms |
| ladybug | peak RSS | 6.3 GB | 0.73 GB |

Every cell passed with 0 gates before and after; no outcome changed. Two
cells moved the other way and are stable across three runs, so they are
recorded as costs of the change, not noise: Turso WAL single-edge writes on
roadNet-CA's low-degree hub went from 19 µs to about 100 µs after the
single-transaction load and truncating checkpoint (the same writes on
wiki-Talk's hub are unchanged), and Turso MVCC's roadNet-CA 1-hop went from
0.34 ms to 0.6–1.0 ms across runs. Both are sub-millisecond cells on a
single-sample or 100-sample scenario; the load halving and the hub-read
gains are the changes' effect, and the memory store's load now carries the
index build it previously did not have (about 230 ms at 200k edges).

Ladybug's multi-writer mode (`concurrent_writes=true`, the engine's
`enable_multi_writes`, run as a second profile) does not make the hot-node
writes faster; it changes what they return. With four writers on their own
connections the engine accepted 34 of the 100 wiki-Talk hub writes and
54 of 100 on roadNet-CA and refused the rest as write-write conflicts, in
1.3 s and 1.8 s of wall time (p50 2.0 ms on wiki-Talk because a refusal is
cheap, p99 62 ms); every accepted write was visible afterwards and the
row passes with 0 gates, the refusals being typed. The serialized default
accepts all 100 at 37 ms each. That is the same shape Turso's WAL mode
showed in §7: a single-writer engine exposed to concurrent writers either
queues them or refuses them, and the harness records which.

The Neo4j pair was run twice in this ladder. The first run
(`20260906T070143Z`, `20260906T070245Z`) came out two to three times
slower on the wiki-Talk hub cells than every other measurement of the same
cells on this host (Bolt A4 74.8 ms p50 and 7.15 s server CPU against
15–17 ms and 2.3–2.5 s; HTTP A1 1.27 s against 0.44–0.46 s), at the same
recorded load average of about 2. The rerun twenty minutes later
reproduced the earlier values. Both runs are in the publication's bundle;
the later one is the row per cell, and the first is listed here as what
it is: a same-host outlier the ledger keeps rather than discards.

The Ladybug rows are the rewrite's measurement at revision `3840d152`
(harness `6b4b08c`); the memory and Turso rows are from the same ladder,
which reproduced their earlier values. The single-statement hub write is
the one Ladybug cell no adapter change moves: about 37 ms of engine CPU per
`MERGE … SET`, identical before and after, which is what the maintainer
note (`LADYBUG-NOTES.md`) hands to the engine's side.

One backend's outcome did change under the pin, for a reason outside the
adapter changes: `fdc685ee` sits on Grust `main`, which since v0.13.0 migrated
`grust-helix`'s SDK path to the `helix-db` 3.0.0 client's typed queries
(commit `7b12784`). Against the digest-pinned `enterprise-dev` image the
harness runs, that path now fails at its first request, the label drop in
`clear`, so `helix-sdk` cannot be opened at all and both datasets record a
failing LOAD row (`open failed: Helix SDK replace/drop failed`; the adapter
deliberately does not render the cause because the client's errors can carry
the configured URL). At v0.13.0 the same path loaded and then rejected every
read (§7.1). The LSQB harness qualifies the v3 SDK against a source-built
Helix server; the strain ladder will need that server image before
`helix-sdk` can be measured again. `helix-http` is unaffected.

The memory A4 p99 rose from 81 µs to a few hundred: the one write that
invalidates the snapshot spawns the thread that releases it. Before
`traverse_ids` (change 4) the hub read measured 15.8 ms, so of the original
23.9 ms about 8 ms was the map walk and about 13 ms the node clones; what
remains is 12,215 id clones and the harness's own visited set.

### 7.3 The host's CPU steal, and the rerun that records it (2026-09-06, evening)

The dedicated host is a burstable `t2.xlarge`. A Turso diagnostic cell
(LSQB, SF0.1) that started after two and a half hours of continuous compute
ran uniformly twice as slow as the same cell an hour earlier: worker setup
133 s instead of 71 s, q2 390 ms instead of 200 ms, q1 490 s instead of
260 s, with nothing else on the machine. `/proc/stat` carried 8.2 hours of
accumulated CPU steal over the 33-hour uptime, the hypervisor's share of
time the guest was denied once its credit balance was spent; after five idle
hours the same cell ran at the original speed with zero steal, and an A/B of
the two binaries agreed on every query. The load average the harness
recorded never moved, because steal is not runnable load.

Two changes followed. The instance was switched to Unlimited credit mode,
which bills the excess instead of throttling. And `src/probe.rs` now reads
the `steal` column of `/proc/stat` before and after every scenario and
records the delta as `host_steal_us` (summed over the vCPUs; compare with
`wall_us` times the vCPU count), so a throttled measurement shows in its own
row; `RESULTS.md` prints it in the Host column when it is non-zero. The LSQB
coordinator emits the same delta once per cell in its run log
(`host_cpu_steal`).

The whole ladder was then rerun from harness `b4659ad` at the same Grust pin
(`3840d152`): 17 runs, 174 cells, hard-gate total 9, in 53 minutes of wall
time against about ten hours before the Ladybug adapter rewrite. Total steal
across the 3,008 s of measured scenarios was 4.8 s, the largest single
reading 0.94 s during a Helix load. Every outcome and every gate reproduces
§7.1 and §7.2 exactly. It is published as `2026-09-06-unlimited` beside the
morning publication, which stays as it was: whether any of its runs were
throttled is not recoverable from the host (CloudWatch's `CPUCreditBalance`
for the instance would say), and the rerun makes the question moot for
every number that matters.
