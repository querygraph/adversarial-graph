# Graph stores under strain: embedded Rust matches Neo4j's reach and beats it on speed; memory is the gap

![Prometheus breaks the chains of a graph, scattering sparks between its nodes.](../../../cover/prometheus-breaks-graphs-headboard.png)

*Prometheus unbound is the sign of the graph benchmarks at adversari.al. The experiments below test graph software on recorded datasets under stated budgets and hard gates.*

**Grust's embedded Rust stores now match Neo4j's reach in the strain benchmark: Turso, in both of its journal modes, is clean on all four core scenario families up to com-Orkut, 117 million edges, the largest graph in the set, level with Neo4j over Bolt. In every same-machine pair where both passed, Turso's WAL store beats Neo4j on loading, hub fan-out, deep walks, cold start and tail latency. Neo4j's remaining advantage is memory: 2.5 to 3 times leaner at that scale. A rerun now under way on the harness's new default allocator has already cut the Rust stores' memory by up to 30% on the graphs it has reached.**

A graph store can answer quickly and still fail the workload around that answer. It may run out of memory while loading the graph, lose an acknowledged write under contention, or accumulate a long queue when requests arrive independently of its response time. The benchmark's rule has not changed: qualify a store by the workload it completes correctly within a stated resource budget, then compare speed among the completed cases. What changed between September 13 and September 16 is which stores complete the largest workload.

The [published strain benchmark](https://adversari.al/graph/strain) now brings together 331 runs in 32 verified evidence bundles, covering September 5–16, 2026, on five machines. Its summary holds 1,097 current cells across 15 backend routes and 14 graphs; the 805 cells that later runs superseded stay on the page in a history section, with the run that replaced each one. A cell identifies a graph, backend, scenario, edge slice and configuration profile. It is one piece of evidence, not a certification of the whole engine.

Those distinctions still govern how the results should be read. A small sliced graph is not a completed whole-graph run. An unsupported operation is not a correctness failure. Passing three scenarios does not establish that a fourth, unrun scenario would pass. The [summary generator](https://github.com/querygraph/adversarial-graph/blob/ab84d11/scripts/render-strain-summary.py) derives every classification below from the published reports, and the page opens with the verdict it computes.

The benchmark asks six practical questions:

| Group | What must hold? |
|---|---|
| Loading | Can the store ingest the offered graph and preserve its edge count within the load budget? |
| Difficult reads | Do hub fan-out and deep walks return the same layers or reached sets as the oracle? |
| Write contention | When 16 writers each attach 200 edges to the same hub, does every write apply durably or receive a typed refusal? |
| Operability | How long until the first correct answer, how much memory remains resident, and what happens to response tails at 50 and 200 requests per second? |
| Policy and commits | Are unbounded requests refused in time, and do replayed guarded commits have exactly one durable effect? |
| Typed graphs | Do recursive deletes, isolation under contention, and schema-sensitive queries preserve their declared semantics? |

A pass requires all nine hard gates to remain zero: wrong answers, lost writes, duplicate durable mutations, isolation anomalies, policy bypasses, hangs without refusal, out-of-memory or crashes, unauthorized disclosure, and nondeterministic receipts. Latency cannot compensate for a fired gate. The [harness and scenario specification](https://github.com/querygraph/adversarial-graph/blob/c13a2a8/ADVERSARIAL-GRAPH.md) make those obligations inspectable.

## Reach

The scale measure is deliberately strict. A whole untyped graph must load, and all four core scenario families must pass: hub reads, deep walks, contended writes and operability, A1, A2, A4 and A12, under the standard profiles.

| Backend route | Largest graph with all four core families passing | Edges |
|---|---|---:|
| Neo4j over Bolt | com-Orkut | 117,185,083 |
| Neo4j over HTTP | com-Orkut | 117,185,083 |
| Turso WAL | com-Orkut | 117,185,083 |
| Turso MVCC | com-Orkut | 117,185,083 |
| grust-memory | soc-Pokec-relationships | 30,622,564 |
| LanceDB | cit-Patents | 16,518,948 |
| SurrealDB, HTTP and SDK routes | ego-Facebook | 88,234 |
| HelixDB, HTTP and SDK routes | ego-Facebook | 88,234 |

On September 13 this table had Turso WAL at GAP-road and no Turso MVCC row above roadNet-CA. What moved them is work in the Grust adapter, measured on its [`turso-mvcc-concurrency` branch](https://github.com/querygraph/grust/pull/6): MVCC loads now run over four writer connections with a checkpoint between rounds, which took the MVCC load rate from 1,500 to 13,000–31,000 edges per second and its peak memory from 15 GB to 3–7 GB at web-Google scale; concurrent single-statement writes share one durable commit; and every load path runs with foreign keys off, as the in-memory reference does. The first MVCC run to reach com-Orkut at all is in this cohort.

These are the largest qualifying graphs in this evidence set, not maximum supported graph sizes. Graph structure changes between rows. Standard server runs use 6 GiB containers; embedded engines run inside the harness process under its host memory guard. The table records completed coverage under disclosed conditions, not an equal-memory capacity contest. grust-memory also loads com-Orkut and passes every family it ran there; it keeps the lower row because not all four families ran, and an early successful run must not become a claim about work that never happened.

## Speed, in same-machine pairs

A pair is one Rust route and Neo4j over Bolt that both passed the same scenario on the same graph, edge slice and machine, under the standard profiles. Rows from different machines are never compared. The [full pairing](https://github.com/querygraph/adversarial-graph/blob/c13a2a8/rust-vs-neo4j.md) is generated from the same evidence as the page; there are 166 pairs.

| Rust route | Beats Neo4j in every pair on | Median advantage |
|---|---|---|
| Turso WAL | loading (6/6), hub fan-out (6/6), deep walk (6/6), cold start (3/3), p99 at 200 req/s (4/4) | 1.5×, 6.5×, 7.6×, 2.0×, 4.0× |
| grust-memory | loading (5/5), hub fan-out (5/5), hot-node writes (5/5), cold start (3/3), p99 at 200 req/s (3/3) | 16×, 105×, 137×, 16×, 5.7× |
| Turso MVCC | hub fan-out (7/7), deep walk (7/7) | 3.8×, 3.9× |

On com-Orkut itself, same machine, no CPU steal: Neo4j loads at 16,800 edges per second and answers the hub fan-out in 144.5 s and the depth-7 walk in 4,082 s; Turso WAL loads at 24,666 and answers in 96.8 s and 943 s. The contended-write scenario is where the routes part. Neo4j and Turso MVCC accept all 3,200 writes, durably. Turso WAL accepts 61 of them and refuses the rest with typed conflicts, which passes, because a typed refusal is not a lost write, but is not the same work; its A4 time must be read with its acceptance count. Turso MVCC, which accepts every write, is slower than Neo4j on that scenario in every durable pair.

The Rust servers do not share in this. SurrealDB and HelixDB have pairs only on the two smallest graphs, because they do not load larger ones under the standard envelope; SurrealDB beats Neo4j only on hot-node writes and HelixDB's SDK route only on hub fan-out, and each loses every other pair. "Does Rust take more strain?" still has no single language-level answer: the routes that win run in process, with no network hop and no serialization, and part of every such margin is that architecture. What the September 16 evidence settles is narrower and firmer: the embedded Rust stores complete the largest workload the benchmark offers, and complete it faster.

## Memory

Memory is where Neo4j leads, and the comparison needs its caveat: a server reports its container's resident memory; an embedded store's only figure is the whole harness process, reference graph included. With that read into the numbers, at com-Orkut Neo4j peaks at 4.9 GB, Turso WAL at 12.4 GB and Turso MVCC at 14.5 GB on its durable four-writer lane, 22.5 GB with eight writers. At soc-LiveJournal1, Neo4j peaks at 6.7 GB and Turso WAL at 10.3 GB. Neo4j is 2.5 to 3 times leaner on the durable rows at the top of the ladder.

That gap is now moving. Every Turso row on the page ran on the platform allocator, because Grust's facade turns Turso's default features off and Turso's own default build uses mimalloc; the harness now sets mimalloc process-wide, tags such rows `alloc=mimalloc`, and is rerunning the in-process routes. The WAL ladder's first four graphs, taken today on the reference machine with the loads also carrying the foreign-key change, load 20–31% faster and peak lower: web-Google 3.3 → 3.2 GB, soc-Pokec 4.4 → 3.0, cit-Patents 6.8 → 5.1, GAP-road 10.8 → 7.8. Those rows are not yet on the page; the MVCC ladder and the other in-process routes follow, and the page's history will show what each replaced.

## Limits, refusals and scope

On shared hosts, recorded wall times are upper bounds. Three of the five machines are burstable instances whose CPU credits, once spent, show up as steal rather than as an error; every run in the cohort records its steal, comparisons on those hosts were taken as alternating pairs, and the absolute numbers above come from the non-burstable reference machine. A broad language ranking would discard the information needed to explain the measurements.

The larger-memory experiments make another useful distinction. Increasing selected server containers from 6 GiB to 24 GiB allowed Memgraph to load GAP-road and pass all four core families. FalkorDB also loaded GAP-road at 24 GiB and passed the families reached, but that run remained incomplete. Those outcomes belong in separate categories, and the earlier failures remain in the evidence.

More memory did not resolve every load failure. Both SurrealDB routes still ended with the container marked `OOMKilled` while loading the complete wiki-Talk graph, 2,394,385 nodes and 5,021,410 edges, at 24 GiB. The reports record the profile, the failed load and the corresponding hard gate, and disclose dirty source paths alongside the recorded Git revision, so they are not clean-checkout reproduction receipts. The [HTTP report](https://adversari.al/evidence/strain/2026-09-13-quegee-24g/20260913T144930Z/report.json) and [SDK report](https://adversari.al/evidence/strain/2026-09-13-quegee-24g/20260913T151125Z/report.json) preserve the details. That observation identifies a failure in those tested versions, routes and resource profiles; it does not say which subsystem allocated the memory, and it does not make every other five-million-edge graph equivalent to wiki-Talk.

An `unsupported` cell exposes scope rather than concealing a failure or granting a pass. Bounded-read policy tests apply to the reference executor. Guarded-commit replay runs only on stores with a guarded commit in their Grust adapter; Neo4j's row reads unsupported because the harness drives it through its own Cypher without one, a gap in the harness path, not in Neo4j, and it is not compared.

For an engineering decision, start with the graph shape and operations the application actually needs. Require complete scenario coverage, check the deployed adapter and transport, and inspect how the system behaves when it reaches its limit. Only then use latency and resource consumption to distinguish the qualifying configurations.

The practical lesson holds: **a fast answer is useful only inside a workload whose correctness, durability and failure behavior you have also measured.** As of September 16, the embedded Rust stores meet that bar at the benchmark's largest scale and answer faster there; Neo4j meets it with less memory. Its [results and receipts](https://adversari.al/graph/strain) can be inspected independently of the sibling [query](https://adversari.al/graph/queries) and [graph-algorithm](https://adversari.al/graph/algorithms) experiments; none contributes to a blended score.
