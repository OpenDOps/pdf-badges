#!/usr/bin/env bash
# Step 1 tests for three upgrade zips (three-zip-upgrade-testing-plan.md).
# Does not run a full pack.sh — uses fixture binaries under a temp dir.
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
# shellcheck source=upgrade-zips.sh
source "$root/scripts/release/upgrade-zips.sh"
# shellcheck source=write-testpath.sh
source "$root/scripts/release/write-testpath.sh"

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

pass() {
  echo "ok: $*"
}

tmpdir=$(mktemp -d "${TMPDIR:-/tmp}/rust-reg-test-upgrade-zips.XXXXXX")
cleanup() { rm -rf "$tmpdir"; }
trap cleanup EXIT

version=9.9.9-test
platform_tag=macos
version_dir="$tmpdir/version"
out_dir="$tmpdir/out"
mkdir -p "$version_dir/admin" "$version_dir/form" "$out_dir"

printf 'server-bytes\n' >"$version_dir/rust-reg"
printf 'wrapper-bytes\n' >"$version_dir/rust-reg-run"
printf 'v%s\n' "$version" >"$version_dir/RELEASE"
printf 'admin\n' >"$version_dir/admin/index.html"
printf 'form\n' >"$version_dir/form/index.html"
# Must not appear in the server zip.
printf 'should-not-be-in-server\n' >"$version_dir/rust-reg-desk"
desk="$tmpdir/rust-reg-desk"
printf 'tray-bytes\n' >"$desk"
chmod 755 "$version_dir/rust-reg" "$version_dir/rust-reg-run" "$desk"

sums=$(write_upgrade_zips \
  "$out_dir" \
  "$version" \
  "$platform_tag" \
  "$version_dir" \
  rust-reg \
  rust-reg-run \
  "$desk")

server_zip="$out_dir/rust-reg-${version}-${platform_tag}-server.zip"
wrapper_zip="$out_dir/rust-reg-${version}-${platform_tag}-wrapper.zip"
tray_zip="$out_dir/rust-reg-${version}-${platform_tag}-tray.zip"
legacy_zip="$out_dir/rust-reg-${version}-macos.zip"

# --- pack_writes_three_zips ---
[[ -f "$server_zip" ]] || fail "missing server zip"
[[ -f "$wrapper_zip" ]] || fail "missing wrapper zip"
[[ -f "$tray_zip" ]] || fail "missing tray zip"
[[ ! -f "$legacy_zip" ]] || fail "legacy single macos.zip must not exist"
pass pack_writes_three_zips

# --- server_zip_layout ---
server_list=$(python3 - "$server_zip" <<'PY'
import sys, zipfile
with zipfile.ZipFile(sys.argv[1]) as z:
    print("\n".join(sorted(z.namelist())))
PY
)
echo "$server_list" | grep -qx 'rust-reg' || fail "server zip missing rust-reg"
echo "$server_list" | grep -qx 'RELEASE' || fail "server zip missing RELEASE"
echo "$server_list" | grep -qx 'admin/index.html' || fail "server zip missing admin/"
echo "$server_list" | grep -qx 'form/index.html' || fail "server zip missing form/"
echo "$server_list" | grep -q 'rust-reg-run' && fail "server zip must not contain rust-reg-run"
echo "$server_list" | grep -q 'rust-reg-desk' && fail "server zip must not contain rust-reg-desk"
echo "$server_list" | grep -q 'form/testpath.html' && fail "shipping fixture must omit form/testpath.html"
echo "$server_list" | grep -q 'form/upgrade-probe.js' && fail "shipping fixture must omit form/upgrade-probe.js"
pass server_zip_layout

# --- wrapper_and_tray_are_single_files ---
wrapper_list=$(python3 - "$wrapper_zip" <<'PY'
import sys, zipfile
with zipfile.ZipFile(sys.argv[1]) as z:
    print("\n".join(sorted(z.namelist())))
PY
)
tray_list=$(python3 - "$tray_zip" <<'PY'
import sys, zipfile
with zipfile.ZipFile(sys.argv[1]) as z:
    print("\n".join(sorted(z.namelist())))
PY
)
[[ "$wrapper_list" == "rust-reg-run" ]] || fail "wrapper zip members: [$wrapper_list]"
[[ "$tray_list" == "rust-reg-desk" ]] || fail "tray zip members: [$tray_list]"
pass wrapper_and_tray_are_single_files

# --- sha256sums_lists_zips_and_files ---
echo "$sums" >"$tmpdir/SHA256SUMS"
for name in \
  "rust-reg-${version}-${platform_tag}-server.zip" \
  "rust-reg-${version}-${platform_tag}-wrapper.zip" \
  "rust-reg-${version}-${platform_tag}-tray.zip" \
  "rust-reg-${platform_tag}" \
  "rust-reg-run-${platform_tag}" \
  "rust-reg-desk-${platform_tag}"
do
  echo "$sums" | grep -E "  ${name}\$" >/dev/null || fail "SHA256SUMS missing line for $name"
done
# Verify digests: zip lines against files; file lines against source binaries.
(
  cd "$out_dir"
  if command -v shasum >/dev/null 2>&1; then
    echo "$sums" | grep '\-server\.zip$' | shasum -a 256 -c - >/dev/null
    echo "$sums" | grep '\-wrapper\.zip$' | shasum -a 256 -c - >/dev/null
    echo "$sums" | grep '\-tray\.zip$' | shasum -a 256 -c - >/dev/null
  else
    echo "$sums" | grep '\-server\.zip$' | sha256sum -c - >/dev/null
    echo "$sums" | grep '\-wrapper\.zip$' | sha256sum -c - >/dev/null
    echo "$sums" | grep '\-tray\.zip$' | sha256sum -c - >/dev/null
  fi
)
# File-key lines name logical keys; check digests match the binaries.
server_hash=$(echo "$sums" | awk -v n="rust-reg-${platform_tag}" '$2==n{print $1}')
run_hash=$(echo "$sums" | awk -v n="rust-reg-run-${platform_tag}" '$2==n{print $1}')
desk_hash=$(echo "$sums" | awk -v n="rust-reg-desk-${platform_tag}" '$2==n{print $1}')
if command -v shasum >/dev/null 2>&1; then
  [[ "$server_hash" == "$(shasum -a 256 "$version_dir/rust-reg" | awk '{print $1}')" ]] || fail "rust-reg file line mismatch"
  [[ "$run_hash" == "$(shasum -a 256 "$version_dir/rust-reg-run" | awk '{print $1}')" ]] || fail "run file line mismatch"
  [[ "$desk_hash" == "$(shasum -a 256 "$desk" | awk '{print $1}')" ]] || fail "desk file line mismatch"
else
  [[ "$server_hash" == "$(sha256sum "$version_dir/rust-reg" | awk '{print $1}')" ]] || fail "rust-reg file line mismatch"
  [[ "$run_hash" == "$(sha256sum "$version_dir/rust-reg-run" | awk '{print $1}')" ]] || fail "run file line mismatch"
  [[ "$desk_hash" == "$(sha256sum "$desk" | awk '{print $1}')" ]] || fail "desk file line mismatch"
fi
pass sha256sums_lists_zips_and_files

# --- skip tray when desk path empty ---
out2="$tmpdir/out2"
mkdir -p "$out2"
sums2=$(write_upgrade_zips \
  "$out2" \
  "$version" \
  linux \
  "$version_dir" \
  rust-reg \
  rust-reg-run \
  "")
[[ -f "$out2/rust-reg-${version}-linux-server.zip" ]] || fail "linux server zip missing"
[[ -f "$out2/rust-reg-${version}-linux-wrapper.zip" ]] || fail "linux wrapper zip missing"
[[ ! -f "$out2/rust-reg-${version}-linux-tray.zip" ]] || fail "tray zip must be absent without desk"
echo "$sums2" | grep -q 'rust-reg-desk-linux' && fail "desk file line must be absent without desk"
pass skip_tray_without_desk

# --- in_server_zip (step 9: probe B packs /testpath into form/) ---
write_upgrade_probe_testpath "$version_dir/form" B
out_probe="$tmpdir/out-probe"
mkdir -p "$out_probe"
write_upgrade_zips \
  "$out_probe" \
  "$version" \
  "$platform_tag" \
  "$version_dir" \
  rust-reg \
  rust-reg-run \
  "$desk" >/dev/null
probe_server="$out_probe/rust-reg-${version}-${platform_tag}-server.zip"
[[ -f "$probe_server" ]] || fail "probe server zip missing"
probe_list=$(python3 - "$probe_server" <<'PY'
import sys, zipfile
with zipfile.ZipFile(sys.argv[1]) as z:
    print("\n".join(sorted(z.namelist())))
PY
)
echo "$probe_list" | grep -qx 'form/testpath.html' || fail "probe server zip missing form/testpath.html"
echo "$probe_list" | grep -qx 'form/upgrade-probe.js' || fail "probe server zip missing form/upgrade-probe.js"
python3 - "$probe_server" <<'PY' || fail "probe testpath body mismatch"
import sys, zipfile
with zipfile.ZipFile(sys.argv[1]) as z:
    html = z.read("form/testpath.html").decode()
    js = z.read("form/upgrade-probe.js").decode()
assert "Upgrade probe" in html, html
assert '<p class="probe-letter">B</p>' in html, html
assert 'export const UPGRADE_PROBE = "B";' in js, js
PY
pass in_server_zip

echo
echo "All step-1 upgrade-zip tests passed."
