#!/usr/bin/env bash
set -euo pipefail

module_root="$(cd "$(dirname "$0")/.." && pwd)"
repo_root="$(cd "$module_root/../.." && pwd)"
out="$module_root/excel_tooling/gen"

mkdir -p "$out"
py="${PYTHON:-python3}"
"$py" -m grpc_tools.protoc \
  -I "$repo_root/proto" \
  --python_out="$out" \
  --grpc_python_out="$out" \
  "$repo_root/proto/irbis/table/v1/table.proto" \
  "$repo_root/proto/irbis/excel/v1/excel.proto"

while IFS= read -r dir; do
  touch "$dir/__init__.py"
done < <(find "$out" -type d)
