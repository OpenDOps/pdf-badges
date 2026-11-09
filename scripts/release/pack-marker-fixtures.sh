#!/usr/bin/env bash
# Build fixture marker packs A/B/C for step 10 (no full pack.sh).
#
# Layout under <out_root> (default: target/marker-packs):
#   A/rust-reg-<verA>.pkg          — stand-in first installer
#   B/  three zips + SHA256SUMS    — probe B markers in form + binaries
#   C/  three zips + SHA256SUMS    — probe C; digests ≠ B
#
# Usage:
#   bash scripts/release/pack-marker-fixtures.sh [out_root]
#
# Sourced helpers also expose: build_marker_fixture_letter <out> <letter> <version> <platform>

set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
# shellcheck source=upgrade-zips.sh
source "$root/scripts/release/upgrade-zips.sh"
# shellcheck source=write-testpath.sh
source "$root/scripts/release/write-testpath.sh"

# Stamp searchable bake marks into fixture binaries (mirrors upgrade-probe-bake-{LETTER}).
stamp_binary() {
  local path=$1
  local role=$2
  local letter=$3
  local version=$4
  {
    printf 'fixture-%s-%s\n' "$role" "$version"
    printf 'upgrade-probe-bake-%s\n' "$letter"
    if [[ "$role" == desk && "$letter" == B ]]; then
      printf 'Upgrade probe\n'
    fi
  } >"$path"
  chmod 755 "$path" || true
}

# Write one letter's version tree + upgrade zips (+ optional A installer stand-in).
# Args: <letter_out_dir> <letter> <version> <platform_tag>
build_marker_fixture_letter() {
  local out=$1
  local letter=$2
  local version=$3
  local platform_tag=$4
  local version_dir="$out/version"
  local zips="$out/zips"
  mkdir -p "$version_dir/admin" "$version_dir/form" "$zips"

  stamp_binary "$version_dir/rust-reg" server "$letter" "$version"
  stamp_binary "$version_dir/rust-reg-run" wrapper "$letter" "$version"
  stamp_binary "$out/rust-reg-desk" desk "$letter" "$version"
  printf 'v%s\n' "$version" >"$version_dir/RELEASE"
  printf 'admin-%s\n' "$letter" >"$version_dir/admin/index.html"
  printf 'form-%s\n' "$letter" >"$version_dir/form/index.html"

  case "$letter" in
    A)
      # Shipping-like: no /testpath (probe off for first install).
      write_upgrade_probe_testpath "$version_dir/form" ""
      # Stand-in installer for three_installers_exist.
      printf 'pkg-fixture-A\n' >"$out/rust-reg-${version}.pkg"
      ;;
    B|C)
      write_upgrade_probe_testpath "$version_dir/form" "$letter"
      ;;
    *)
      echo "pack-marker-fixtures: letter must be A, B, or C (got $letter)" >&2
      return 1
      ;;
  esac

  write_upgrade_zips \
    "$zips" \
    "$version" \
    "$platform_tag" \
    "$version_dir" \
    rust-reg \
    rust-reg-run \
    "$out/rust-reg-desk" >"$zips/SHA256SUMS"
}

build_all_marker_fixtures() {
  local out_root=${1:-"$root/target/marker-packs"}
  local platform_tag=${MARKER_PLATFORM:-macos}
  rm -rf "$out_root"
  mkdir -p "$out_root"

  # Numeric alphas only (release.yml / parse_version reject 16a-style suffixes).
  local ver_a=${MARKER_VER_A:-9.9.1-alpha.160}
  local ver_b=${MARKER_VER_B:-9.9.1-alpha.161}
  local ver_c=${MARKER_VER_C:-9.9.1-alpha.162}

  build_marker_fixture_letter "$out_root/A" A "$ver_a" "$platform_tag"
  build_marker_fixture_letter "$out_root/B" B "$ver_b" "$platform_tag"
  build_marker_fixture_letter "$out_root/C" C "$ver_c" "$platform_tag"

  echo "marker fixtures: $out_root (platform=$platform_tag)" >&2
  echo "$out_root"
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  build_all_marker_fixtures "${1:-}"
fi
