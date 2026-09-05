#!/usr/bin/env python3
"""Freeze the run bundles under reports/ into a site evidence directory.

    scripts/bundle-site-evidence.py OUT_DIR --host "DESCRIPTION" [--since RUN]

Copies every reports/<run>/report.json (and results.jsonl when present)
into OUT_DIR/<run>/, then writes OUT_DIR/manifest.json with the harness
source revision, the dataset manifest digest, and the byte count and
SHA-256 of every payload, sorted by path. The manifest is what the site
verifier checks; nothing under OUT_DIR is edited afterwards.

`--host` is the one-line description of the machine the runs were taken
on, recorded verbatim in the manifest. `--since RUN` includes only bundles
whose stamp is at or after RUN (e.g. `20260905T082424Z`), for a publication
that covers one host's runs out of a shared reports directory. The manifest
shape is the one the site verifier pins (`verify-strain-evidence.mjs`);
each report.json carries its own `harness_revision` for per-run provenance.
"""
import argparse, hashlib, json, os, shutil, subprocess, sys
parser = argparse.ArgumentParser()
parser.add_argument("out_dir")
parser.add_argument("--host", required=True, help="one-line description of the host the runs were taken on")
parser.add_argument("--since", default="", help="include only run stamps >= this value")
args = parser.parse_args()
root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
out = os.path.abspath(args.out_dir)
if os.path.exists(out) and os.listdir(out):
    sys.exit(f"refusing to write into non-empty {out}")
os.makedirs(out, exist_ok=True)
rev = subprocess.check_output(["git", "-C", root, "rev-parse", "HEAD"], text=True).strip()
dirty = subprocess.check_output(["git", "-C", root, "status", "--porcelain"], text=True).strip() != ""
sha = lambda p: hashlib.sha256(open(p, "rb").read()).hexdigest()
payloads, runs = [], []
grust_versions = set()
for run in sorted(os.listdir(os.path.join(root, "reports"))):
    src = os.path.join(root, "reports", run)
    if not os.path.isfile(os.path.join(src, "report.json")) or run < args.since:
        continue
    os.makedirs(os.path.join(out, run))
    for name in ["report.json", "results.jsonl"]:
        if os.path.isfile(os.path.join(src, name)):
            shutil.copyfile(os.path.join(src, name), os.path.join(out, run, name))
            p = os.path.join(out, run, name)
            payloads.append({"path": f"{run}/{name}", "bytes": os.path.getsize(p), "sha256": sha(p)})
    report = json.load(open(os.path.join(src, "report.json")))
    grust_versions.add(report.get("grust_version", "0.13.0"))
    runs.append({"run": run, "smoke": bool(report.get("summary", {}).get("smoke")),
                 "results": len(report.get("results", [])),
                 "hard_gate_total": sum(sum(r["gates"].values()) for r in report.get("results", []))})
manifest = {
    "schema": "adversarial-graph-strain-evidence-v1",
    "track": "strain",
    "harness": "adversarial-graph",
    "harness_revision": rev + ("-dirty" if dirty else ""),
    "grust_graph_version": grust_versions.pop() if len(grust_versions) == 1 else sys.exit(f"runs mix grust-graph versions: {sorted(grust_versions)}"),
    "dataset_manifest_sha256": sha(os.path.join(root, "datasets", "MANIFEST.json")),
    "host": args.host,
    "runs": runs,
    "payloads": sorted(payloads, key=lambda p: p["path"]),
}
json.dump(manifest, open(os.path.join(out, "manifest.json"), "w"), indent=2, sort_keys=True)
open(os.path.join(out, "manifest.json"), "a").write("\n")
print(f"{len(runs)} runs, {len(payloads)} payloads, revision {manifest['harness_revision']}")
