#!/usr/bin/env bash
# Step 10 tests for marker packs A/B/C (three-zip-upgrade-testing-plan.md).
# Uses fixture packs from pack-marker-fixtures.sh — does not run pack.sh.
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
# shellcheck source=pack-marker-fixtures.sh
source "$root/scripts/release/pack-marker-fixtures.sh"

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

pass() {
  echo "ok: $*"
}

tmpdir=$(mktemp -d "${TMPDIR:-/tmp}/rust-reg-test-marker-packs.XXXXXX")
cleanup() { rm -rf "$tmpdir"; }
trap cleanup EXIT

platform_tag=macos
export MARKER_PLATFORM=$platform_tag
out=$(build_all_marker_fixtures "$tmpdir/packs")

ver_a=${MARKER_VER_A:-9.9.1-alpha.160}
ver_b=${MARKER_VER_B:-9.9.1-alpha.161}
ver_c=${MARKER_VER_C:-9.9.1-alpha.162}

file_key_hash() {
  local sums=$1
  local name=$2
  awk -v n="$name" '$2 == n { print $1; exit }' "$sums"
}

# --- three_installers_exist ---
[[ -f "$out/A/rust-reg-${ver_a}.pkg" ]] || fail "missing A installer pkg"
for letter in B C; do
  case $letter in
    B) ver=$ver_b ;;
    C) ver=$ver_c ;;
  esac
  z="$out/$letter/zips"
  [[ -f "$z/rust-reg-${ver}-${platform_tag}-server.zip" ]] || fail "$letter missing server zip"
  [[ -f "$z/rust-reg-${ver}-${platform_tag}-wrapper.zip" ]] || fail "$letter missing wrapper zip"
  [[ -f "$z/rust-reg-${ver}-${platform_tag}-tray.zip" ]] || fail "$letter missing tray zip"
  [[ -f "$z/SHA256SUMS" ]] || fail "$letter missing SHA256SUMS"
  grep -q "rust-reg-${ver}-${platform_tag}-server.zip" "$z/SHA256SUMS" || fail "$letter SHA256SUMS missing server zip line"
  grep -q "rust-reg-${platform_tag}$" "$z/SHA256SUMS" || fail "$letter SHA256SUMS missing rust-reg file line"
done
pass three_installers_exist

# --- digests_differ_where_expected ---
# A also has zips (fixture builds them); compare file-key digests A→B→C.
sums_a="$out/A/zips/SHA256SUMS"
sums_b="$out/B/zips/SHA256SUMS"
sums_c="$out/C/zips/SHA256SUMS"
for key in \
  "rust-reg-${platform_tag}" \
  "rust-reg-run-${platform_tag}" \
  "rust-reg-desk-${platform_tag}"
do
  ha=$(file_key_hash "$sums_a" "$key")
  hb=$(file_key_hash "$sums_b" "$key")
  [[ -n "$ha" && -n "$hb" ]] || fail "missing file key $key in A or B sums"
  [[ "$ha" != "$hb" ]] || fail "A and B digests must differ for $key"
done
tray_b=$(file_key_hash "$sums_b" "rust-reg-desk-${platform_tag}")
tray_c=$(file_key_hash "$sums_c" "rust-reg-desk-${platform_tag}")
[[ "$tray_b" != "$tray_c" ]] || fail "C tray digest must differ from B"
pass digests_differ_where_expected

# --- b_zip_contains_markers ---
server_b="$out/B/zips/rust-reg-${ver_b}-${platform_tag}-server.zip"
tray_b_zip="$out/B/zips/rust-reg-${ver_b}-${platform_tag}-tray.zip"
python3 - "$server_b" "$tray_b_zip" <<'PY' || fail "B zip markers mismatch"
import sys, zipfile
server, tray = sys.argv[1:]
with zipfile.ZipFile(server) as z:
    html = z.read("form/testpath.html").decode()
    js = z.read("form/upgrade-probe.js").decode()
assert "Upgrade probe" in html, html
assert '<p class="probe-letter">B</p>' in html, html
assert 'export const UPGRADE_PROBE = "B";' in js, js
with zipfile.ZipFile(tray) as z:
    desk = z.read("rust-reg-desk")
text = desk.decode("utf-8", errors="replace")
assert "upgrade-probe-bake-B" in text, text
assert "Upgrade probe" in text, text
PY
pass b_zip_contains_markers

echo
echo "All step-10 marker-pack tests passed."
