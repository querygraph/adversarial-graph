#!/usr/bin/env bash
# Build the HelixDB server image the `helix-sdk` backend needs (see
# scripts/helix-sdk-server/Dockerfile for why the registry image will not do).
#   scripts/build-helix-sdk-server.sh [SOURCE_DIR] [JOBS]
# SOURCE_DIR is a HelixDB checkout; it is reset to the qualified revision.
# The image is tagged ag-helix-sdk-server:0ef3cee0 and compose picks it up as
# the `helix-sdk` service (HELIX_SDK_IMAGE overrides). Never run this on a host
# with a ladder in flight: it is an hour of every core.
set -euo pipefail
REV=0ef3cee0faf28bb81072fb149b982dcdb166d60a
SRC=${1:-$HOME/src/helixdb-0ef3cee}
JOBS=${2:-$(( $(nproc) > 2 ? $(nproc) - 2 : 1 ))}
HERE=$(cd "$(dirname "$0")" && pwd)
if [ ! -d "$SRC/.git" ]; then
  git clone -q https://github.com/HelixDB/helix-db "$SRC"
fi
git -C "$SRC" fetch -q origin "$REV" 2>/dev/null || true
git -C "$SRC" checkout -q --detach "$REV"
[ -z "$(git -C "$SRC" status --porcelain)" ] || { echo "source tree is dirty: $SRC" >&2; exit 2; }
echo "## building ag-helix-sdk-server:0ef3cee0 from $SRC at $(git -C "$SRC" rev-parse HEAD) with $JOBS jobs, $(date -u +%FT%TZ)"
docker buildx build --platform linux/amd64 --progress plain --load \
  --build-arg JOBS="$JOBS" \
  --file "$HERE/helix-sdk-server/Dockerfile" \
  --tag ag-helix-sdk-server:0ef3cee0 \
  "$SRC"
echo "## image $(docker image inspect ag-helix-sdk-server:0ef3cee0 --format '{{.Id}}') at $(date -u +%FT%TZ)"
