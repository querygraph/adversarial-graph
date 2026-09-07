#!/usr/bin/env bash
# Full-graph tiers, one (backend, dataset) invocation at a time, smallest
# dataset first, with a wall-clock cap per invocation. A backend that hits
# the cap on a dataset is not tried on larger ones; the log says where it
# stopped. Only the container a backend needs is up while it runs.
#
#   scripts/run-full-tiers.sh [--cap SECONDS] [--datasets a,b,c] [backend ...]
set -euo pipefail
cd "$(dirname "$0")/.."
CAP=7200; DATASETS="wiki-Talk,roadNet-CA,web-Google,cit-Patents,soc-LiveJournal1,com-Orkut"
while [ $# -gt 0 ]; do case "$1" in --cap) CAP=$2; shift 2;; --datasets) DATASETS=$2; shift 2;; *) break;; esac; done
BACKENDS=("$@"); [ ${#BACKENDS[@]} -eq 0 ] && BACKENDS=(memory turso-wal turso-mvcc postgres neo4j neo4j-http falkor lancedb)
service_for() { case "$1" in postgres) echo postgres;; surreal-*) echo surreal;; falkor) echo falkor;; helix-*) echo helix;; neo4j*) echo neo4j;; memgraph) echo memgraph;; age) echo age;; *) echo "";; esac; }
wait_ready() { case "$1" in
  postgres) until docker compose exec -T postgres pg_isready -U postgres -d graph >/dev/null 2>&1; do sleep 1; done;;
  falkor) until docker compose exec -T falkor redis-cli ping 2>/dev/null | grep -q PONG; do sleep 1; done;;
  neo4j) until curl -sf -m 2 http://127.0.0.1:17474 >/dev/null; do sleep 2; done; sleep 5;;
  memgraph) until echo 'RETURN 1;' | docker compose exec -T memgraph mgconsole >/dev/null 2>&1; do sleep 1; done;;
  age) until docker compose exec -T age pg_isready -U postgres -d graph >/dev/null 2>&1; do sleep 1; done;;
esac; }
IFS=, read -ra DS <<<"$DATASETS"
for b in "${BACKENDS[@]}"; do
  svc=$(service_for "$b")
  if [ -n "$svc" ]; then echo "## $b: starting $svc"; docker compose --profile external up -d "$svc" >/dev/null 2>&1; wait_ready "$svc"; fi
  for d in "${DS[@]}"; do
    echo "## $b $d: start $(date -u +%H:%M:%SZ)"
    if timeout "$CAP" ./target/release/ag run --dataset "$d" --backend "$b" --out reports; then
      echo "## $b $d: done $(date -u +%H:%M:%SZ)"
    else
      rc=$?; echo "## $b $d: exit $rc after cap ${CAP}s or failure $(date -u +%H:%M:%SZ); not trying larger tiers for $b"; break
    fi
  done
  if [ -n "$svc" ]; then docker compose --profile external stop "$svc" >/dev/null 2>&1; fi
done
echo "FULL_TIERS_DONE"
