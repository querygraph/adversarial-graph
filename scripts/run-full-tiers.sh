#!/usr/bin/env bash
# Full-graph tiers, one (backend, dataset) invocation at a time, smallest
# dataset first, with a wall-clock cap per invocation. A backend that hits
# the cap on a dataset is not tried on larger ones; the log says where it
# stopped. Only the container a backend needs is up while it runs.
#
#   scripts/run-full-tiers.sh [--cap SECONDS] [--datasets a,b,c] [backend ...]
#   AG_RSS_LIMIT_GB=13 AG_MEM_AVAILABLE_MIN_GB=1 …   host memory guard (§13, §15)
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
# Host memory guard. A run whose resident set passes AG_RSS_LIMIT_GB, or
# that leaves the host with less than AG_MEM_AVAILABLE_MIN_GB of available
# memory (the harness plus the store's container plus everything else
# resident), is killed and the cell is logged as host.memory-exceeded: a
# host-capacity outcome, never a store finding. Rerun that tier on a host it
# fits (see FABLE-TO-FABLE §13). The available-memory floor is what catches
# the failure mode the RSS limit cannot: a 6 GB client next to a 6 GiB
# container on a 15 GiB host thrashes without any single process being
# large (§15). Default: no guard.
RSS_LIMIT_GB="${AG_RSS_LIMIT_GB:-}"
AVAIL_MIN_GB="${AG_MEM_AVAILABLE_MIN_GB:-}"
mem_available_kb() { awk '/^MemAvailable:/ {print $2}' /proc/meminfo 2>/dev/null || true; }
# Only this ladder's own harness process: the `ag` descended from the pair
# the guard was handed, never the `timeout` wrapper, never a log watcher whose
# command line mentions it, and never another ladder's or a probe's `ag` on
# the same host (a host-wide match could kill a run this guard does not own).
ag_pids() { # $1 = pid of the background pair
  local child
  for child in $(pgrep -P "$1" 2>/dev/null); do
    if ps -o args= -p "$child" 2>/dev/null | grep -q '^\./target/release/ag run'; then echo "$child"
    else ag_pids "$child"; fi
  done
}
guard() { # $1 = pid of the background pair
  # The guard runs as a background subshell under the script's `set -e`;
  # a missing /proc/meminfo (macOS) or a pid that exited between pgrep and
  # ps would end it silently, and the parent's later `kill` of a guard that
  # is already gone would then end the ladder. Not errexit in here.
  set +e
  [ -z "$RSS_LIMIT_GB" ] && [ -z "$AVAIL_MIN_GB" ] && return 0
  local rss_lim=$((${RSS_LIMIT_GB:-0} * 1024 * 1024)) avail_min=$((${AVAIL_MIN_GB:-0} * 1024 * 1024))
  local low=0 # consecutive readings under the floor; two in a row (10 s) kill
  while kill -0 "$1" 2>/dev/null; do
    local avail; avail=$(mem_available_kb)
    if [ "$avail_min" -gt 0 ] && [ -n "$avail" ] && [ "$avail" -lt "$avail_min" ]; then low=$((low + 1)); else low=0; fi
    for pid in $(ag_pids "$1"); do
      rss=$(ps -o rss= -p "$pid" 2>/dev/null | tr -d ' ')
      [ -z "$rss" ] && continue
      if [ "$rss_lim" -gt 0 ] && [ "$rss" -gt "$rss_lim" ]; then
        echo "## host.memory-exceeded: ag pid $pid rss $((rss / 1048576)) GB > ${RSS_LIMIT_GB} GB at $(date -u +%H:%M:%SZ); killing"
        kill "$pid"
      elif [ "$low" -ge 2 ]; then
        echo "## host.memory-exceeded: host MemAvailable ${avail} kB < ${AVAIL_MIN_GB} GB floor twice in a row, ag pid $pid at rss ${rss} kB, at $(date -u +%H:%M:%SZ); killing"
        kill "$pid"
      fi
    done
    sleep 5
  done
}
# Tenancy blackouts. AG_BLACKOUT_UTC="HH:MM-HH:MM,..." names windows (UTC)
# during which a co-tenant's job owns the host's memory; no pair is started
# whose cap could still be running when a window opens, and the ladder
# sleeps until the window closes. Every wait is logged, so a host's rows
# carry the schedule they were taken under.
BLACKOUTS="${AG_BLACKOUT_UTC:-}"
wait_for_window() { # $1 = the backend's compose service, stopped while waiting
  [ -z "$BLACKOUTS" ] && return 0
  local waited=""
  while true; do
    local now start_s cap_end blocked="" w
    # 10# forces decimal: at 08:xx or 09:xx UTC a bare "08" is invalid octal
    # and the arithmetic -- and, under set -e, the ladder -- dies.
    now=$(date -u +%s); start_s=$(( 10#$(date -u +%H) * 3600 + 10#$(date -u +%M) * 60 + 10#$(date -u +%S) ))
    cap_end=$(( start_s + CAP + 300 ))
    IFS=, read -ra WINDOWS <<<"$BLACKOUTS"
    for w in "${WINDOWS[@]}"; do
      local a b as bs
      a=${w%-*}; b=${w#*-}
      as=$(( 10#${a%:*} * 3600 + 10#${a#*:} * 60 )); bs=$(( 10#${b%:*} * 3600 + 10#${b#*:} * 60 ))
      # A window later today that the pair would still be inside, or one we
      # are in right now; windows are compared on the same day (they are
      # short and never span midnight).
      if { [ "$start_s" -ge "$as" ] && [ "$start_s" -lt "$bs" ]; } || { [ "$as" -ge "$start_s" ] && [ "$as" -lt "$cap_end" ]; }; then blocked="$w"; break; fi
    done
    if [ -z "$blocked" ]; then
      if [ -n "$waited" ] && [ -n "${1:-}" ]; then docker compose --profile external up -d "$1" >/dev/null 2>&1; wait_ready "$1"; fi
      return 0
    fi
    if [ -z "$waited" ] && [ -n "${1:-}" ]; then docker compose --profile external stop "$1" >/dev/null 2>&1; fi
    waited=1
    b=${blocked#*-}; bs=$(( 10#${b%:*} * 3600 + 10#${b#*:} * 60 ))
    local sleep_s=$(( bs - start_s )); [ "$sleep_s" -le 0 ] && sleep_s=$(( sleep_s + 86400 ))
    echo "## tenancy blackout $blocked: waiting $((sleep_s / 60)) min from $(date -u +%H:%M:%SZ) before the next pair"
    sleep "$sleep_s"
  done
}
for b in "${BACKENDS[@]}"; do
  svc=$(service_for "$b")
  if [ -n "$svc" ]; then echo "## $b: starting $svc"; docker compose --profile external up -d "$svc" >/dev/null 2>&1; wait_ready "$svc"; fi
  for d in "${DS[@]}"; do
    wait_for_window "$svc"
    echo "## $b $d: start $(date -u +%H:%M:%SZ)"
    # The pair's own output is kept so the decision below reads the
    # harness's completion marker, not the exit code: `ag run` exits 1 when a
    # hard gate fails, and a failing gate is a finding, not a reason to skip
    # the larger tiers. Only a run that never reached its final write (cap,
    # host guard, crash) stops the climb.
    pairlog=$(mktemp -t ag-pair.XXXXXX)
    timeout "$CAP" ./target/release/ag run --dataset "$d" --backend "$b" --out reports 2>&1 | tee "$pairlog" &
    run=$!; guard "$run" & g=$!
    rc=0; wait "$run" || rc=$?; kill "$g" 2>/dev/null || true # neither a failing pair nor an already-exited guard may end the ladder
    if grep -q "^== report:" "$pairlog"; then
      echo "## $b $d: done $(date -u +%H:%M:%SZ) (exit $rc; gates are in the bundle)"
    else
      echo "## $b $d: exit $rc after cap ${CAP}s, host memory guard, or crash $(date -u +%H:%M:%SZ); no complete bundle; not trying larger tiers for $b"; rm -f "$pairlog"; break
    fi
    rm -f "$pairlog"
  done
  if [ -n "$svc" ]; then docker compose --profile external stop "$svc" >/dev/null 2>&1; fi
done
echo "FULL_TIERS_DONE"
