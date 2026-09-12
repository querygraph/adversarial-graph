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
# --cap is the load budget and, separately, the families' budget: the harness
# ends a load at AG_LOAD_BUDGET_S with its gate, and a store whose measured
# rate on this host projects past it is not sent (the row says so); the
# pair's own timeout is twice the cap, so a store that loads inside its
# budget gets the families under theirs.
CAP=7200; DATASETS="wiki-Talk,roadNet-CA,web-Google,cit-Patents,soc-LiveJournal1,com-Orkut"
while [ $# -gt 0 ]; do case "$1" in --cap) CAP=$2; shift 2;; --datasets) DATASETS=$2; shift 2;; *) break;; esac; done
BACKENDS=("$@"); [ ${#BACKENDS[@]} -eq 0 ] && BACKENDS=(memory turso-wal turso-mvcc postgres neo4j neo4j-http falkor lancedb)
service_for() { case "$1" in postgres) echo postgres;; surreal-*) echo surreal;; falkor) echo falkor;; helix-sdk) echo helix-sdk;; helix-*) echo helix;; neo4j*) echo neo4j;; memgraph) echo memgraph;; age) echo age;; *) echo "";; esac; }
# A service that never becomes ready is a recorded failure of the pair, not
# a ladder that waits forever: every probe runs under AG_READY_TIMEOUT
# (default 600 s) and a miss is logged with the service and the deadline.
READY_TIMEOUT="${AG_READY_TIMEOUT:-600}"
# PostgreSQL and AGE answer pg_isready during the image's first-boot
# initialization and then restart the server; a pair opened in that window
# saw "connection closed" and failed at open (lakecat, 2026-09-10, both age
# tiers in the same second). Readiness is a real query, twice, 3 s apart.
pg_ready() { # $1 = compose service
  docker compose exec -T "$1" psql -U postgres -d graph -tAc 'select 1' 2>/dev/null | grep -qx 1 || return 1
  sleep 3
  docker compose exec -T "$1" psql -U postgres -d graph -tAc 'select 1' 2>/dev/null | grep -qx 1
}
ready_probe() { case "$1" in
  postgres) pg_ready postgres;;
  falkor) docker compose exec -T falkor redis-cli ping 2>/dev/null | grep -q PONG;;
  neo4j) curl -sf -m 2 http://127.0.0.1:17474 >/dev/null;;
  memgraph) echo 'RETURN 1;' | docker compose exec -T memgraph mgconsole >/dev/null 2>&1;;
  age) pg_ready age;;
  surreal) curl -sf -m 2 http://127.0.0.1:18000/health >/dev/null 2>&1;;
  helix) curl -sf -m 2 http://127.0.0.1:18082/health >/dev/null 2>&1;;
  helix-sdk) curl -sf -m 2 http://127.0.0.1:18083/healthz >/dev/null 2>&1 && curl -sf -m 2 http://127.0.0.1:18083/readyz >/dev/null 2>&1;;
  *) return 0;;
esac; }
wait_ready() { # $1 = compose service; returns 1 and logs on deadline
  local deadline=$(( $(date +%s) + READY_TIMEOUT ))
  until ready_probe "$1"; do
    if [ "$(date +%s)" -ge "$deadline" ]; then
      echo "## $1: not ready after ${READY_TIMEOUT}s at $(date -u +%H:%M:%SZ); the pair is not started"; return 1
    fi
    sleep 1
  done
  [ "$1" = neo4j ] && sleep 5
  return 0
}
# Whatever ends the ladder -- a cap, a guard, a signal, an error under
# set -e -- the store containers it started are stopped.
trap 'docker compose --profile external stop >/dev/null 2>&1 || true' EXIT
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
    cap_end=$(( start_s + 2 * CAP + 300 ))
    IFS=, read -ra WINDOWS <<<"$BLACKOUTS"
    for w in "${WINDOWS[@]}"; do
      local a b as bs
      a=${w%-*}; b=${w#*-}
      as=$(( 10#${a%:*} * 3600 + 10#${a#*:} * 60 )); bs=$(( 10#${b%:*} * 3600 + 10#${b#*:} * 60 ))
      # A window we are in right now, one later today that the pair's cap
      # would still be running into, or -- when the cap crosses midnight --
      # tomorrow's occurrence of the same window.
      if { [ "$start_s" -ge "$as" ] && [ "$start_s" -lt "$bs" ]; } \
         || { [ "$as" -ge "$start_s" ] && [ "$as" -lt "$cap_end" ]; } \
         || { [ "$cap_end" -gt 86400 ] && [ $(( as + 86400 )) -lt "$cap_end" ]; }; then blocked="$w"; break; fi
    done
    if [ -z "$blocked" ]; then
      if [ -n "$waited" ] && [ -n "${1:-}" ]; then docker compose --profile external up -d "$1" >/dev/null 2>&1; wait_ready "$1" || return 1; fi
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
  if [ -n "$svc" ]; then
    # A fresh container, never the one the previous backend's ladder stopped:
    # `up -d` restarts a stopped container with its data, and on 2026-09-10
    # a Neo4j started that way spent six hours recovering the previous
    # pair's 117 M edges before reporting "Database 'neo4j' unavailable".
    echo "## $b: starting $svc"; docker compose --profile external rm -f -s "$svc" >/dev/null 2>&1 || true; docker compose --profile external up -d "$svc" >/dev/null 2>&1
    if ! wait_ready "$svc"; then echo "## $b: skipped, service never became ready; no bundle"; docker compose --profile external stop "$svc" >/dev/null 2>&1 || true; continue; fi
  fi
  first_pair=1
  for d in "${DS[@]}"; do
    wait_for_window "$svc"
    # Every pair starts from a fresh container: the harness clears a store
    # at open, but a store left at its memory limit by the previous tier
    # (Memgraph at GAP-road, 2026-09-10) drops the connection under the
    # clear, and the next tier fails at open on the residue.
    if [ -n "$svc" ] && [ "$first_pair" -eq 0 ]; then
      docker compose --profile external rm -f -s "$svc" >/dev/null 2>&1 || true
      docker compose --profile external up -d "$svc" >/dev/null 2>&1
      if ! wait_ready "$svc"; then echo "## $b $d: $svc not ready after a fresh start; the pair is not started"; continue; fi
    fi
    first_pair=0
    echo "## $b $d: start $(date -u +%H:%M:%SZ)"
    # The pair's own output is kept so the decision below reads the
    # harness's completion marker, not the exit code: `ag run` exits 1 when a
    # hard gate fails, and a failing gate is a finding, not a reason to skip
    # the larger tiers. Only a run that never reached its final write (cap,
    # host guard, crash) stops the climb.
    pairlog=$(mktemp -t ag-pair.XXXXXX)
    # No pipeline here: with `| tee` the pid in $! was tee's, so a guard that
    # walks down from the pair it was handed found no `ag` at all. The pair
    # writes its own log and the ladder echoes it after; the guard is handed
    # `timeout`, whose child is the harness.
    # -k: a pair that does not exit on SIGTERM at the cap is killed a minute later.
    AG_LOAD_BUDGET_S="$CAP" timeout -k 60 "$((2 * CAP))" ./target/release/ag run --dataset "$d" --backend "$b" --out reports >"$pairlog" 2>&1 &
    run=$!; guard "$run" & g=$!
    rc=0; wait "$run" || rc=$?; kill "$g" 2>/dev/null || true # neither a failing pair nor an already-exited guard may end the ladder
    cat "$pairlog"
    if grep -q "^== report:" "$pairlog"; then
      echo "## $b $d: done $(date -u +%H:%M:%SZ) (exit $rc; gates are in the bundle)"
      # A load the store could not finish inside its budget, or one the
      # harness projected past it, is a row; a larger tier is not.
      if grep -qE "not attempted: .* load budget|did not finish inside the .* load budget" "$pairlog"; then
        echo "## $b: not trying larger tiers after the load budget at $d"; rm -f "$pairlog"; break
      fi
      # A container the kernel took at its memory limit is a placement too,
      # and a larger tier will not fit either: 22 rows on 2026-09-11 said
      # what the first one said.
      if grep -q "OOMKilled" "$pairlog"; then
        echo "## $b: not trying larger tiers after the container's memory limit at $d"; rm -f "$pairlog"; break
      fi
    else
      echo "## $b $d: exit $rc after the pair cap $((2 * CAP))s, host memory guard, or crash $(date -u +%H:%M:%SZ); no complete bundle; not trying larger tiers for $b"; rm -f "$pairlog"; break
    fi
    rm -f "$pairlog"
  done
  if [ -n "$svc" ]; then docker compose --profile external stop "$svc" >/dev/null 2>&1; fi
done
echo "FULL_TIERS_DONE"
