#!/usr/bin/env bash
# Write the three step-16 upgrade zips and print SHA256SUMS lines for the zips
# and for the platform-qualified file digests.
#
# Usage:
#   write_upgrade_zips <out_dir> <version> <platform_tag> <version_dir> \
#     <bin_name> <run_name> <desk_path_or_empty>
#
# platform_tag: linux | linux-arm64 | macos | macos-intel | windows
# desk_path_or_empty: path to rust-reg-desk binary; empty skips the tray zip.
#
# Prints lines: "<hex>  <name>" for each zip and each file key.
# Sourced by pack.sh, or invoked as:
#   bash scripts/release/upgrade-zips.sh write ...

set -euo pipefail

print_sum_as() {
  local file=$1
  local name=$2
  local hash
  if command -v shasum >/dev/null 2>&1; then
    hash=$(shasum -a 256 "$file" | awk '{print $1}')
  elif command -v sha256sum >/dev/null 2>&1; then
    hash=$(sha256sum "$file" | awk '{print $1}')
  else
    local py=python3
    if ! command -v python3 >/dev/null 2>&1; then
      py=python
    fi
    hash=$("$py" - "$file" <<'PY'
import hashlib, pathlib, sys
print(hashlib.sha256(pathlib.Path(sys.argv[1]).read_bytes()).hexdigest())
PY
)
  fi
  printf '%s  %s\n' "$hash" "$name"
}

zip_deflated() {
  local src_dir=$1
  local dest=$2
  local py=python3
  if ! command -v python3 >/dev/null 2>&1; then
    py=python
  fi
  "$py" - "$src_dir" "$dest" <<'PY'
import pathlib, sys, zipfile
root, dest = sys.argv[1:]
root = pathlib.Path(root)
with zipfile.ZipFile(dest, "w", compression=zipfile.ZIP_DEFLATED) as archive:
    for path in sorted(root.rglob("*")):
        if path.is_file():
            archive.write(path, path.relative_to(root).as_posix())
PY
}

# Write one-file zip whose archive member is basename(file).
zip_one_file() {
  local file=$1
  local dest=$2
  local member
  member=$(basename "$file")
  local stage
  stage=$(mktemp -d "${TMPDIR:-/tmp}/rust-reg-onezip.XXXXXX")
  cp "$file" "$stage/$member"
  chmod 755 "$stage/$member" || true
  zip_deflated "$stage" "$dest"
  rm -rf "$stage"
}

write_upgrade_zips() {
  local out_dir=$1
  local version=$2
  local platform_tag=$3
  local version_dir=$4
  local bin_name=$5
  local run_name=$6
  local desk_path=${7:-}

  mkdir -p "$out_dir"

  local server_zip="$out_dir/rust-reg-${version}-${platform_tag}-server.zip"
  local wrapper_zip="$out_dir/rust-reg-${version}-${platform_tag}-wrapper.zip"
  local tray_zip="$out_dir/rust-reg-${version}-${platform_tag}-tray.zip"

  local server_stage
  server_stage=$(mktemp -d "${TMPDIR:-/tmp}/rust-reg-server-zip.XXXXXX")
  # Server zip: version tree without the wrapper (and without a tray if present).
  local path
  for path in "$version_dir"/*; do
    local base
    base=$(basename "$path")
    if [[ "$base" == "$run_name" || "$base" == "rust-reg-desk" || "$base" == "rust-reg-desk.exe" ]]; then
      continue
    fi
    if [[ -d "$path" ]]; then
      cp -R "$path" "$server_stage/$base"
    else
      cp "$path" "$server_stage/$base"
    fi
  done
  if [[ ! -f "$server_stage/$bin_name" ]]; then
    echo "upgrade-zips: missing $bin_name in $version_dir" >&2
    rm -rf "$server_stage"
    return 1
  fi
  if [[ ! -f "$server_stage/RELEASE" || ! -d "$server_stage/admin" || ! -d "$server_stage/form" ]]; then
    echo "upgrade-zips: server tree needs RELEASE, admin/, form/" >&2
    rm -rf "$server_stage"
    return 1
  fi
  zip_deflated "$server_stage" "$server_zip"
  rm -rf "$server_stage"

  if [[ ! -f "$version_dir/$run_name" ]]; then
    echo "upgrade-zips: missing $run_name in $version_dir" >&2
    return 1
  fi
  zip_one_file "$version_dir/$run_name" "$wrapper_zip"

  print_sum_as "$server_zip" "$(basename "$server_zip")"
  print_sum_as "$wrapper_zip" "$(basename "$wrapper_zip")"
  print_sum_as "$version_dir/$bin_name" "rust-reg-${platform_tag}"
  print_sum_as "$version_dir/$run_name" "rust-reg-run-${platform_tag}"

  if [[ -n "$desk_path" ]]; then
    if [[ ! -f "$desk_path" ]]; then
      echo "upgrade-zips: desk binary missing: $desk_path" >&2
      return 1
    fi
    zip_one_file "$desk_path" "$tray_zip"
    print_sum_as "$tray_zip" "$(basename "$tray_zip")"
    print_sum_as "$desk_path" "rust-reg-desk-${platform_tag}"
  fi
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  if [[ "${1:-}" != "write" ]]; then
    echo "usage: $0 write <out_dir> <version> <platform_tag> <version_dir> <bin_name> <run_name> [desk_path]" >&2
    exit 1
  fi
  shift
  write_upgrade_zips "$@"
fi
