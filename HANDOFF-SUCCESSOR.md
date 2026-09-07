# Hand-off from the laptop session to quegee, its EC2 successor (2026-09-07)

The laptop that ran the taskmaster session for the adversarial graph
benchmarks travels from 2026-09-07 ~22:00 UTC and is no longer a host.
**quegee** (c5n.4xlarge, 16 vCPU, 40 GB, `ssh quegee`) takes its role:
coordinator of the four-host program,
site admission, the contended-baseline rows' replacement, and the
largest-memory cells. Read this, then `FABLE-TO-FABLE.md` §9 onward
(§24–§30 are the current state), then `AGENTS.md` (neutrality at every
level; no strategy talk about beating any vendor in anything committed).

## Bootstrap (done on quegee by the laptop; kept for the next host)

`scripts/bootstrap-host.sh` after the host's own Debian bootstrap (Docker
29 + Compose v5, as on `lakecat`, `grust`, `eigen`). It needs a GitHub key
on the box for the three private repositories (`querygraph/adversarial-
graph`, `adversarial-site`, `grust`): generate one on the host and add it
to the account as the eigen box's was (`~/.ssh/eigen`, title `eigen`).
Datasets: 2.3 GB from GDC/SNAP mirrors via `scripts/fetch-datasets.sh`,
or `rsync` from `grust:~/src/adversarial-graph/datasets/`.

Reaching the other hosts: the user's key `~/.ssh/gagarin.pem` is on
quegee (mode 0400) with `~/.ssh/config` entries `lakecat`, `grust`,
`eigen` over the private network (172.31.34.193, 172.31.35.136,
172.31.41.165; public 3.133.81.104, 18.117.218.138, 18.216.201.246).
quegee pulls their bundles with `rsync` into `reports-hosts/<host>/`.

## What the laptop held that nothing else did

On quegee under `~/src/adversarial-graph/reports-hosts/` (`laptop/`,
`lakecat/`, `grust/`, `eigen/`), and staged on the grust box at
`grust:~/handoff-laptop/`:

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
| quegee (this host) | 40 GB (c5n.4xlarge) | coordinator; site admission; the soc-LiveJournal1 tier for the container-backed backends (client ~23–26 GB + 6 GiB container); any embedded-store tier above web-Google (Turso MVCC cit-Patents reached 26 GB and the cap; LanceDB wiki-Talk needed 42.5 GB); LSQB admissions |
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

## Memory on quegee: 40 GB, no swap, and the rule that keeps it standing

Every ladder here runs with `AG_RSS_LIMIT_GB=30` and
`AG_MEM_AVAILABLE_MIN_GB=2`: the client net leaves a 6 GiB container plus
the OS and Docker inside 40 GB, and the available-memory floor ends a
pair before the kernel does (the floor is what lakecat learned in §15;
two low readings five seconds apart, so a transient does not fire it).
A pair that trips either is a placement outcome, logged, never a row.
Known footprints against that net: the container-backed backends on
soc-LiveJournal1 need 22–23 GB of client (fits); the memory store on
com-Orkut 28.9 GB (fits, barely, and is already published); Turso MVCC
above web-Google 26 GB and rising (does not fit with headroom); LanceDB
wiki-Talk 42.5 GB (does not fit; item 4a). The two engineering items
that lower the client's footprint are the compact reference (§28) and
LanceDB's bulk batching (4a); until they land, the largest tiers of the
embedded stores are simply not run here, and the ledger says where each
row came from.

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

1. The SF0.3 matrix: blocked on Turso's resident index at the 6 GiB
   container budget (§32); choose and implement one of its two contract
   changes (declared `backend.memory-exceeded` termination, or a
   memory-bounded route fallback), then rerun on eigen inside its
   windows and admit it. Admit the native Neo4j SF0.3 lane when
   `~/eigen-native.log` on eigen says `EIGEN_NATIVE_DONE` and its audit
   passes (the site's `NATIVE_SERVER` pin must gain the amd64 image).
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
