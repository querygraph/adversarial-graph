#!/usr/bin/env python3
"""Freeze run bundles into a site evidence directory.

    scripts/bundle-site-evidence.py OUT_DIR --host "DESCRIPTION" [--reports DIR]
        [--since RUN] [--exclude RUN,RUN,...]

Copies every <reports>/<run>/report.json (and results.jsonl when present)
into OUT_DIR/<run>/, then writes OUT_DIR/manifest.json (schema v2) with the
bundling harness revision, the dataset manifest digest, one entry per run
carrying that run's own harness revision, dirty paths and completeness as
its report states them, and the byte count and SHA-256 of every payload,
sorted by path. The manifest is what the site verifier checks; nothing
under OUT_DIR is edited afterwards.

Runs may come from several harness revisions and some may carry a `-dirty`
stamp (the report says which paths differed); every revision's base commit
must be reachable from origin/main, or the run is refused, because a
reader must be able to fetch the source a row was measured with. Runs that
predate the dirty-path and completeness fields are recorded with nulls.

`--host` is the one-line description of the machine the runs were taken
on, recorded verbatim. `--reports` selects the run directory (default
`reports/`; bundles pulled from another host live elsewhere). `--since RUN`
includes only stamps at or after RUN; `--exclude` drops superseded stamps.
"""
import argparse, hashlib, json, os, shutil, subprocess, sys
parser = argparse.ArgumentParser()
parser.add_argument("out_dir")
parser.add_argument("--host", required=True, help="one-line description of the host the runs were taken on")
parser.add_argument("--reports", default="", help="run directory (default: reports/ in the harness checkout)")
parser.add_argument("--since", default="", help="include only run stamps >= this value")
parser.add_argument("--exclude", default="", help="comma-separated run stamps to leave out (superseded bundles)")
args = parser.parse_args()
root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
reports_dir = os.path.abspath(args.reports) if args.reports else os.path.join(root, "reports")
out = os.path.abspath(args.out_dir)
if os.path.exists(out) and os.listdir(out):
    sys.exit(f"refusing to write into non-empty {out}")
os.makedirs(out, exist_ok=True)
git = lambda *a: subprocess.run(["git", "-C", root, *a], capture_output=True, text=True)
head = git("rev-parse", "HEAD").stdout.strip()
if git("status", "--porcelain", "--untracked-files=no").stdout.strip():
    sys.exit("refusing to bundle from a harness checkout with uncommitted tracked changes")
sha = lambda p: hashlib.sha256(open(p, "rb").read()).hexdigest()
excluded = {r for r in args.exclude.split(",") if r}
payloads, runs, grust_versions, refused = [], [], set(), []
for run in sorted(os.listdir(reports_dir)):
    src = os.path.join(reports_dir, run)
    if not os.path.isfile(os.path.join(src, "report.json")) or run < args.since or run in excluded:
        continue
    report = json.load(open(os.path.join(src, "report.json")))
    revision = report.get("harness_revision", "unknown")
    base = revision.removesuffix("-dirty")
    if len(base) != 40 or git("merge-base", "--is-ancestor", base, "origin/main").returncode != 0:
        refused.append((run, revision))
        continue
    os.makedirs(os.path.join(out, run))
    for name in ["report.json", "results.jsonl"]:
        if os.path.isfile(os.path.join(src, name)):
            shutil.copyfile(os.path.join(src, name), os.path.join(out, run, name))
            p = os.path.join(out, run, name)
            payloads.append({"path": f"{run}/{name}", "bytes": os.path.getsize(p), "sha256": sha(p)})
    grust_versions.add(report.get("grust_version", "0.13.0"))
    runs.append({"run": run, "smoke": bool(report.get("summary", {}).get("smoke")),
                 "results": len(report.get("results", [])),
                 "hard_gate_total": sum(sum(r["gates"].values()) for r in report.get("results", [])),
                 "harness_revision": revision,
                 "harness_dirty_paths": report.get("harness_dirty_paths"),
                 "complete": report.get("summary", {}).get("complete")})
if refused:
    print("refused (revision not reachable from origin/main):", file=sys.stderr)
    for run, revision in refused:
        print(f"  {run} {revision}", file=sys.stderr)
if not runs:
    sys.exit("no runs to bundle")
if len(grust_versions) != 1:
    sys.exit(f"runs mix grust-graph versions: {sorted(grust_versions)}")
manifest = {
    "schema": "adversarial-graph-strain-evidence-v2",
    "track": "strain",
    "harness": "adversarial-graph",
    "harness_revision": head,
    "grust_graph_version": grust_versions.pop(),
    "dataset_manifest_sha256": sha(os.path.join(root, "datasets", "MANIFEST.json")),
    "host": args.host,
    "runs": runs,
    "payloads": sorted(payloads, key=lambda p: p["path"]),
}
json.dump(manifest, open(os.path.join(out, "manifest.json"), "w"), indent=2, sort_keys=True)
open(os.path.join(out, "manifest.json"), "a").write("\n")
dirty = sum(1 for r in runs if r["harness_revision"].endswith("-dirty"))
revisions = sorted({r["harness_revision"].removesuffix("-dirty")[:7] for r in runs})
print(f"{len(runs)} runs ({dirty} dirty-stamped, {len(refused)} refused), {len(payloads)} payloads, "
      f"bundled at {head[:7]}, run revisions {' '.join(revisions)}")
