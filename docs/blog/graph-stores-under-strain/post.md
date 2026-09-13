# Graph stores under strain: fast answers are only part of the job

![Illustrated Bay Area infrastructure crossing the San Andreas Fault, with roads, power, fiber and water networks under strain.](../../../cover/san-andreas.png.PNG)

*The headboard illustrates infrastructure under stress. The experiments below test graph software on recorded datasets; they are not a seismic simulation.*

A graph store can answer quickly and still fail the workload around that answer. It may run out of memory while loading the graph, lose an acknowledged write under contention, or accumulate a long queue when requests arrive independently of its response time.

**The takeaway from our strain benchmarks is to qualify a graph store by the workload it completes correctly within a stated resource budget, then compare speed among those completed cases. Programming language alone does not predict that boundary.**

The [published strain benchmark](https://adversari.al/graph/strain) now brings together 302 runs in 29 evidence bundles, covering September 5–13, 2026. Its summary contains 1,028 latest cells across 15 backend routes and 14 graphs. A cell identifies a graph, backend, scenario, edge slice and configuration profile. It is one piece of evidence, not a certification of the whole engine.

Those distinctions change how the results should be read. A small sliced graph is not a completed whole-graph run. An unsupported operation is not a correctness failure. Passing three scenarios does not establish that a fourth, unrun scenario would pass. The [summary generator](https://github.com/querygraph/adversarial-graph/blob/ba20a55/scripts/render-strain-summary.py) derives these classifications from the published reports.

The benchmark asks six practical questions:

| Group | What must hold? |
|---|---|
| Loading | Can the store ingest the offered graph and preserve its edge count within the load budget? |
| Difficult reads | Do hub fan-out and deep walks return the same layers or reached sets as the oracle? |
| Write contention | When 100 writers attach edges to the same hub, does every write apply durably or receive a typed refusal? |
| Operability | How long until the first correct answer, how much memory remains resident, and what happens to response tails at 50 and 200 requests per second? |
| Policy and commits | Are unbounded requests refused in time, and do replayed guarded commits have exactly one durable effect? |
| Typed graphs | Do recursive deletes, isolation under contention, and schema-sensitive queries preserve their declared semantics? |

A pass requires all nine hard gates to remain zero: wrong answers, lost writes, duplicate durable mutations, isolation anomalies, policy bypasses, hangs without refusal, out-of-memory or crashes, unauthorized disclosure, and nondeterministic receipts. Latency cannot compensate for a fired gate. The [harness and scenario specification](https://github.com/querygraph/adversarial-graph/blob/ba20a55/ADVERSARIAL-GRAPH.md) make those obligations inspectable.

The most useful scale measure in the current summary is deliberately strict. A whole untyped graph must load, and all four core scenario families must pass: hub reads, deep walks, contended writes and operability—A1, A2, A4 and A12. Here are selected results under the standard profiles:

| Backend route | Largest graph with all four core families passing | Edges |
|---|---|---:|
| Neo4j over Bolt | com-Orkut | 117,185,083 |
| Neo4j over HTTP | com-Orkut | 117,185,083 |
| Turso WAL | GAP-road | 57,708,624 |
| grust-memory | soc-Pokec-relationships | 30,622,564 |
| LanceDB | cit-Patents | 16,518,948 |
| SurrealDB, HTTP and SDK routes | ego-Facebook | 88,234 |
| HelixDB, HTTP and SDK routes | ego-Facebook | 88,234 |

These are the largest qualifying graphs in this evidence set, not maximum supported graph sizes. Graph structure changes between rows. Standard server runs use 6 GiB containers; embedded engines run inside the harness process under its host memory guard. Runs span different machines. The table therefore records completed coverage under disclosed conditions, not an equal-memory capacity contest. Each underlying cell is linked from the [strain matrix](https://adversari.al/graph/strain).

Turso WAL and grust-memory also have com-Orkut runs where the families that ran passed. They do not receive the same all-four designation because coverage was incomplete. Keeping that distinction prevents an early successful run from becoming a claim about work that never happened.

This also explains why “does Rust take more strain?” has no single language-level answer. The measured Rust routes include embedded engines, an in-process graph implementation, and database servers behind HTTP or SDK adapters. Their allocation, persistence, query execution and transport costs differ substantially. Some in-process routes record lower operation times than Neo4j in paired successful cases. Skipping a network round trip is part of that execution model; it is not evidence that rewriting a server in Rust would reproduce the difference.

On shared hosts, recorded wall times are upper bounds. CPU measurements, host load, architecture, resource limits and execution paths must accompany any timing comparison. A broad language ranking would discard the information needed to explain the measurements.

The larger-memory experiments make another useful distinction. Increasing selected server containers from 6 GiB to 24 GiB allowed Memgraph to load GAP-road and pass all four core families. FalkorDB also loaded GAP-road at 24 GiB and passed the families reached, but that run remained incomplete. Those outcomes belong in separate categories, and the earlier failures remain in the evidence.

More memory did not resolve every load failure. In the September 13 follow-up, both SurrealDB routes still ended with the container marked `OOMKilled` while loading the complete wiki-Talk graph: 2,394,385 nodes and 5,021,410 edges. The reports record the 24 GiB profile, the failed load and the corresponding hard gate. They also disclose dirty source paths alongside the recorded Git revision, so these should not be described as clean-checkout reproduction receipts. The [HTTP report](https://adversari.al/evidence/strain/2026-09-13-quegee-24g/20260913T144930Z/report.json) and [SDK report](https://adversari.al/evidence/strain/2026-09-13-quegee-24g/20260913T151125Z/report.json) preserve the details.

That observation identifies a failure in those tested versions, routes and resource profiles. It does not establish whether the dominant allocation came from graph representation, indexes, request handling or another subsystem. That requires further profiling. It also does not justify treating every other graph of five million edges as equivalent to wiki-Talk.

A refusal needs equally careful reading. In the contention scenario, a store may safely refuse writes with typed conflicts and still pass. That means it preserved the contract; it does not mean it accepted the same amount of work as another store. Acceptance, refusal and latency must be read together. Likewise, bounded-read policy tests apply to the reference executor, and guarded-commit tests apply only where that capability exists. An `unsupported` cell exposes scope rather than concealing a failure or granting a pass.

For an engineering decision, start with the graph shape and operations the application actually needs. Require complete scenario coverage, check the deployed adapter and transport, and inspect how the system behaves when it reaches its limit. Only then use latency and resource consumption to distinguish the qualifying configurations.

The practical lesson is simple: **a fast answer is useful only inside a workload whose correctness, durability and failure behavior you have also measured.** The strain benchmark supplies those separate observations and preserves their boundaries. Its [results and receipts](https://adversari.al/graph/strain) can be inspected independently of the sibling [query](https://adversari.al/graph/queries) and [graph-algorithm](https://adversari.al/graph/algorithms) experiments; none contributes to a blended score.
