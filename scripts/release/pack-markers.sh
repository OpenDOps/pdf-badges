#!/usr/bin/env bash
# Documented entry for step-10 real marker packs (Mac/Linux proof assets).
#
# Fixture packs (CI / local asserts without a triple cargo pack):
#   bash scripts/release/pack-marker-fixtures.sh [out_root]
#   bash scripts/release/test-marker-packs.sh
#
# Real packs (one letter per run; bake probe into binaries + form /testpath):
#   1. Bump Cargo.toml version to a numeric prerelease (e.g. 2.0.0-alpha.160).
#      Do not use suffixes like alpha.16a — release.yml / parse_version reject them.
#   2. Export matching tag + probe letter, then pack:
#
#        # A — first install (shipping tray / no bake, or letter A)
#        unset RUST_REG_UPGRADE_PROBE
#        REGISTRATION_RELEASE_TAG=v2.0.0-alpha.160 bash scripts/release/pack.sh
#
#        # B — all three zips differ; tray menu + /testpath = B
#        RUST_REG_UPGRADE_PROBE=B \
#          REGISTRATION_RELEASE_TAG=v2.0.0-alpha.161 bash scripts/release/pack.sh
#
#        # C — probe C; no Upgrade probe menu; icon ≠ B; /testpath = C
#        RUST_REG_UPGRADE_PROBE=C \
#          REGISTRATION_RELEASE_TAG=v2.0.0-alpha.162 bash scripts/release/pack.sh
#
#   3. Publish as GitHub prereleases (or host SHA256SUMS + zips). Clients need
#      REGISTRATION_UPDATE_PRERELEASE=1 to see them.
#
# This script only builds fixtures unless MARKER_REAL=1 (then runs pack.sh once
# for the current Cargo version + RUST_REG_UPGRADE_PROBE).

set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)

if [[ "${MARKER_REAL:-}" == "1" ]]; then
  echo "pack-markers: real pack (RUST_REG_UPGRADE_PROBE=${RUST_REG_UPGRADE_PROBE:-unset})" >&2
  exec bash "$root/scripts/release/pack.sh"
fi

# shellcheck source=pack-marker-fixtures.sh
source "$root/scripts/release/pack-marker-fixtures.sh"
build_all_marker_fixtures "${1:-"$root/target/marker-packs"}"
