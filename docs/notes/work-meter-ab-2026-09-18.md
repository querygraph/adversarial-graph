# Does the lock-free work meter move strain? Measured: no re-run

Answer to `adversarial-graph-algorithms/docs/strain-requalification-handoff.md`
(2026-09-18), which asked quegee to decide, from one cheap A/B, whether Grust
`23de753` ("Charge work and observe cancellation without the execution
mutex", branch `work/algorithms-performance`) changes any strain number.

## Which strain arms execute the patched code

The meter is reached only through `grust-cypher` reads. In the strain harness
those run in exactly one scenario: **A8**, the typed-graph differential, on
the in-process reference (`backends.rs` `cypher()` → `run_bounded_read_query_indexed`)
and on its Turso route. A3's policy checks refuse before execution, in
microseconds. **LOAD, A1, A2, A4 and A12 — every ranked metric and every
Neo4j pair on the strain page — never call it.** The change therefore cannot
move the leaderboard or the pairs; the most it can move is A8's recorded wall.

## The A/B

Host eigen (t2.2xlarge, 8 vCPU; quegee was mid-ladder, so only the relative
result is used; per-run steal 15–46 jiffies, i.e. nil). Harness `43ae409`,
two release binaries identical except for the Grust pin:

- base: `6af2006` — the strain pin; code-identical to `a04ebd7` plus the
  Ladybug bulk-copy change, which A8 on `memory` never touches;
- patch: `23de753` — the meter change on top of `3a73954`.

Both builds compiled `grust-core`, `grust-procedures` and `grust-cypher` from
their own checkouts (binary hashes `c7799afd…` / `a1770b7a…`). A8 on
`ldbc-snb-sf0.1`, backend `memory`, five rounds, order alternating per round,
capped at 16 GB. Every round: LOAD pass, A8 with 27 of 29 queries matched and
the same 2 (`r2-posts-per-creator`, `r5-reply-fanin`) refused at the 110 s
bounded-read budget on both binaries.

| | base | patch | patch / base |
|---|---:|---:|---:|
| A8 wall, median of 5 | 234.43 s | 234.58 s | 1.001 |
| the 27 completed queries, store side, sum of per-query medians | 12,490 ms | 12,752 ms | **1.021** |
| same, per-round sums: median (MAD) | 12,525 (64) | 12,764 (51) | +239 ms |
| the 27 completed queries, oracle side | 1,245 ms | 1,257 ms | 1.010 |

The A8 wall is dominated by the two budget timeouts (2 × 110 s), identical by
construction. On the queries that complete, the patched binary is **2.1%
slower**, uniformly (+1 to +5% on 22 of 27 queries; the rest are sub-millisecond
noise), and the separation is about four MADs, so it is small but real on
this host. The oracle side, which charges far less often, moves 1%.

One observable difference, contrary to the patch's "no caller observes a
behavioral difference": the refusal text for the two budget-exhausted queries
changes from `bounded read execution timed out` to `bounded read
candidate-work units (procedure execution timed out) while checking MATCH
relationship uniqueness` — cancellation is now observed at a different check
point. Same outcome (a typed refusal), same gates (zero), different note.

## Decision

- **No strain re-run.** Nothing the page ranks or pairs executes the meter;
  the one arm that does moves by 2% in the slower direction and is a
  correctness scenario whose wall is recorded, not ranked.
- **Do not move the strain pin to `23de753` on performance grounds.** If it
  is merged for the algorithms benchmark's sake, strain picks it up at its
  next pin bump like any other commit; the A8 walls will read ~2% higher and
  this note is the explanation.
- The algorithms result (72.8% of full-path Dijkstra in `charge_work`) is a
  per-entry workload; a row-oriented Cypher read charges orders of magnitude
  less often per unit of wall time, which is why the two benchmarks disagree
  in sign. Both numbers are right for their workloads; neither transfers.

Raw reports: eigen `~/meter-ab/{base,patch}-{1..5}/*/report.json`, script
`meter-ab.sh`, comparison `meter-ab-compare.py` (session scratch). Not
published evidence: a different host, an unranked arm, and a pin the page
does not use.
