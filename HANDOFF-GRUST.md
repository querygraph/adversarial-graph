# Handoff: the `grust` box (8 vCPU, 31 GiB, 849 GB) in the adversarial graph benchmarks

Written 2026-09-06 22:30 UTC by the laptop session, for the Claude session
running on this host. The laptop session is the taskmaster for completing
both ledgers; the plan is `FABLE-TO-FABLE.md` §9 and this host's role is
§9.3a. Read `AGENTS.md` first: the benchmarks are neutral at every level.

## What is already on this host

- `~/src/adversarial-graph` (this repo) and `~/src/grust`, both clones of
  the public GitHub repos at today's heads. Pull both before doing anything.
- `datasets/`: the S and M tiers (17 files, 2.4 GB), synced from the laptop
  with their SHA-256 manifest. The L tier is not here and is deferred.
- The harness build was started by the laptop over ssh
  (`cargo build --release --features postgres,surreal,falkor,lancedb,neo4j,helix,ladybug`,
  log in `~/ag-build.log`); if `target/release/ag` exists, use it, otherwise
  rerun that command. `clang` was installed for the bindgen crates; cmake,
  protobuf-compiler, libssl-dev and pkg-config were already present.
- Docker 29.8 with compose; you are in the `docker` group. The crawler
  (`et`, eigentimes) is the host's other tenant: about a tenth of one core.
  Never stop or touch it.

## Your role

The third host. The laptop runs the fast stores' full-graph tiers and the
LSQB matrices; the EC2 box (`lakecat`) develops the new scenario families
and adapters. You run what neither can: the slow-loading stores at full
scale now, and correctness runs of the new families as they land.

## Queue, in order

1. **Full-graph tiers of the slow-loading stores.** `scripts/run-full-tiers.sh`
   runs one (backend, dataset) at a time, smallest dataset first, under a
   wall-clock cap, and stops a backend at the first tier that exceeds it:
   ```
   scripts/run-full-tiers.sh --cap 7200 --datasets wiki-Talk,roadNet-CA,web-Google,cit-Patents,soc-LiveJournal1 lancedb ladybug
   scripts/run-full-tiers.sh --cap 7200 --datasets wiki-Talk,roadNet-CA,web-Google surreal-sdk surreal-http helix-http helix-sdk
   ```
   Surreal and Helix were measured only at 10,000 edges so far (the Surreal
   adapter's load is O(E²); Helix SDK fails on a request-type casing bug in
   the pinned adapter, see below). Try them at full scale under the cap and
   record where they stop; a store that cannot load a tier inside two hours
   is a finding, written into the run's notes, not a gap. Bring services up
   with `docker compose --profile external up -d <service>` (the script does
   it) and export `BENCHMARK_CPU_LIMIT=8` here.
2. **Publish.** `python3 scripts/render-results.py`, then
   `scripts/bundle-site-evidence.py <site>/public/evidence/strain/<date>`
   is the laptop's job; from here, commit `reports/` and `RESULTS.md`, add a
   numbered section to `FABLE-TO-FABLE.md` saying which runs are new and
   what stopped where, and push. The laptop admits the bundle on the site
   and pins its digest.
3. **Correctness runs of the new families** once the EC2 session pushes
   them (A8 differential Cypher, A6 isolation, A5 recursive deletes, A12):
   every backend, slice scale first, then full where the cap allows. Their
   gates do not depend on timing, so this shared host is the right place.
4. **Gates for pushed changes**: after pulling a new harness or grust
   revision, `cargo test` for the crates that changed and the harness's own
   tests, before running anything on it.

Do not run the LSQB matrices (`~/src/grust/benchmarks/lsqb`) here unless
`FABLE-TO-FABLE.md` says so; the laptop owns those receipts for now. Do not
edit `~/src/grust` here; the EC2 session owns harness development, and the
laptop merges.

## Known findings to carry, not fix here

- Helix SDK: `serialization error: invalid Helix SDK read: unknown variant
  Read, expected read or write`; the pinned `grust-helix` sends `Read`
  where the server's SDK expects `read`. The EC2 session owns the fix
  (§9.3 item 6). Until then `helix-sdk` rows fail at LOAD; keep them, they
  are evidence.
- Surreal adapter loads edges one statement at a time without an
  `(in, out)` index; 6.9 edges/s at 10k on the laptop.
- Ladybug loads through per-element statements unless the adapter's Arrow
  bulk path is used; the EC2 session's `342c202` added the bulk load.
  Whether the harness's `put_graph` reaches it is exactly what your first
  tier will show.

## Rules

- One system under test at a time; the harness records host load and steal
  on every row, and the crawler is part of that load, disclosed.
- Results are generated (`render-results.py`), never hand-edited; failures
  stay visible; unsupported is never a pass.
- Pull all three repos before touching any (`~/src/adversarial-graph`,
  `~/src/grust`; the site is the laptop's); write a numbered section in
  `FABLE-TO-FABLE.md` on every handoff; never rewrite what another session
  wrote there.
- Neutral framing everywhere, including commit messages and notes.
