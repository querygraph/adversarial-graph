#!/usr/bin/env python3
"""Freeze the run bundles under reports/ into a site evidence directory.

    scripts/bundle-site-evidence.py OUT_DIR

Copies every reports/<run>/report.json (and results.jsonl when present)
into OUT_DIR/<run>/, then writes OUT_DIR/manifest.json with the harness
source revision, the dataset manifest digest, and the byte count and
SHA-256 of every payload, sorted by path. The manifest is what the site
verifier checks; nothing under OUT_DIR is edited afterwards.
"""
import hashlib, json, os, shutil, subprocess, sys
root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
out = os.path.abspath(sys.argv[1])
if os.path.exists(out) and os.listdir(out):
    sys.exit(f"refusing to write into non-empty {out}")
os.makedirs(out, exist_ok=True)
rev = subprocess.check_output(["git", "-C", root, "rev-parse", "HEAD"], text=True).strip()
dirty = subprocess.check_output(["git", "-C", root, "status", "--porcelain"], text=True).strip() != ""
sha = lambda p: hashlib.sha256(open(p, "rb").read()).hexdigest()
payloads, runs = [], []
for run in sorted(os.listdir(os.path.join(root, "reports"))):
    src = os.path.join(root, "reports", run)
    if not os.path.isfile(os.path.join(src, "report.json")):
        continue
    os.makedirs(os.path.join(out, run))
    for name in ["report.json", "results.jsonl"]:
        if os.path.isfile(os.path.join(src, name)):
            shutil.copyfile(os.path.join(src, name), os.path.join(out, run, name))
            p = os.path.join(out, run, name)
            payloads.append({"path": f"{run}/{name}", "bytes": os.path.getsize(p), "sha256": sha(p)})
    report = json.load(open(os.path.join(src, "report.json")))
    runs.append({"run": run, "smoke": bool(report.get("summary", {}).get("smoke")),
                 "results": len(report.get("results", [])),
                 "hard_gate_total": sum(sum(r["gates"].values()) for r in report.get("results", []))})
manifest = {
    "schema": "adversarial-graph-strain-evidence-v1",
    "track": "strain",
    "harness": "adversarial-graph",
    "harness_revision": rev + ("-dirty" if dirty else ""),
    "grust_graph_version": "0.13.0",
    "dataset_manifest_sha256": sha(os.path.join(root, "datasets", "MANIFEST.json")),
    "host": "shared laptop, 1-minute load average 300-950 during every run; wall times are upper bounds",
    "runs": runs,
    "payloads": sorted(payloads, key=lambda p: p["path"]),
}
json.dump(manifest, open(os.path.join(out, "manifest.json"), "w"), indent=2, sort_keys=True)
open(os.path.join(out, "manifest.json"), "a").write("\n")
print(f"{len(runs)} runs, {len(payloads)} payloads, revision {manifest['harness_revision']}")
