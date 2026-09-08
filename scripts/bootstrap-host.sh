#!/usr/bin/env bash
# Bootstrap a fresh Debian 13 x86_64 host as a benchmark host for the strain
# ledger and the LSQB matrix: packages, Rust, the three repositories, the
# datasets, the harness build and the Docker images. Idempotent; run as the
# benchmark user (needs sudo for apt). Docker itself is expected from the
# host's own bootstrap (the other hosts run Docker 29 with Compose v5); if
# it is missing this script says so and stops.
#
#   AG_HOST=quegee scripts/bootstrap-host.sh   # everything below
#   AG_HOST=quegee AG_SKIP_BUILD=1 scripts/bootstrap-host.sh
#
# AG_HOST is required and is the host's name in FABLE-TO-FABLE, in the ssh
# config of every other host, and on the tailnet. A host cloned from another
# host's image inherits that host's name, and then two machines answer to one
# name: `ssh grust` reaches one of them, the tailnet shows `grust` and
# `grust-1`, and a log line naming a host means nothing. Set it here, once,
# before anything else runs.
set -euo pipefail
: "${AG_HOST:?set AG_HOST to the name of this host, e.g. AG_HOST=quegee}"
[[ "$AG_HOST" =~ ^[a-z][a-z0-9-]{0,62}$ ]] || {
  echo "AG_HOST must be a lowercase DNS label: $AG_HOST"; exit 1; }
if [ "$(hostnamectl --static 2>/dev/null)" != "$AG_HOST" ]; then
  echo "naming this host $AG_HOST (was $(hostnamectl --static 2>/dev/null || echo unknown))"
  sudo hostnamectl set-hostname "$AG_HOST"
  # Debian's cloud images map 127.0.1.1 to the old name; leave any other
  # 127.0.1.1 line alone rather than guessing at a host's own /etc/hosts.
  sudo sed -i -E "s/^(127\.0\.1\.1[[:space:]]+).*$/\1${AG_HOST}/" /etc/hosts
fi
# Tailscale keeps the name it registered with, so a renamed host stays wrong on
# the tailnet until it is told. Harmless when Tailscale is not joined.
if command -v tailscale >/dev/null 2>&1 && tailscale status >/dev/null 2>&1; then
  sudo tailscale set --hostname="$AG_HOST" || true
fi
command -v docker >/dev/null || { echo "docker is missing: run the host bootstrap first"; exit 1; }
docker compose version >/dev/null 2>&1 || { echo "docker compose v2 is missing"; exit 1; }
sudo apt-get install -y -qq jq git curl build-essential pkg-config libssl-dev cmake clang protobuf-compiler python3 nodejs npm >/dev/null
command -v cargo >/dev/null || { curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal >/dev/null; source "$HOME/.cargo/env"; }
mkdir -p ~/src && cd ~/src
for repo in adversarial-graph adversarial-site grust; do
  [ -d "$repo" ] || git clone -q "git@github.com:querygraph/$repo.git"
  git -C "$repo" pull -q --ff-only || true
done
cd ~/src/adversarial-graph
[ -f datasets/wiki-Talk.txt.gz ] || scripts/fetch-datasets.sh
if [ -z "${AG_SKIP_BUILD:-}" ]; then
  cargo build --release --features postgres,surreal,falkor,lancedb,neo4j,helix,ladybug,age 2>&1 | grep -E "^error|Finished" -A6
  (cd ~/src/grust/benchmarks/lsqb && cargo build --release 2>&1 | grep -E "^error|Finished" -A6)
fi
docker compose --profile external pull -q neo4j memgraph falkor postgres age surreal helix 2>&1 | tail -2 || true
(cd ~/src/adversarial-site && npm install --silent >/dev/null 2>&1 && node scripts/verify-strain-evidence.mjs | tail -1)
./target/release/ag backends 2>/dev/null | awk '/^[a-z]/{print $1}' | tr '\n' ' '; echo
echo "BOOTSTRAP_DONE $(date -u +%H:%M:%SZ)"
