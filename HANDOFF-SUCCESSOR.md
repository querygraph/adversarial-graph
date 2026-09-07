# Hand-off from the laptop session to its EC2 successor (2026-09-07)

The laptop that ran the taskmaster session for the adversarial graph
benchmarks travels from 2026-09-07 ~22:00 UTC and is no longer a host.
A new EC2 instance takes its role: coordinator of the four-host program,
site admission, the contended-baseline rows' replacement, and the
largest-memory cells. Read this, then `FABLE-TO-FABLE.md` §9 onward
(§24–§30 are the current state), then `AGENTS.md` (neutrality at every
level; no strategy talk about beating any vendor in anything committed).

## Bootstrap

`scripts/bootstrap-host.sh` after the host's own Debian bootstrap (Docker
29 + Compose v5, as on `lakecat`, `grust`, `eigen`). It needs a GitHub key
on the box for the three private repositories (`querygraph/adversarial-
graph`, `adversarial-site`, `grust`): generate one on the host and add it
to the account as the eigen box's was (`~/.ssh/eigen`, title `eigen`).
Datasets: 2.3 GB from GDC/SNAP mirrors via `scripts/fetch-datasets.sh`,
or `rsync` from `grust:~/src/adversarial-graph/datasets/`.

Reaching the other hosts needs the user's key (`~/.ssh/gagarin.pem`, user
`admin`, hosts `lakecat` 3.133.81.104, `grust` 18.117.218.138, `eigen`
18.216.201.246, all also on the Tailscale net as `*.tail693a26.ts.net`):
the user decides whether to place it on this host. Without it, the other
hosts push their bundles to a branch instead of the coordinator pulling
them.

## What the laptop held that nothing else did

Staged on the grust box at `grust:~/handoff-laptop/`:

- `reports-laptop/`: every laptop run bundle (the contended baseline;
  the published ones are in `adversarial-site` `public/evidence/strain/
  2026-09-07-laptop`, the superseded ones excluded there as §26 lists).
- `reports-hosts/{lakecat,grust,eigen}/`: the other hosts' bundles as
  pulled for admission (their `reports/` directories are the source of
  truth).
- `laptop-scripts/`: the ladder wrappers used here (`ladder-*.sh`,
  `rss-guard.sh`); the repo's `scripts/run-full-tiers.sh` is the real
  tool and has everything they did.

## Roles now (from §24, §28, §30)

| host | RAM | role |
|---|---|---|
| successor (this host) | 42 GB (c5n.4xlarge) | coordinator; site admission; the soc-LiveJournal1 tier for the container-backed backends (client ~23–26 GB + 6 GiB container); any embedded-store tier above web-Google (Turso MVCC cit-Patents reached 26 GB and the cap; LanceDB wiki-Talk needed 42.5 GB); LSQB admissions |
| eigen | 31 GB | the LSQB SF0.3 matrix and native Neo4j SF0.3 lane inside the tenancy windows (02:30–03:45, 12:30–13:30, 14:30–15:45 UTC); cit-Patents rows done |
| grust | 31 GB | idle; embedded stores through web-Google done and published; available for reruns (no LSQB unless told) |
| lakecat | 15 GiB | scenario work (A5, A6, A8 adapters, Helix SDK fix); clean-host slices; nothing above cit-Patents |

## In flight at hand-off

- Laptop: `neo4j-http soc-LiveJournal1` (started 17:42 UTC, cap 19:42)
  then `memgraph soc-LiveJournal1` (may not finish before departure; if
  its bundle is missing from `reports-laptop/`, run it here:
  `AG_RSS_LIMIT_GB=<host-8> scripts/run-full-tiers.sh --datasets
  soc-LiveJournal1 memgraph`). Publish both as a laptop addendum or as
  this host's first publication.
- eigen: the SF0.3 matrix under `~/eigen-lsqb.sh` (log `~/eigen-lsqb.log`,
  matrix log `benchmarks/lsqb/out/matrix-sf0.3-w2r10-<rev>-eigen.log`),
  then the native lane. Admission: `merge-reports.sh`, the publication
  receipt, `bundle-native-neo4j.py`, then the site's graph verifiers
  (`scripts/verify-graph-*.mjs`, `PUBLICATION_TRUST` pins, `KRILL_SAIL_
  SOURCES`, `NATIVE_SERVER` must gain the amd64 image `a9d46c94…`).
- lakecat: the four §26 reruns (offer again), the A6 AGE retake, the
  label-aware `get_node`/`put_node` adapters.

## Rules that held all week (and why)

- Placement, not limits: a guard (`AG_RSS_LIMIT_GB`, `AG_MEM_AVAILABLE_
  MIN_GB`, `AG_BLACKOUT_UTC`) records a host outcome, never a store
  finding; a killed run has no bundle and cannot be published as one.
- Commit first, then `git fetch && git rebase origin/main`, then push;
  on a FABLE-TO-FABLE conflict take origin's file and append your section
  with the next free number. Never edit another host's section.
- Never rewrite a running bash script in place; edit a copy and `mv` it.
- Watch long runs with coarse cell-level monitors and re-read the log at
  least every 30–45 minutes; a flooding monitor is silently stopped.
- Every bundle carries per-run provenance (`harness_revision`,
  `harness_dirty_paths`, `summary.complete`); the v2 site manifest
  repeats it and the verifier checks it. A base revision not on
  `origin/main` is refused.
- Neo4j: heap 2G + page cache 2G in the 6 GiB budget; Memgraph
  `--memory-limit` 5120 MB. Both corrections and their reasons are in
  §23; rows under the old values are superseded.

## Open work, in order

1. Admit eigen's SF0.3 matrix and native Neo4j SF0.3 to the graph ledger.
2. The soc-LiveJournal1 network rows: finish `memgraph`, then `age`
   (its A2 alone exceeds two hours at web-Google, so expect a cap in
   A2; the LOAD and A1 rows still count) and `postgres` (A2 capped on
   the laptop; same expectation).
3. A compact reference (CSR over interned ids) in the harness would cut
   the client's 26 GB several-fold and let the 15 GiB and 31 GB hosts
   hold soc-LiveJournal1; §28. Not started.
4. Ladybug via the Arrow bulk path (§16.1) and the Helix SDK casing fix
   (§9.3 item 6), both adapter work in `grust`.
4a. LanceDB's 42.5 GB client on wiki-Talk: `grust-lancedb` loads in
   batches of 500 rows, each an append to two in-process tables, so a
   5 M-edge graph is ~20,000 fragments per edge table whose manifests
   and metadata LanceDB keeps in the process. Raise the bulk-load batch
   size (tens of thousands of rows) and compact after the load, then
   rerun wiki-Talk under a guard to measure it; the laptop's published
   row stands until then. Hypothesis, not yet profiled.
5. A9–A11 (M3 stack integrity) remain unimplemented.
