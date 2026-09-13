

## 57. The strain page becomes a summary and one hierarchy (2026-09-13)

The user found the strain page a confusing mass of tables, each seemingly
crippled by something, and asked for executive summaries, the scenario
groups explained, whether Rust takes more strain than the rest, and every
table under hierarchical disclosure with clear meaning; then, seeing the
first pass, that the dated publications and the September 5 notice should
not be on the page at all.

The page is now generated from the published evidence by
`scripts/render-strain-summary.py`: an executive summary, the reach ranking,
the strain matrix, the Rust question with the head-to-head against Neo4j
over Bolt, the six scenario groups, and every latest cell (1,028) under
family, backend, tier and graph, each level with a verdict in words. The
typed and 24 GiB matrices and the family table fold under one-line
summaries that are asserted against the data. The ledger is gone; the
evidence pins sit under one toggle, as the site verifier requires. The
Prometheus banner heads all four graph pages (site `163a55a`).

On the strict measure (A1, A2, A4 and A12 all pass on a whole graph): Neo4j
over Bolt is clean up to com-Orkut (117.2 M edges); the best Rust store,
Turso WAL, up to GAP-road (57.7 M); SurrealDB and HelixDB up to ego-Facebook
(88.2 k). Where both passed on the same graph and machine, the in-process
Rust engines beat Neo4j consistently on speed, partly because they skip the
network hop.

Two errors caught before publishing: the first draft counted a graph as
sound when only some families had run (grust-memory's com-Orkut run
predates A12), which ranked it first; and one comparative sentence was
typed, not computed. Both are fixed. The comparison also sits in
`rust-vs-neo4j.md`, which the user asked to keep uncommitted.
