#!/usr/bin/env bash
# Fetch the GRAPH-ADVERSARIAL-v1 dataset ladder into datasets/ and record
# byte sizes and SHA-256 digests in datasets/MANIFEST.json.
#
#   scripts/fetch-datasets.sh            # tiers S and M (~2.2 GB)
#   scripts/fetch-datasets.sh --large    # also tier L (twitter-2010 5.5 GB, friendster 9.4 GB, GAP)
#   scripts/fetch-datasets.sh --only wiki-Talk,roadNet-CA
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
dest="${AG_DATASET_DIR:-$root/datasets}"
mkdir -p "$dest"
large=0; only=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --large) large=1 ;;
    --only) only="$2"; shift ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
  shift
done

# name|tier|url|pathology
DATASETS=(
  "email-Eu-core|S|https://snap.stanford.edu/data/email-Eu-core.txt.gz|labels"
  "email-Eu-core-labels|S|https://snap.stanford.edu/data/email-Eu-core-department-labels.txt.gz|labels"
  "ego-Facebook|S|https://snap.stanford.edu/data/facebook_combined.txt.gz|small-social"
  "wiki-Talk|S|https://snap.stanford.edu/data/wiki-Talk.txt.gz|super-node,small-scc"
  "roadNet-CA|S|https://snap.stanford.edu/data/roadNet-CA.txt.gz|high-diameter"
  "web-Google|S|https://snap.stanford.edu/data/web-Google.txt.gz|many-components"
  "ldbc-snb-sf0.1|S|https://datasets.ldbcouncil.org/snb-interactive-v1/social_network-sf0.1-CsvBasic-LongDateFormatter.tar.zst|typed-property-graph"
  "icij-offshore-leaks|S|https://offshoreleaks-data.icij.org/offshoreleaks/csv/full-oldb.LATEST.zip|typed-property-graph"
  "cit-Patents|M|https://snap.stanford.edu/data/cit-Patents.txt.gz|dag"
  "ldbc-snb-sf1|M|https://datasets.ldbcouncil.org/snb-interactive-v1/social_network-sf1-CsvBasic-LongDateFormatter.tar.zst|typed-property-graph"
  "soc-Pokec-relationships|M|https://snap.stanford.edu/data/soc-pokec-relationships.txt.gz|typed-property-graph"
  "soc-Pokec-profiles|M|https://snap.stanford.edu/data/soc-pokec-profiles.txt.gz|typed-property-graph"
  "soc-LiveJournal1|M|https://snap.stanford.edu/data/soc-LiveJournal1.txt.gz|degree-skew"
  "com-Orkut|M|https://snap.stanford.edu/data/bigdata/communities/com-orkut.ungraph.txt.gz|dense-communities"
  "sx-stackoverflow|M|https://snap.stanford.edu/data/sx-stackoverflow.txt.gz|temporal-multi-edge"
  "twitter-2010|L|https://snap.stanford.edu/data/twitter-2010.txt.gz|super-node,scale"
  "com-Friendster|L|https://snap.stanford.edu/data/bigdata/communities/com-friendster.ungraph.txt.gz|dense-communities,scale"
  "GAP-road|L|https://sparse.tamu.edu/MM/GAP/GAP-road.tar.gz|high-diameter,scale"
)

sha256() { (sha256sum "$1" 2>/dev/null || shasum -a 256 "$1") | cut -d' ' -f1; }

entries=()
for row in "${DATASETS[@]}"; do
  IFS='|' read -r name tier url pathology <<<"$row"
  if [[ "$tier" == "L" && $large -eq 0 ]]; then continue; fi
  if [[ -n "$only" && ",$only," != *",$name,"* ]]; then continue; fi
  file="$dest/$name.${url##*/}"
  file="$dest/${url##*/}"
  if [[ -f "$file" ]]; then
    echo "have  $name  ($(du -h "$file" | cut -f1))"
  else
    echo "fetch $name  <- $url"
    curl -fL --retry 5 --retry-delay 5 -o "$file.part" "$url"
    mv "$file.part" "$file"
  fi
  # GNU stat first: on Linux `stat -f %z` succeeds and prints a filesystem
  # status block, which wrote an unparseable MANIFEST.json on 2026-09-10.
  bytes=$(stat -c %s "$file" 2>/dev/null || stat -f %z "$file")
  entries+=("{\"name\":\"$name\",\"tier\":\"$tier\",\"url\":\"$url\",\"file\":\"${url##*/}\",\"bytes\":$bytes,\"sha256\":\"$(sha256 "$file")\",\"pathology\":\"$pathology\"}")
done

manifest="$dest/MANIFEST.json"
{
  echo "{"
  echo "  \"schema\": \"adversarial-graph/datasets/v1\","
  echo "  \"fetched_at\": \"$(date -u +%Y-%m-%dT%H:%M:%SZ)\","
  echo "  \"datasets\": ["
  first=1
  for e in "${entries[@]}"; do
    if [[ $first -eq 0 ]]; then echo ","; fi
    printf '    %s' "$e"; first=0
  done
  echo
  echo "  ]"
  echo "}"
} > "$manifest.tmp"
python3 -c "import json,sys; json.load(open(sys.argv[1]))" "$manifest.tmp" || { echo "manifest would not parse; kept the old one, see $manifest.tmp" >&2; exit 1; }
mv "$manifest.tmp" "$manifest"
echo "wrote $manifest (${#entries[@]} datasets)"
