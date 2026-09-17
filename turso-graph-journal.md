# Turso under adversarial-graph: a working journal

> **How to read this.** This is the lab journal of the Turso work between
> 2026-09-10 and 2026-09-17, kept in the order things were learned. Later
> sections supersede earlier ones where they disagree (§3's A4 correction
> applies to every `synchronous=NORMAL` A4 figure, including §8's; §5's
> boxed note supersedes asks 2 and 3 below it; §7b resolves the foreign-key
> defect §3 raises). The current summary is the book chapter "Turso under
> strain" in Grust (`docs/book/chapters/turso-under-strain.md`) and the body
> of Grust PR #6; the Rust-vs-Neo4j pairing the site counts is
> `rust-vs-neo4j.md`, generated. Final revisions: Grust `a04ebd7`
> (`turso-mvcc-concurrency`), harness `0636f4f`, site `b5356f8`.

Status: 2026-09-16. Grust `turso-mvcc-concurrency` @ `e5d7b4e` (PR #6), harness
`main` @ `e4ef441`. **Turso 0.7.2** (crates.io) throughout -- see the box in §5
before treating any of this as current; upstream `main` is 1,859 commits ahead
and has already landed engine-level group commit. The rerun against `main`
completed 2026-09-17 13:40Z: same host, same config, zero steal, `main` loads
20-23% slower at 63-117 M edges and reads 4-24% faster (§3); the fastest fill
on `main` is through WAL (§3). Grust PR #6 is at `abea518`. Note also that the harness pins Grust at git rev `f6d3391`, four
commits behind `e5d7b4e`; the difference is `91f6e3f` (measured flat),
`48b4252` (reverted) and `618caa0` (tunables), so it does not move the numbers,
but the binary that produced them is not literally the PR head. Hosts: quegee (16 cores, 40 GiB), grust / eigen (8 vCPU,
31 GiB), lakecat (4 cores, 15 GiB). All numbers below are from real SNAP graphs
on real disks, not microbenchmarks or tmpfs.

## 1. The one-paragraph summary

Turso in MVCC mode is the only journal mode that survives concurrent writes to a
hot node: it accepts 3,200 of 3,200 attempted writes where WAL accepts 20-120.
WAL loads 1.5-1.7x faster. Everything we could fix at the Grust level, we fixed
(group commit, parallel loads, bounded checkpoint memory), which moved MVCC loads
1.9-9.5x and A4 write contention from ~22 s to 3.9-5.3 s. What remains is inside
Turso: a single global commit lock that serializes ~35% of a parallel load, and
an in-memory MVCC index whose comparison cost is ~16% of load CPU. Those two are
the entire remaining gap.

## 2. Head-to-head: MVCC vs WAL

Same host, same binary, quegee, three largest graphs:

| dataset | edges | MVCC e/s | WAL e/s | WAL load | MVCC A4 | WAL A4 |
|---|---|---|---|---|---|---|
| com-Orkut | 117.2 M | 16,046 | 24,666 | 1.54x | **3,200/3,200** in 1.82 s | 61/3,200 |
| soc-LiveJournal1 | 69.0 M | 19,749 | 31,256 | 1.58x | **3,200/3,200** in 4.81 s | 119/3,200 |
| sx-stackoverflow | 63.5 M | 18,504 | 31,243 | 1.69x | **3,200/3,200** in 6.54 s | 50/3,200 |

Full seven-graph lanes (grust, 2xlarge, MVCC with 4 parallel writers; quegee WAL):

| dataset | MVCC e/s | MVCC RSS | WAL e/s | WAL RSS |
|---|---|---|---|---|
| web-Google | 30,985 | 7.1 GB | 46,905 | 3.3 GB |
| cit-Patents | 26,661 | 10.7 GB | 38,076 | 6.8 GB |
| soc-Pokec | 26,385 | 9.9 GB | 38,286 | 4.4 GB |
| GAP-road | 27,708 | 23.8 GB | 38,656 | 10.8 GB |
| sx-stackoverflow | 13,464 | 13.2 GB | 31,243 | 9.9 GB |

Every cell passed with `hard_gate_total = 0` and `duplicate_durable_mutation = 0`.
A3 is unsupported on both; A7 passes on both (it is not run on Neo4j: the
harness's Neo4j path has no `GraphCommitStore`, a gap in the harness, not in
Neo4j).

**Read this as: WAL is a faster loader that cannot take concurrent writes; MVCC
is a slower loader that can.** For a load-then-serve workload with concurrent
writers -- ours -- MVCC is the only correct choice, and the work below is about
making its load cost bearable.

## 3. What we changed in Grust, and what it bought

| change | effect |
|---|---|
| Group commit (`with_group_commit`) | On **0.7.2**: A4 ~22 s -> 3.9-5.3 s, all 3,200 accepted, still durable (one fsync per batch, not per write). On **`main`: turn it OFF** -- see below. |
| Parallel MVCC loads (`set_mvcc_load_parallelism`) | 1.9-9.5x on the same host: GAP-road 2,928 -> 27,708 e/s, cit-Patents 5,526 -> 26,661, sx-stackoverflow 2,929 -> 13,464 |
| Round checkpoints (`MVCC_PARALLEL_ROUND_GROUPS=5`) | peak load memory 15 GB -> 3-7 GB with no loss of speed |
| `connect_shared()` | handles onto one `Database` per file; needed because Turso's `DATABASE_MANAGER` already shares one per (device, inode) |
| `set_synchronous()` / `TursoSynchronous` | exposes the fsync policy; worth ~3% on loads (they commit rarely), decisive for single-statement writes |

Durability note: group commit batches *concurrent* single-statement writes into
one transaction. Every acknowledged write is durable when it is acknowledged.
It is not a relaxation of `synchronous`.

### Our group commit is counterproductive on Turso `main`

Measured 2026-09-16 on grust: same host, same Grust head (`e5d7b4e`), same
workload (16 writers x 200 writes to one hot node, `synchronous=FULL`, shared
handles), three alternating pairs per cell, engine version asserted in the probe
before any timing. All twelve runs `accepted=3200 conflicts=0 consistent=true`:

| Turso | our group commit ON | our group commit OFF |
|---|---|---|
| **0.7.2** (crates.io, our pin) | 3.19 / 3.27 / 3.16 s (~1000 w/s) | 21.10 / 21.21 / 20.85 s (~152 w/s) |
| **`main`** (0.8.0-pre.11) | 3.24 / 3.27 / 3.19 s (~990 w/s) | **2.63 / 2.64 / 2.61 s (~1219 w/s)** |

Three conclusions, none of them inside the ~2% drift floor:

1. On **0.7.2 our group commit is worth 6.6x** (21.1 s -> 3.2 s). It was the
   right fix for the version we pin.
2. On **`main` it costs 19%** (2.62 s -> 3.23 s). `main` enables engine-level
   group commit by default; our client-side batching adds a second coordination
   layer that fights it.
3. **Our ON numbers are identical across both versions (~3.2 s).** Our layer
   sets the ceiling, so it completely masks the engine's improvement. `main`
   with our layer off (2.62 s) beats the best 0.7.2 configuration (3.16 s)
   by 1.21x.

**Action:** disable our group commit on `main` / 0.8.x; keep it on 0.7.2. The
harness now takes `AG_TURSO_GROUP=on|off` (default `on`) and tags such rows
`turso_group=off` so they cannot silently supersede rows that used it.

**Publishing constraint:** crates.io `max_stable_version` for `turso` is still
**0.7.2** -- all eleven 0.8.0 releases are pre-releases. A published Grust must
not force a pre-release on downstreams, so 0.7.2 stays our floor, the group
committer stays in the code, and its default should flip when 0.8.0 goes stable.

(The 22.75 s / 3.40 s figures quoted elsewhere in this document are from eigen
and must not be compared against these grust numbers.)

### Load throughput: `main` is slower than 0.7.2, and the gap grows with writers

Measured 2026-09-16 on eigen (quiet, 8 vCPU), same Grust head, engine version
asserted per run, web-Google **1 M-edge prefix** (a `--edges` default I did not
notice at first; full-graph runs are in §8), MVCC, two alternating rounds:

| writers | Turso `main` edges/s | Turso 0.7.2 edges/s | `main` vs 0.7.2 |
|---|---|---|---|
| 1 | 20,308 / 20,219 | 21,302 / 21,269 | **-5%** |
| 4 | 35,732 / 35,261 | 41,931 / 41,332 | **-15%** |

Node loads show the same shape: -10% at 1 writer (30.1k vs 33.8k nodes/s),
-19% at 4 (42.5k vs 52.2k). Scaling 1 -> 4 writers is 1.96x on 0.7.2 but only
1.76x on `main`.

Read: a cost that *grows with the number of concurrent committers* is the
signature of commit coordination, and `main`'s new engine group commit is
exactly that -- every committer now takes a ticket, checks `durable_through`,
and may park behind a lead. A bulk loader with a few large concurrent
transactions gets no benefit from batching and pays the coordination. The ~5%
at one writer is a smaller, contention-independent cost that this experiment
does not locate.

**Source review (Fable 5.1, 2026-09-16, Turso `19710d58d`) says the
group-commit path is NOT a credible cause of 17%:** uncontended it is ~6 mutex
operations per commit (`core/mvcc/database/mod.rs:1974-2030`), and our loader
commits ~170 times per million edges -- at 1 ms each that is 0.6%. The
concurrency-scaling shape has a different explanation, ranked:

- **H1 -- mid-round automatic checkpoints.** Any commit that pushes the
  logical log past `mvcc_checkpoint_threshold` (4,120,000 B, unchanged since
  0.7.2) tries a TRUNCATE checkpoint inline
  (`persistent_storage/mod.rs:255-261`, `mod.rs:3679-3700`). It needs the
  checkpoint write lock, which every open transaction holds in read mode. On
  0.7.2 staggered writers make that return `Busy` and skip; on `main` group
  commit releases followers together (`mark_durable`), opening windows where
  all writers are between transactions and one wins -- a stop-the-world
  write-out inside a writer's `COMMIT`, while the others' `BEGIN CONCURRENT`
  gets `Busy` and Grust retries 8 times with no backoff (`lib.rs:968-970`,
  classified by substring at `lib.rs:1518-1521`). A passing load can hide up
  to 7 retries per transaction. Decisive test: `mvcc_checkpoint_threshold = -1`
  around the parallel rounds (our explicit round checkpoint already bounds
  memory) plus a retry counter.
- **H2 -- heavier per-row path on `main`.** Every seek now takes
  `chain_falls_through_for_tx` / `btree_covers_chain_for_tx`
  (`mod.rs:5973-5983`) and `skipmap_row_while_uncovered` (from `07b6f3de2`);
  consistent with nodes regressing too. `SortableIndexKey::compare` is
  byte-identical between versions; PR #8385 is not in `main`. Only `perf`
  separates H2 from H1.
- **H3 -- allocator contention.** `grust-turso` already has a `mimalloc`
  feature (`Cargo.toml:16`) and it was **off in every measurement in this
  document**. Estimated 5-15% at 4-8 writers; applies to both versions.

**A methodology defect the review found, which biases every WAL-vs-MVCC ratio
above:** parallel MVCC writers never run `PRAGMA foreign_keys = ON` -- only the
bootstrap connection does (`lib.rs:1293`), and `connect_shared` handles
inherit nothing (`lib.rs:227-263`). So parallel MVCC loads skip two
`REFERENCES nodes(id)` probes per edge that WAL and single-writer MVCC loads
pay, and silently relax a schema constraint during loads. Whether to enforce
FKs on load writers is a correctness decision, not a tuning one; either way
the comparison must be made with both sides on the same setting.

Two more items the review flagged as unproven in this document: the "35%
serialized = `pager_commit_lock`" reading is inferred (0.7.2's per-insert rowid
write lock, `4c26a4b6b`, and spinning waiters are in that fraction too -- refit
on `main`), and `set_bulk_load_via_wal` has not been verified on `main`
(journal-mode switching was touched by `aaea59de6`).

**E2 result (2026-09-16 20:48Z, eigen at 0% steal, full 5,105,039-edge
web-Google, 4 writers, two alternating rounds): engine group commit is NOT the
cause.** `main` with `PRAGMA mvcc_group_commit = off` 31,306 / 31,827 e/s;
`on` 31,805 / 31,614 e/s -- identical within drift. 0.7.2 36,600 / 37,015 e/s,
still ~16% faster; nodes 39.4-40.0k vs 47.3-47.9k nodes/s. Exactly what the
source review predicted. The remaining suspects are H1 (mid-round automatic
checkpoints), H2 (heavier per-row seek path) and H3 (glibc malloc); E1 and E3
are running.

**The regression is MVCC-specific; `main` is FASTER on WAL.** Lakecat matrix,
round 1 (t2.xlarge, alternating engines per cell, full 5,105,039-edge
web-Google, 2026-09-16):

| mode | writers | `main` e/s (r1 / r2) | 0.7.2 e/s (r1 / r2) | `main` vs 0.7.2 |
|---|---|---|---|---|
| MVCC | 1 | 14,377 / 14,541 | 15,152 / 15,334 | **-5%** |
| MVCC | 4 | 24,403 / 24,899 | 29,310 / 29,773 | **-16%** |
| **WAL** | 1 | **50,603 / 50,856** | 41,700 / 42,071 | **+21%** (nodes 125k vs 93k/s) |
| **WAL** | 4 (= 1 writer) | **51,159 / 50,617** | 42,038 / 41,662 | **+21%** |

Two rounds, engines alternated within every cell, no overlap between engines
in any cell. (Lakecat is a t2.xlarge and was stealing ~4-6% during this run;
the alternation makes the *ratios* trustworthy, the absolute rates not.)

Same MVCC shape as the eigen sweep, and a clean reversal on WAL: `main`'s
B-tree write path improved (`9ef17336d` rightmost-leaf append, `df1678018`
recycled record buffers) while its MVCC per-row path got heavier. That points
at H2 over H3 (an allocator problem would show on WAL too). It also means that
on `main`, loading an MVCC store *through* WAL (`set_bulk_load_via_wal`: load in
WAL, checkpoint, switch to MVCC) is potentially 2x parallel MVCC. The
journal-mode round trip IS verified on `main`: in the 79-test `grust-turso` run
against 0.8.0-pre.11 (provenance asserted), `a_live_mvcc_store_switched_to_wal_and_back`
and `a_wal_loaded_database_reopened_as_mvcc` passed and the `wal_bulk_load.rs`
suite ran clean. Timing it on `main` is queued.

**E1 result (2026-09-16 ~21:15Z, eigen, 0 s steal in every run, full graph,
4 writers, two alternating rounds): H1 is dead.** `PRAGMA
mvcc_checkpoint_threshold = -1` for the parallel rounds changes nothing --
`main` 31,475 / 31,208 e/s default vs 31,510 / 31,345 off; 0.7.2 36,050 /
36,436 vs 35,230 / 36,219 -- and the new retry counter in `load_transaction`
read **0 Busy/conflict retries in all eight runs**. The mid-round-checkpoint
and Busy-storm mechanism never fires on this workload. With E2 also null, the
`main` MVCC load regression is not in commit coordination or checkpointing;
it is in the per-row MVCC path (H2), which Grust cannot reach. E3 (allocator)
is the last Grust-side lever being tested; lakecat's WAL reversal already says
it is not the differential cause. Next: `perf` on `main` vs 0.7.2 (E8) to
attribute H2 by symbol -- that attribution is the upstream bug report.

**E3 result (2026-09-16 21:40Z, eigen, 0 s steal in all eight runs, full
graph, 4 writers, two alternating rounds): mimalloc is a free ~15% on BOTH
engines, and not the reason `main` is slower.**

| allocator | `main` e/s (r1 / r2) | 0.7.2 e/s (r1 / r2) |
|---|---|---|
| glibc (every measurement before this) | 31,206 / 31,306 | 36,172 / 35,245 |
| **mimalloc** (`grust-turso` feature, off until now) | **35,673 / 35,347** (+14%) | **41,729 / 41,490** (+16%) |

`grust-turso` has declared `mimalloc = ["turso/mimalloc"]` all along; no run in
this document used it. The gain is the same on both versions, so the `main`
MVCC regression survives it intact (H2). Recommendation: leave the feature
opt-in in the library (a `#[global_allocator]` is an application decision)
and turn it on in the harness and in any deployment that owns its binary.

**E8 result (2026-09-16 21:32Z, lakecat, `perf record -F 499`, MVCC 4
writers, full graph; ~6-8% steal during both, alternated): the regression is
in the value comparison inside the MVCC index-key compare.** Self time:

| symbol | `main` | 0.7.2 |
|---|---|---|
| `mvcc::database::SortableIndexKey::compare` | 20.8% | 19.6% |
| `types::cmp_in_column` | **6.2%** | -- (`compare_immutable_single` 2.7% + types 1.5%) |
| `sqlite3_ondisk::read_value_serial_type` | **4.3%** | 1.7% |
| B-tree index seeks (`indexbtree_move_to_internal` + `seek`) | 3.5% | 5.8% |
| `malloc` + `cfree` | 2.4% | 3.4% |

The index-key comparison path is ~31% of the load on `main` vs ~25.5% on
0.7.2: `compare` is byte-identical between versions (verified), but its
callee changed from `compare_immutable_single` to `cmp_in_column`, which
decodes serial types about 2.5x as often per comparison. Meanwhile the B-tree
side got cheaper -- that is the WAL +21%. This is exactly the hot spot
upstream PR #8385 ("raw-bytes index key comparison": walk serialized records
directly, no per-column `ValueRef` materialization, no simdutf8 for
byte-collated text) is written to remove. An A/B of `main` + #8385 vs `main`
vs 0.7.2 is running on lakecat; if it recovers the gap, the upstream message
is "your draft fixes it", with numbers.

### The fastest fill on `main`: load through WAL (94k edges/s, 2.65x parallel MVCC)

Grust `c8ec310` (foreign keys off on every load path -- see §7b), mimalloc on,
eigen at 0 s steal in every run, full 5,105,039-edge web-Google, two
alternating rounds, 2026-09-16 22:07Z:

| load path | `main` e/s | 0.7.2 e/s | `main` vs 0.7.2 |
|---|---|---|---|
| WAL store, WAL load | **93,064 / 93,812** | 76,820 / 76,503 | +22% |
| **MVCC store filled via WAL** (`set_bulk_load_via_wal`) | **94,080 / 94,172** | 76,818 / 76,651 | +22% |
| MVCC store, 4 parallel writers | 35,539 / 35,479 | 41,548 / 41,854 | -15% |

Three conclusions:

1. **The via-WAL fill costs nothing over plain WAL** -- the journal-mode round
   trip (WAL -> load -> checkpoint -> MVCC) is free at this size, and it is
   test-verified on both engines. An MVCC store on `main` can be filled at
   94k edges/s and then served with `main`'s engine group commit.
2. **WAL roughly doubled** against every earlier number in this document
   (43-51k e/s), and the split is measured (eigen, 0 s steal, two alternating
   rounds, 22:18Z): with FK off and **glibc**, WAL is 80,687 / 81,967 e/s on
   `main` and 70,313 / 69,651 on 0.7.2. So **FK-off alone is ~+60%** (0.7.2:
   42.9k -> 70k; `main`: ~50.6k -> 81k) -- three to six times the source
   review's 10-20% estimate; the two `REFERENCES nodes(id)` probes per edge
   were costing WAL over a third of its throughput -- and **mimalloc adds
   +9% (0.7.2) / +16% (`main`)** on top.
   **Retroactive caveat on §2:** every WAL row there enforced FKs and every
   parallel-MVCC row did not, so "WAL loads 1.5-1.7x faster" *understates*
   WAL; with both sides FK-off the load gap is wider. The concurrent-write
   half of that comparison (WAL accepting 20-120 of 3,200) is unaffected.
3. Parallel MVCC is unchanged by the FK commit (its writers never enforced
   FKs), so the -15% `main` MVCC regression stands and is now the *only*
   configuration where `main` trails 0.7.2.

**At 3x the size it holds, and the MVCC regression grows.** cit-Patents,
16,518,948 edges, eigen, 0-3 s steal per run, `c8ec310`, mimalloc, two
alternating rounds (2026-09-16 ~23:30Z):

| load path | `main` e/s | 0.7.2 e/s | `main` vs 0.7.2 |
|---|---|---|---|
| **MVCC store filled via WAL** | **79,732 / 79,264** | 67,254 / 66,068 | +19% |
| MVCC store, 4 parallel writers | 25,729 / 25,631 | 34,953 / 34,974 | **-26%** |

Via-WAL is **3.1x** parallel MVCC on `main` here (2.65x on web-Google). And
the 4-writer MVCC gap widens from -15% at 5.1 M edges to **-26% at 16.5 M**
-- consistent with a per-comparison cost in an index whose depth grows with
row count, i.e. exactly the `cmp_in_column` / `read_value_serial_type` path
that #8385 removes. Expect it to be worse still at com-Orkut scale.

**At 63.5 M edges on the clean host (quegee, c5n, 0.2 s steal), `main`
MVCC 4 writers, `sync=normal`, glibc, `e5d7b4e` loader, 2026-09-17
00:25Z:** sx-stackoverflow **9,912 e/s**, load 6,406 s, 13.6 GB, all gates
pass. Not a matched pair yet -- quegee's 0.7.2 cell for this graph ran 8
writers (18,504 e/s), and the nearest 4-writer 0.7.2 cell is grust's 13,464
(t2 host, FULL sync), a cross-host -26% that agrees with cit-Patents but
cannot be cited as clean. A 0.7.2 run at 4 writers on quegee follows the
ladder to close it. What this cell does establish, same host and config as
the 0.7.2 NORMAL row: **`main`'s read path is faster** -- A2 688 s vs 767 s,
A1 1.9 s vs 2.6 s. (A4 at NORMAL -- 2.87 s here vs 6.54 s on that row -- is
not evidence either way; see the A4 correction below.) The MVCC *load* path
regressed; reads did not.

Same pattern on the next graph (quegee, 02:39Z, 0.3 s steal):
**soc-LiveJournal1 `main` 4 writers 9,973 e/s**, load 6,918 s, 15.7 GB, all
gates pass; vs the 0.7.2 8-writer row (19,749 e/s) again unmatched on writer
count, but on reads at identical config **A1 24.5 s vs 29.5 s, A2 978 s vs
1,065 s** in `main`'s favor; A4 5.15 s at NORMAL (against 4.81 s on that
row and **1.43 s** on the matched 0.7.2 run below -- A4 at NORMAL is noise,
see the correction below). And the largest (quegee, 06:47Z, 0.5 s steal):
**com-Orkut `main` 4 writers 8,707 e/s**, load 13,458 s (3 h 44 m), 17.4 GB,
all gates pass; reads **A1 120.6 s vs 138.1 s, A2 1,197 s vs 1,312 s** in
`main`'s favor (A4 1.88 s vs 1.82 s at NORMAL -- not evidence either way,
see below). Three of three at-scale cells say the same thing: **load
slower, reads faster.** The matched 0.7.2 pair at
4 writers (crates.io 0.7.2, Grust `f6d3391` -- the same pin as the original
0.7.2 rows -- NORMAL, glibc) started 06:48Z on the same host to make the
load comparison same-config. **First matched row (08:27Z, 0.2 s steal on both
sides):**

| sx-stackoverflow, 4 writers, NORMAL, glibc, quegee | `main` | 0.7.2 | `main` vs 0.7.2 |
|---|---|---|---|
| load | 9,912 e/s (6,406 s) | 12,344 e/s (5,144 s) | **-20%** |
| peak RSS | 13.6 GB | 11.3 GB | +20% |
| A1 | 1.9 s | 2.5 s | 23% faster |
| A2 (depth 7, 2.26 M reached) | 688 s | 719 s | 4% faster |
| A4 | 2.87 s, 3,200/3,200 | 3.20 s, 3,200/3,200 | 10% faster |

| soc-LiveJournal1, same config (10:17Z) | `main` | 0.7.2 | `main` vs 0.7.2 |
|---|---|---|---|
| load | 9,973 e/s (6,918 s) | 12,660 e/s (5,450 s) | **-21%** |
| peak RSS | 15.7 GB | 13.5 GB | +16% |
| A1 | 24.5 s | 26.8 s | 9% faster |
| A2 (depth 14, 4.4 M reached) | 978 s | 1,023 s | 4% faster |
| A4 | 5.15 s, 3,200/3,200 | 1.43 s, 3,200/3,200 | **3.6x slower** |

Same-host, the at-scale load regression is **-20 to -21%** (web-Google -15%,
cit-Patents -26%); **reads (A1, A2) are faster on `main` on every graph**;
RSS is 16-21% higher on `main` at this size (none at 5.1 M edges).

**Correction on A4 -- I over-read it.** Earlier text here called `main`'s
soc-LiveJournal1 A4 (5.15 s) "within drift" of the 0.7.2 8-writer row
(4.81 s) and said every scenario was faster on `main`. The matched 0.7.2 run
does A4 in **1.43 s**. Across every `synchronous=NORMAL` cell in this
document A4 spans 1.4-5.2 s on *both* engines with no consistent direction
(sx-stackoverflow: `main` 10% faster; soc-LiveJournal1: 3.6x slower; the
old 0.7.2 8-writer rows: 1.8-6.5 s). At NORMAL there is no fsync to
amortize, so A4 measures conflict-retry luck under contention, not the
commit path. **The only A4 comparison to cite is the FULL-sync alternating
A/B in §3: `main` 2.62 s vs 0.7.2's best 3.16 s.** "Serve faster" means
reads.

| com-Orkut, same config (10:17Z) | `main` | 0.7.2 | `main` vs 0.7.2 |
|---|---|---|---|
| load | 8,707 e/s (13,458 s) | 11,240 e/s (10,426 s) | **-23%** |
| peak RSS | 17.4 GB | 13.3 GB | +31% |
| A1 | 120.6 s | 159.4 s | 24% faster |
| A2 (depth 7, 3.07 M reached) | 1,197 s | 1,478 s | 19% faster |
| A4 (NORMAL -- not cited) | 1.88 s | 1.72 s | -- |

All three at-scale rows are now same-host, same-config, zero-steal.

**Production recommendation on `main`, as it stands:** fill MVCC stores through
WAL; serve with engine group commit on and Grust's client committer off;
build with `mimalloc`. Every one of those is a switch that already exists.

**Profile under PR #8385 (lakecat, 22:36Z, same `perf` flags as E8):** the
index-key path that was ~31% on `main` (`SortableIndexKey::compare` 20.8% +
`types::cmp_in_column` 6.2% + `read_value_serial_type` 4.3%) becomes one
symbol, **`types::compare_serialized_records` at 12.2%**, and the three old
symbols leave the top list. B-tree seeks 6.7% (0.7.2: 5.8%); `from_utf8`
1.3% remains (the PR skips validation only for byte-based collations). A
~19-point drop in exactly the path E8 blamed, matching the +29-30% below.

**PR #8385 measured (2026-09-16 22:14Z, lakecat, MVCC 4 writers, full
graph, mimalloc, three arms alternated, 6-10% steal so ratios only):**

| arm | e/s (r1 / r2) |
|---|---|
| PR #8385 branch (`62b0e660e`) | 29,981 / 30,902 |
| `main` (`19710d58d`, pre.11) | 27,525 / 27,991 |
| 0.7.2 | 32,533 / 33,111 |

**Caveat that governs the reading:** the PR branch resolves as `turso_core
v0.8.0-pre.4` -- its base `bad083faf` is seven pre-releases behind our `main`.
So "+9% over `main`" is the PR *plus* whatever changed between pre.4 and
pre.11, in unknown proportion. The fourth arm at the PR's own base settles it
(same host, alternating, two rounds):

| arm | e/s (r1 / r2) |
|---|---|
| PR base `bad083faf` (pre.4) | 23,857 / 24,201 |
| **PR #8385 on that base** | **30,978 / 31,376** |

**PR #8385 alone is +29-30%** on our 4-writer MVCC bulk load. The other
arms put pre.4 -> pre.11 at +15% of unrelated drift, and 0.7.2 at 32.8k on
this host. If the PR's gain composes on pre.11 as it does on pre.4,
`main`+#8385 lands near 36k -- ~10% *ahead* of 0.7.2 on the one load where
`main` trails today. That composition could not be measured here: the five
PR commits cherry-picked onto `19710d58d` conflict in `core/mvcc/cursor.rs`
and `core/mvcc/database/mod.rs`, and resolving conflicts inside Turso's MVCC
internals is the PR author's rebase, not ours. It is an inference until they
rebase; we offered to measure the rebased branch.

**The lever (now demoted -- it does not move loads):** `main` exposes `PRAGMA mvcc_group_commit = on|off` (store-wide;
added with `core/mvcc: enable group commit by default`, 2026-09-11). If turning
it off for the duration of a `put_graph` recovers 0.7.2's load speed, the
production configuration on `main` becomes: engine group commit **off during
bulk loads, on for serving**, our client-side group committer removed -- and
`main` is then strictly better than 0.7.2 on both axes. That A/B (full 5.1 M
edges, 4 writers, on/off/0.7.2 alternating) is running; `set_mvcc_group_commit()`
mirrors `set_synchronous()` in the probe tree.

## 4. Where the time actually goes

`perf` on lakecat, MVCC web-Google load:

- **16.2%** `SortableIndexKey::compare` -- the MVCC in-memory index skiplist,
  comparing four text columns per probe
- ~8% other MVCC bookkeeping
- ~8% B-tree seeks
- ~3% checkpoint

WAL, same load: **22%** B-tree seeks, and none of the MVCC overhead.

Amdahl fit over 1/2/4/8 writers on eigen (326 / 220 / 171.5 / 142.3 s) gives
**~35% serialized**. That serialized fraction is Turso's global
`pager_commit_lock`. It is why parallel MVCC loads asymptotically approach, but
cannot pass, WAL's single-writer speed: at 8 writers we reach ~81% of WAL.

## 5. What would make Turso faster for us (upstream asks, in priority order)

> **Read this section against the version it was written for.** Everything above
> is Turso **0.7.2**, the crates.io release we pin. Upstream `main` is
> **0.8.0-pre.11, 1,859 commits ahead**, and it has already closed or is closing
> several of these. Verified 2026-09-16 against a local clone:
>
> - **Ask 3 (engine group commit) is DONE upstream.** `main` has
>   `step_await_group_commit` with lead/prefix group work, tickets and parked
>   waiters; `core/mvcc: enable group commit by default` means it is on without
>   opt-in. It does not exist in `v0.7.2`. Our Grust-side group commit was the
>   right fix for 0.7.2 and may be redundant -- or may double-batch -- on `main`.
> - **Ask 2 (cheaper index keys) is IN FLIGHT.** PR #8385 "core/mvcc: optimize
>   cursor record caching and index key comparison" (open, draft, stacked on
>   #8381) walks serialized records directly and skips simdutf8 validation for
>   byte-based collations. Their own measurement: UTF-8 validation ~14% and
>   comparison machinery ~35% of an index-seek workload -- the same hot spot our
>   profile found at 16.2%. Do not duplicate it.
> - **Asks 1, 4, 5, 6 have not been checked against `main` yet.**
>
> The lesson, recorded because it cost a week: **check upstream `main` before
> building a client-side workaround for an engine limitation.** Pinning a
> crates.io release while the project moves 1,859 commits makes solved problems
> look unsolved.
>
> Turso's `CONTRIBUTING.md` also asks that contributors to MVCC/b-tree submit
> bug reports rather than AI-generated fixes unless they know the layer well.
> Our contribution here is evidence -- reproducers and benchmark data -- not
> patches to their commit path.

1. **Break up the global commit lock.** One `pager_commit_lock` serializes every
   commit across every connection. This is the single biggest lever: ~35% of a
   parallel load is spent behind it, and it caps parallel loading near WAL speed
   no matter how many writers we add. Per-page or range-scoped commit locking,
   or a commit pipeline that lets non-overlapping transactions finalize
   concurrently, would directly convert into load throughput.
2. **Cheaper MVCC index keys.** `SortableIndexKey::compare` over four text
   columns is 16.2% of load CPU. Native support for integer or pre-encoded
   composite keys (a single comparable byte string built once per row, rather
   than a four-column comparison per skiplist probe) would cut most of that. We
   measured dropping one key column (`identity_key`) as worth 2-4%, which
   indicates the cost is the *number* of comparisons, not key width -- so the
   fix is a cheaper comparison, not a narrower key.
3. **Group commit inside Turso.** We built it in Grust because Turso has none:
   concurrent single-statement writers each pay their own fsync under the global
   lock, which is why A4 took ~22 s before. Engine-level group commit would give
   this to every Turso user, and would compose better with the commit path than
   our client-side batching can.
4. **Checkpoints that coexist with concurrent writers.** Under
   `BEGIN CONCURRENT`, automatic checkpoints return `Busy`, so free-running
   writers skip them and the logical log grows until memory does (15 GB on a
   web-Google load). We work around it with explicit round checkpoints. Note
   `experimental_mvcc_passive_checkpoint` is *not* the answer: enabling it was a
   regression (web-Google 5.1k e/s at 11.5 GB, vs 16.4k at 1.7 GB with it off).
5. **A bulk-ingest path for MVCC.** WAL gets a fast sequential append; MVCC pays
   full per-row machinery even when loading into empty tables. An explicit
   "load" mode -- build the index once at the end, skip per-row MVCC
   bookkeeping -- is the shape that would close the 1.5-1.7x gap. We tried the
   Grust-side approximations and none worked (§6); this one has to be in the
   engine.
6. **Make WAL's writer rejection visible.** WAL accepts 2-4% of concurrent hot-node
   writes and rejects the rest as busy. That is defensible behavior, but it means
   any WAL benchmark that reports only throughput is measuring the wrong thing.

## 6. Ideas we measured and rejected (do not re-run these)

| idea | result |
|---|---|
| Deferred secondary indexes, `CREATE INDEX` after load | **slower**: 437.9 s vs 349.6 s |
| Plain `INSERT` for loads into empty tables (skip `ON CONFLICT`) | **~4% slower** on WAL, a wash on MVCC; a plain INSERT pays the same B-tree insert |
| Writer page cache 128 MiB + `mvcc_gc_threshold=-1` | flat (inside ~2% host drift) |
| Round/batch size tuning | buys memory, not time |
| Dropping `identity_key` from the edge key | real but small: 2.0% MVCC, 3.9% WAL -- not worth making `TursoDialect` stateful across ~10 call sites |
| Integer-interned keys | not started; the `identity_key` result says comparison count, not key width, is the cost |
| `experimental_mvcc_passive_checkpoint` | **regression**, reverted |
| Loading MVCC via the WAL path (`set_bulk_load_via_wal`) | works, and is exactly WAL speed -- but it is WAL, with WAL's concurrency behavior |

## 7. Methodology notes, so these numbers are read correctly

- `/tmp` is tmpfs on all four hosts. Early timings that used it skipped fsync
  entirely and were discarded; everything here is on real disk.
- Run-to-run drift on lakecat is ~2% (the same WAL load gave 126.6 / 128.5 /
  134.3 s in one night). Any claimed effect under ~4% needs alternating A/B
  pairs, not a single comparison.
- Compare against the deployed pin, not a branch head. We once reported a 2.35x
  "win" that was really recovery from a regression already present in the
  baseline we chose.
- After patching a probe binary, verify its mtime before trusting a timing. One
  `identity_key` probe silently timed a stale binary and nearly reported a fake
  3% gain.
- Cross-host comparisons are not evidence. quegee has 16 cores; grust and eigen
  have 8 vCPU. Only same-host pairs appear in §2.
- The harness client guard (`AG_RSS_LIMIT_GB`) kills the *client*, not the
  server. Two earlier Neo4j attempts died at `rss 26 GB > 24 GB` while building
  the graph in memory, leaving populated stores and no report bundle.

## 7a. CPU steal: which hosts can be trusted for what

grust and eigen are **t2.2xlarge** and lakecat is **t2.xlarge** -- burstable.
quegee is a **c5n.4xlarge** (9 s of steal over its whole uptime). Discovered
2026-09-16 20:46Z from the harness's own `host_steal_us`:

| grust lane | web-Google | cit-Patents | soc-Pokec |
|---|---|---|---|
| 0.7.2 MVCC (earlier today) | 0.1% steal | 0.1% | 0.0% |
| `main` MVCC (after it) | **8.7%** | **24.4%** | **29.4%** (641 s stolen of 2,181 s) |

The 0.7.2 lane spent the credit bucket; the `main` lane inherited it empty. The
"`main` is 1.7x slower on grust" that this produced was AWS, not Turso. That
ladder was stopped at GAP-road and its cells are **void**
(`logs-h2h-main-mvcc-groupoff-STEAL-VOID.log`); the `main` ladder must be
rerun on quegee.

Consequences for the rest of this document:
- **Trust:** everything on quegee (all Neo4j cells, the MVCC/WAL lanes, the
  same-host pairs in §2). The eigen writer sweep and the lakecat matrix
  alternate engines within one run, so both engines saw the same steal and
  their *relative* results stand; their absolute rates do not.
- **Caveat:** the hot-node A/B in §3 ran `main` and 0.7.2 as sequential runs
  on grust (18:56-19:10Z). The within-version ON/OFF pairs alternated and are
  robust. The cross-version delta ("`main` off 2.62 s beats 0.7.2 on 3.16 s")
  is exposed to steal -- conservatively, since `main` ran later and would have
  been the throttled one, but it should be re-measured on quegee.
- **Rule:** record steal around every timing (`/proc/stat` field 9, or
  `host_steal_us`); reject cells above ~2%; never compare configurations as
  sequential lanes on a t2 host; give absolute-throughput work to quegee.
- **Memory on `main`:** the quegee `main` ladder at 8 writers reached
  **34.5 GB RSS on GAP-road** (4 GB available, swap still 0) and was stopped
  at 22:25Z before the guard or swap could act; that GAP-road cell is void.
  Whether `main` needs more memory than 0.7.2 *per writer* was not
  established by the harness cells (quegee `main` ran 8 writers, grust 0.7.2
  ran 4). A matched pair settles it -- eigen, web-Google, 4 writers, mimalloc,
  FK off, peak `VmHWM` sampled from `/proc/<pid>/status` each second
  (`/usr/bin/time` is not installed there), alternating, 2026-09-16 23:38Z:
  **`main` 3.9 / 4.4 GB, 0.7.2 4.0 / 3.4 GB.** No per-writer difference
  (the spread within each engine is as large as the spread between them). The 34.5 GB on GAP-road was 8 writers x a 57.7 M-edge,
  23.9 M-node graph, not the engine; the lesson is writer count, not version. The three largest
  graphs were relaunched at 4 writers (guard 36 GB, avail floor 3 GB), and a
  matched 0.7.2 run at 4 writers follows so the pair is same-config.

## 7b. Foreign keys: loads now skip them on every path (Grust `3f1ed19`)

The Turso schema declares `from_id`/`to_id` as `REFERENCES nodes(id) ON DELETE
CASCADE` (`grust-sql-core/src/lib.rs:126-127`), and `bootstrap` sets
`PRAGMA foreign_keys = ON` on the bootstrap connection only. Parallel MVCC
writers come from `connect_shared`, which never set the pragma. So until
2026-09-16 **whether a dangling edge was accepted depended on the writer
count**: WAL and single-writer MVCC loads rejected it, parallel MVCC loads
accepted it -- same store, same `put_graph`. That also biased every WAL-vs-MVCC
load ratio in §2, since only the parallel path was skipping two index probes
per edge.

Decision (user, 2026-09-16): **foreign keys off on every load path.** Grounds:
the Memory reference accepts dangling edges (a vertex "exists only as an edge
endpoint", `grust-memory/src/lib.rs:94`); `grust-core` rejects them only under
explicit schema validation; `ON DELETE CASCADE` fires on the connection that
deletes, which keeps the pragma on, so serving semantics are unchanged; and the
harness pre-drops dangling edges (`dangling_edges_dropped: 0` everywhere), so
no benchmark outcome depended on enforcement.

Implementation: `put_graph_rows` reads `PRAGMA foreign_keys`, sets it off
*outside any transaction* (SQLite ignores the pragma inside one), runs the
load, and restores it whether or not the load failed.
`tests/dangling_edges.rs` loads a dangling edge through WAL, one MVCC writer
and four MVCC writers, reads it back identically on each, then checks a single
`put_edge` to a missing node is still refused. 80/80 on Turso 0.7.2 and on
`main`. Effect on throughput: §3's "fastest fill" table.

## 8. Open questions

- Does parallel MVCC scale past 8 writers on a 16-core host, or does the commit
  lock flatten it earlier? Untested at 12-16 writers.
- GAP-road MVCC peaked at 23.8 GB against a 26 GB guard on a 31 GiB box. That
  cell passes but has no headroom; it needs either a bigger host or a lower
  round-group setting before it can be called reproducible.
- **Neo4j at the top of the ladder -- answered 2026-09-16** (quegee, c5n,
  <1 s steal, `AG_RSS_LIMIT_GB=38`; both prior attempts had died at the
  client guard, not in Neo4j). Same host as the Turso 0.7.2 pairs in §2:

  | com-Orkut (117 M) | Neo4j | Turso MVCC | Turso WAL |
  |---|---|---|---|
  | load e/s | 16,800 | 16,046 | **24,666** |
  | RSS | **4.9 GB** | 22.5 GB | 12.4 GB |
  | A1 | 144.5 s | 138.1 s | **96.8 s** |
  | A2 (depth 7, 3.07 M reached) | 4,082 s | **1,312 s** | 943 s |
  | A4 (MVCC at NORMAL -- not comparable) | 2.90 s, 3,200/3,200 | 1.82 s, 3,200/3,200 | 1.05 s, 61/3,200 |
  | A7 | not run (harness path has no `GraphCommitStore`) | pass | pass |

  | soc-LiveJournal1 (69 M) | Neo4j | Turso MVCC | Turso WAL |
  |---|---|---|---|
  | load e/s | 28,062 | 19,749 | **31,256** |
  | RSS | **6.7 GB** | 19.8 GB | 10.3 GB |
  | A1 | 41.8 s | 29.5 s | **19.5 s** |
  | A2 (depth 14, 4.4 M reached) | 2,501 s | 1,065 s | **773 s** |
  | A4 (MVCC at NORMAL -- not comparable) | 2.90 s, 3,200/3,200 | 4.81 s, 3,200/3,200 | 1.29 s, 119/3,200 |

  Read: Turso wins traversal (A1, A2 by 2.3-3.1x) on both. A7 is not
  compared (see above). On com-Orkut MVCC matches Neo4j's load (-4.5%); on
  soc-LiveJournal1 Neo4j loads 1.42x faster. The MVCC rows here are the
  8-writer `synchronous=NORMAL` lane, so their A4 is not comparable with
  Neo4j's durable commits (§3's A4 correction); in the durable same-host
  pairs the strain page counts, Neo4j wins every hot-node-write pair against
  MVCC and loses five of six against WAL, which accepts far fewer writes.
  Neo4j is 3-4.6x leaner in memory throughout. Neo4j's A2 on soc-LiveJournal1 is heavier
  (depth 14) than com-Orkut's (depth 7); the 2.3-3.1x holds on both.
  Neo4j has no cells for web-Google or cit-Patents, and cit-Patents A2 is
  vacuous on every backend (`expected_reached = 0`), so A2 claims rest on
  the five graphs with substance.
