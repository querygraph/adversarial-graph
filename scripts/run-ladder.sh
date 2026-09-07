#!/usr/bin/env bash
# Run the smoke ladder one system at a time: start only the container a
# backend needs, run every requested dataset against it, stop the container,
# move on. This is the mode for a small dedicated host (one system under test
# resident at a time, the harness and the server sharing the cores) and it
# keeps every run's server_cpu/server_memory attributable to one process.
#
#   scripts/run-ladder.sh [--datasets wiki-Talk,roadNet-CA] [--limit-edges N]
#                         [--out reports] [backend ...]
# Backends default to every built-in one (`ag backends`). Surreal backends are
# capped at 10k edges unless --limit-edges is given (adapter O(E^2) load).
set -euo pipefail
cd "$(dirname "$0")/.."
DATASETS="wiki-Talk,roadNet-CA"; LIMIT=""; OUT="reports"; EXTRA=(--smoke)
while [ $# -gt 0 ]; do
  case "$1" in
    --datasets) DATASETS="$2"; shift 2;;
    --limit-edges) LIMIT="$2"; shift 2;;
    --out) OUT="$2"; shift 2;;
    --full) EXTRA=(); shift;;
    *) break;;
  esac
done
BACKENDS=("$@")
[ ${#BACKENDS[@]} -eq 0 ] && mapfile -t BACKENDS < <(./target/release/ag backends | awk '/^[a-z]/{print $1}')
service_for() {
  case "$1" in
    postgres) echo postgres;; surreal-*) echo surreal;; falkor) echo falkor;;
    helix-*) echo helix;; neo4j*) echo neo4j;; memgraph) echo memgraph;; age) echo age;; *) echo "";;
  esac
}
wait_ready() {
  case "$1" in
    postgres) until docker compose exec -T postgres pg_isready -U postgres -d graph >/dev/null 2>&1; do sleep 1; done;;
    falkor) until docker compose exec -T falkor redis-cli ping 2>/dev/null | grep -q PONG; do sleep 1; done;;
    surreal) until curl -sf -m 2 http://127.0.0.1:18000/health >/dev/null; do sleep 1; done;;
    helix) until curl -sf -m 2 -X POST http://127.0.0.1:18082/v1/query -H 'Content-Type: application/json' \
      -d '{"request_type":"read","query":{"queries":[],"returns":[]},"parameters":{},"parameter_types":{}}' >/dev/null; do sleep 1; done;;
    neo4j) until curl -sf -m 2 http://127.0.0.1:17474 >/dev/null; do sleep 2; done; sleep 5;;
    memgraph) until echo 'RETURN 1;' | docker compose exec -T memgraph mgconsole >/dev/null 2>&1; do sleep 1; done;;
    age) until docker compose exec -T age pg_isready -U postgres -d graph >/dev/null 2>&1; do sleep 1; done;;
  esac
}
for b in "${BACKENDS[@]}"; do
  svc=$(service_for "$b")
  lim=("${LIMIT:+--limit-edges}" "${LIMIT}")
  [ -z "$LIMIT" ] && case "$b" in surreal-*) lim=(--limit-edges 10000);; *) lim=();; esac
  if [ -n "$svc" ]; then
    echo "## $b: starting $svc"
    docker compose --profile external up -d "$svc"
    wait_ready "$svc"
  fi
  echo "## $b: running $DATASETS ${lim[*]:-}"
  ./target/release/ag run "${EXTRA[@]}" --dataset "$DATASETS" --backend "$b" ${lim[@]+"${lim[@]}"} --out "$OUT" || echo "## $b: exit $?"
  if [ -n "$svc" ]; then
    echo "## $b: stopping $svc"
    docker compose --profile external stop "$svc" >/dev/null
  fi
done
echo "LADDER_DONE"
