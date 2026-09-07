#!/usr/bin/env bash
# Pause a benchmark run while this host's other tenant refits.
#
# The grust box is shared, and that tenancy is disclosed on every row through
# host_loadavg and host_steal_us. Two of the tenant's jobs are large enough to
# change a measurement rather than just tint it: the Eigen Times v2 export
# (~18 min, ~20 GB resident) at 15:00 UTC and the Eigen Hacks rebuild (~9 min)
# at 13:00 UTC. A tier running through one of those can slow enough to reach
# the wall-clock cap, which would be recorded as a store finding when the cause
# was tenancy -- the one confusion AGENTS.md rules out.
#
# So the benchmark yields: while either service is active, every `ag run` (and
# the timeout wrapping it, so the cap is not delivered mid-pause) is stopped,
# and resumed when the service finishes. Each pause is written to
# reports/host-pauses.txt with its span.
#
# The measurement consequence, which the run's notes must carry: a paused
# pair's wall time includes the pause and is an upper bound; its CPU columns
# are unaffected. GNU timeout's alarm is real-time, so a pause still consumes
# the cap -- a pair that both paused and hit the cap is rerun, not reported.
set -uo pipefail
cd "$(dirname "$0")/.."
SERVICES=(eigentimes-v2.service eigenhacks-daily.service)
NOTES="${NOTES:-reports/host-pauses.txt}"
mkdir -p "$(dirname "$NOTES")"

busy() { for s in "${SERVICES[@]}"; do
    systemctl is-active --quiet "$s" && { echo "$s"; return 0; }; done; return 1; }
pids() { pgrep -f "release/ag run"; pgrep -f "timeout --foreground"; }

paused=""
while true; do
  if svc=$(busy); then
    if [ -z "$paused" ]; then
      paused=$(date +%s)
      for p in $(pids); do kill -STOP "$p" 2>/dev/null; done
      printf '%s\tpause\t%s\tag stopped for the tenant refit\n' \
        "$(date -u +%FT%TZ)" "$svc" >> "$NOTES"
    fi
  elif [ -n "$paused" ]; then
    for p in $(pids); do kill -CONT "$p" 2>/dev/null; done
    printf '%s\tresume\tafter %ss\twall time of any pair spanning this is an upper bound\n' \
      "$(date -u +%FT%TZ)" "$(( $(date +%s) - paused ))" >> "$NOTES"
    paused=""
  fi
  sleep 15
done
