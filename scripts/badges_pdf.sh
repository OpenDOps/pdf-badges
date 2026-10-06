#!/usr/bin/env bash
# Read a badge workbook with excel-tooling and write a PDF with pdf-tooling.
#
#   scripts/badges_pdf.sh [workbook.xlsx] [output.pdf]
#
# Defaults: ~/Downloads/бейджи_248.xlsx and target/badges.pdf
# Requires the excel-tooling and pdf-tooling images from this repo.

set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
workbook="${1:-$HOME/Downloads/бейджи_248.xlsx}"
output="${2:-$root/target/badges.pdf}"

if [[ ! -f "$workbook" ]]; then
  echo "workbook not found: $workbook" >&2
  exit 1
fi

mkdir -p "$(dirname "$output")"
cd "$root"
docker compose up -d --wait excel-tooling

excel_id="$(docker compose ps -q excel-tooling)"
network="$(docker inspect "$excel_id" --format '{{range $name, $_ := .NetworkSettings.Networks}}{{$name}}{{end}}')"

if ! docker compose up -d --wait pdf-tooling; then
  # The published host port is optional. The client calls pdf-tooling on the private network.
  docker rm -f rust-reg-pdf-tooling-badges >/dev/null 2>&1 || true
  docker run -d --name rust-reg-pdf-tooling-badges \
    --network "$network" \
    --network-alias pdf-tooling \
    pdf-tooling >/dev/null
  ready=0
  for _ in $(seq 1 30); do
    if docker exec rust-reg-pdf-tooling-badges /usr/local/bin/tcpcheck; then
      ready=1
      break
    fi
    sleep 1
  done
  if [[ "$ready" -ne 1 ]]; then
    echo "pdf-tooling did not accept connections" >&2
    exit 1
  fi
fi
workbook_dir="$(cd "$(dirname "$workbook")" && pwd)"
output_dir="$(cd "$(dirname "$output")" && pwd)"

docker run --rm \
  --network "$network" \
  --entrypoint /opt/venv/bin/python \
  -e PYTHONDONTWRITEBYTECODE=1 \
  -v "$root:/repo:ro" \
  -v "$workbook_dir:/in:ro" \
  -v "$output_dir:/out" \
  -v "$HOME/Library/Fonts:/fonts:ro" \
  excel-tooling \
  /repo/scripts/badges_pdf.py \
  --workbook "/in/$(basename "$workbook")" \
  --output "/out/$(basename "$output")" \
  --excel excel-tooling:50051 \
  --pdf pdf-tooling:50052 \
  --template /repo/scripts/badges/tadviser.yaml \
  --font-dir /fonts \
  --proto-root /repo/proto \
  --excel-gen /repo/modules/excel-tooling/excel_tooling/gen
