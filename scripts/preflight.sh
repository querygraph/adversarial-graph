#!/usr/bin/env bash
# Post-compute preflight: exercise everything that runs AFTER a measurement --
# the harness's report writer, the site bundler, the row renderer and the site's
# own verifier -- on a one-minute smoke output, before any window is spent.
# Every post-compute bug of §40-§44 surfaced only at the end of a long run;
# this makes such a bug cost a minute. Run it before every ladder, and after
# every change to src/report.rs, src/main.rs, the bundler, the renderer or
# the site verifier. Exit non-zero on any failure.
#
#   scripts/preflight.sh            # memory backend, wiki-Talk 200k slice
#   scripts/preflight.sh falkor     # a containerized backend (needs compose)
set -euo pipefail
cd "$(dirname "$0")/.."
backend=${1:-memory}
site=${AG_SITE:-$HOME/src/adversarial-site}
work=$(mktemp -d "${TMPDIR:-/tmp}/ag-preflight.XXXXXX")
trap 'rm -rf "$work"' EXIT
say() { printf '## preflight: %s\n' "$*"; }

[ -x target/release/ag ] || { say "no release binary; build first"; exit 1; }
say "harness $(git rev-parse --short HEAD) $( [ -z "$(git status --porcelain --untracked-files=no)" ] && echo clean || echo DIRTY )"

say "1/4 smoke run: $backend on wiki-Talk 200k"
./target/release/ag run --dataset wiki-Talk --backend "$backend" --smoke --out "$work/reports" >"$work/run.log" 2>&1 \
  || { say "harness run failed"; tail -5 "$work/run.log"; exit 1; }
run=$(ls "$work/reports" | head -1)
python3 - "$work/reports/$run/report.json" <<'PY'
import json, sys
d = json.load(open(sys.argv[1]))
assert d["summary"]["complete"] is True, "smoke report not marked complete"
assert d["results"], "smoke report has no cells"
assert d["harness_revision"] and len(d["harness_revision"].split("-")[0]) == 40, "no full harness revision"
bad = [r for r in d["results"] if r["outcome"] == "pass" and sum(r["gates"].values()) > 0]
assert not bad, f"a pass carries hard gates: {[(r['scenario'], r['gates']) for r in bad]}"
print("   report ok:", len(d["results"]), "cells,", d["summary"]["outcomes"])
PY

say "2/4 bundle into a site evidence directory"
python3 scripts/bundle-site-evidence.py "$work/pub" --reports "$work/reports" --host "preflight smoke, not a publication" >"$work/bundle.log" 2>&1 \
  || { say "bundler failed"; cat "$work/bundle.log"; exit 1; }
tail -1 "$work/bundle.log" | sed 's/^/   /'

say "3/4 render the site rows"
python3 scripts/render-results.py --html-rows preflight --reports "$work/reports" >"$work/rows.html" \
  || { say "renderer failed"; exit 1; }
grep -c '^<tr>' "$work/rows.html" | sed 's/^/   rows: /'

say "4/4 the site's own verifier on the bundle"
node --input-type=module -e "
import { verifyPublication } from '$site/scripts/verify-strain-evidence.mjs';
const s = verifyPublication('$work/pub', 'preflight');
console.log('   verified:', s.runs, 'run(s),', s.results, 'result(s), hard-gate total', s.gateTotal, 'schema', s.schema);
" || { say "site verifier rejected the bundle"; exit 1; }

say "PASS: harness -> bundle -> rows -> site verifier all agree on this revision"
