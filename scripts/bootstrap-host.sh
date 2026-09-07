#!/usr/bin/env bash
# Bootstrap a fresh Debian 13 x86_64 host as a benchmark host for the strain
# ledger and the LSQB matrix: packages, Rust, the three repositories, the
# datasets, the harness build and the Docker images. Idempotent; run as the
# benchmark user (needs sudo for apt). Docker itself is expected from the
# host's own bootstrap (the other hosts run Docker 29 with Compose v5); if
# it is missing this script says so and stops.
#
#   scripts/bootstrap-host.sh            # everything below
#   AG_SKIP_BUILD=1 scripts/bootstrap-host.sh
set -euo pipefail
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
