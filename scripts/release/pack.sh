#!/usr/bin/env bash
# Build this machine's release asset and print its SHA256SUMS line.
# REGISTRATION_RELEASE_TAG, when unset, becomes v plus the Cargo.toml version.
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"

cargo_version=$(awk '
  /^\[package\]/ { in_pkg = 1; next }
  /^\[/ { in_pkg = 0 }
  in_pkg && /^version = / {
    gsub(/"/, "", $3)
    print $3
    exit
  }
' Cargo.toml)
if [[ -z "$cargo_version" ]]; then
  echo "pack: Cargo.toml has no package version" >&2
  exit 1
fi

if [[ -z "${REGISTRATION_RELEASE_TAG:-}" ]]; then
  REGISTRATION_RELEASE_TAG="v${cargo_version}"
fi
export REGISTRATION_RELEASE_TAG
if [[ "$REGISTRATION_RELEASE_TAG" != "v${cargo_version}" ]]; then
  echo "pack: REGISTRATION_RELEASE_TAG must be v${cargo_version}, got ${REGISTRATION_RELEASE_TAG}" >&2
  exit 1
fi
version="${REGISTRATION_RELEASE_TAG#v}"
tag="$REGISTRATION_RELEASE_TAG"

uname_s=$(uname -s)
case "$uname_s" in
  Darwin) os=macos ;;
  Linux) os=linux ;;
  MINGW*|MSYS*|CYGWIN*) os=windows ;;
  *)
    echo "pack: unsupported os $uname_s" >&2
    exit 1
    ;;
esac

echo "pack: cargo build --release $tag" >&2
cargo build --release --bin rust-reg --bin rust-reg-run >&2

release_dir="$root/target/release"
if [[ "$os" == windows ]]; then
  bin_name=rust-reg.exe
  run_name=rust-reg-run.exe
else
  bin_name=rust-reg
  run_name=rust-reg-run
fi

build_front() {
  local dir=$1
  if [[ ! -d "$dir/node_modules" ]]; then
    echo "pack: npm ci in $dir" >&2
    (cd "$dir" && npm ci >&2)
  fi
  echo "pack: npm run build in $dir" >&2
  (cd "$dir" && npm run build >&2)
}

build_front "$root/registration-admin"
build_front "$root/registration-form"

work=$(mktemp -d "${TMPDIR:-/tmp}/rust-reg-pack.XXXXXX")
cleanup() { rm -rf "$work"; }
trap cleanup EXIT

assemble_version() {
  local dest=$1
  mkdir -p "$dest/admin" "$dest/form"
  cp "$release_dir/$bin_name" "$dest/$bin_name"
  chmod 755 "$dest/$bin_name" || true
  cp -R "$root/registration-admin/dist/." "$dest/admin/"
  cp -R "$root/registration-form/build/." "$dest/form/"
  printf '%s\n' "$tag" > "$dest/RELEASE"
}

case "$os" in
  macos)
    prefix="$work/root/Library/rust-reg"
    version_dir="$prefix/versions/$version"
    mkdir -p "$prefix/bin" "$(dirname "$work/root/Library/LaunchDaemons")"
    mkdir -p "$work/root/Library/LaunchDaemons"
    cp "$release_dir/$run_name" "$prefix/bin/$run_name"
    chmod 755 "$prefix/bin/$run_name"
    assemble_version "$version_dir"
    cat > "$work/root/Library/LaunchDaemons/com.opendops.rust-reg.plist" <<'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>com.opendops.rust-reg</string>
  <key>ProgramArguments</key>
  <array>
    <string>/Library/rust-reg/bin/rust-reg-run</string>
    <string>--prefix</string>
    <string>/Library/rust-reg</string>
  </array>
  <key>KeepAlive</key>
  <true/>
  <key>RunAtLoad</key>
  <true/>
</dict>
</plist>
EOF
    asset="$release_dir/rust-reg-${version}.pkg"
    pkgbuild \
      --root "$work/root" \
      --identifier com.opendops.rust-reg \
      --version "$version" \
      --install-location / \
      "$asset" >&2
    ;;
  linux)
    deb="$work/deb"
    version_dir="$deb/opt/rust-reg/versions/$version"
    mkdir -p "$deb/opt/rust-reg/bin" "$deb/lib/systemd/system" "$deb/DEBIAN"
    cp "$release_dir/$run_name" "$deb/opt/rust-reg/bin/$run_name"
    chmod 755 "$deb/opt/rust-reg/bin/$run_name"
    assemble_version "$version_dir"
    cat > "$deb/lib/systemd/system/rust-reg.service" <<'EOF'
[Unit]
Description=rust-reg registration server

[Service]
Type=simple
ExecStart=/opt/rust-reg/bin/rust-reg-run --prefix /opt/rust-reg
Restart=always

[Install]
WantedBy=multi-user.target
EOF
    cat > "$deb/DEBIAN/control" <<EOF
Package: rust-reg
Version: $version
Section: utils
Priority: optional
Architecture: amd64
Maintainer: OpenDOps <opendops@users.noreply.github.com>
Description: Registration server
EOF
    cat > "$deb/DEBIAN/postinst" <<'EOF'
#!/bin/sh
set -e
if command -v systemctl >/dev/null 2>&1; then
  systemctl daemon-reload
  systemctl enable rust-reg.service
fi
EOF
    chmod 755 "$deb/DEBIAN/postinst"
    asset="$release_dir/rust-reg_${version}_amd64.deb"
    dpkg-deb --root-owner-group --build "$deb" "$asset" >&2
    ;;
  windows)
    stage="$work/stage"
    tree_version="$work/tree/versions/$version"
    mkdir -p "$stage" "$work/tree/bin" "$tree_version"
    assemble_version "$stage"
    assemble_version "$tree_version"
    cp "$release_dir/$run_name" "$work/tree/bin/$run_name"
    tar -C "$work" -cf "$work/payload.tar" stage tree
    asset="$release_dir/rust-reg-${version}.exe"
    cp "$release_dir/$bin_name" "$asset"
    py=python3
    if ! command -v python3 >/dev/null 2>&1; then
      py=python
    fi
    "$py" - "$asset" "$work/payload.tar" <<'PY'
import pathlib, sys
exe, tar = sys.argv[1:]
blob = pathlib.Path(tar).read_bytes()
with pathlib.Path(exe).open("ab") as out:
    out.write(blob)
    out.write(len(blob).to_bytes(8, "little"))
    out.write(b"RUSTREG1")
PY
    ;;
esac

if command -v shasum >/dev/null 2>&1; then
  hash=$(shasum -a 256 "$asset" | awk '{print $1}')
elif command -v sha256sum >/dev/null 2>&1; then
  hash=$(sha256sum "$asset" | awk '{print $1}')
else
  py=python3
  if ! command -v python3 >/dev/null 2>&1; then
    py=python
  fi
  hash=$("$py" - "$asset" <<'PY'
import hashlib, pathlib, sys
print(hashlib.sha256(pathlib.Path(sys.argv[1]).read_bytes()).hexdigest())
PY
)
fi
printf '%s  %s\n' "$hash" "$(basename "$asset")"
