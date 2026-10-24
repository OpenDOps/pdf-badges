#!/usr/bin/env bash
# Build this machine's installer and its upgrade zip, and print their SHA256SUMS lines.
# REGISTRATION_RELEASE_TAG, when unset, becomes v plus the Cargo.toml version.
# The desk tray is packed with the platform installer. REGISTRATION_SKIP_DESK=1
# leaves that wrapper out and ships the standalone server.
# The upgrade zip is the version directory: rust-reg, admin/, form/, and RELEASE.
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
mac_cpu=""
case "$uname_s" in
  Darwin) os=macos ;;
  Linux) os=linux ;;
  MINGW*|MSYS*|CYGWIN*) os=windows ;;
  *)
    echo "pack: unsupported os $uname_s" >&2
    exit 1
    ;;
esac

toolchain=$(bash "$root/scripts/rust-host.sh")
case "$toolchain" in
  stable-aarch64-apple-darwin) mac_cpu=arm64 ;;
  stable-x86_64-apple-darwin) mac_cpu=intel ;;
esac

echo "pack: cargo +${toolchain} build --release $tag" >&2
cargo "+${toolchain}" build --release --bin rust-reg --bin rust-reg-run >&2
with_desk=1
if [[ "${REGISTRATION_SKIP_DESK:-}" == "1" ]]; then
  with_desk=0
  echo "pack: standalone release, skipping the desk wrapper" >&2
else
  echo "pack: cargo +${toolchain} build desk wrapper" >&2
  cargo "+${toolchain}" build --release --manifest-path "$root/desk/Cargo.toml" --target-dir "$root/target" >&2
fi

release_dir="$root/target/release"
if [[ "$os" == windows ]]; then
  bin_name=rust-reg.exe
  run_name=rust-reg-run.exe
  desk_name=rust-reg-desk.exe
else
  bin_name=rust-reg
  run_name=rust-reg-run
  desk_name=rust-reg-desk
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

admin_dist="$root/registration-admin/dist"
form_client="$root/registration-form/build/client"
if [[ "${REGISTRATION_SKIP_FRONT:-}" == "1" ]]; then
  echo "pack: using the prebuilt admin and form trees" >&2
  if [[ ! -f "$admin_dist/index.html" || ! -f "$form_client/index.html" ]]; then
    echo "pack: prebuilt admin/dist or form/build/client is missing index.html" >&2
    exit 1
  fi
else
  build_front "$root/registration-admin"
  build_front "$root/registration-form"
fi

work=$(mktemp -d "${TMPDIR:-/tmp}/rust-reg-pack.XXXXXX")
cleanup() { rm -rf "$work"; }
trap cleanup EXIT

assemble_version() {
  local dest=$1
  mkdir -p "$dest/admin" "$dest/form"
  cp "$release_dir/$bin_name" "$dest/$bin_name"
  chmod 755 "$dest/$bin_name" || true
  cp -R "$admin_dist/." "$dest/admin/"
  cp -R "$form_client/." "$dest/form/"
  # The client build copies public/dbenums next to the form. That dump is not part of the release.
  rm -rf "$dest/form/dbenums"
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
    if [[ "$with_desk" == 1 ]]; then
      cp "$release_dir/$desk_name" "$prefix/bin/$desk_name"
      chmod 755 "$prefix/bin/$desk_name"
      app="$work/root/Applications/reg-server.app/Contents"
      mkdir -p "$app/MacOS"
      cp "$release_dir/$desk_name" "$app/MacOS/reg-server"
      chmod 755 "$app/MacOS/reg-server"
      cat > "$app/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key>
  <string>reg-server</string>
  <key>CFBundleDisplayName</key>
  <string>reg-server</string>
  <key>CFBundleIdentifier</key>
  <string>com.opendops.rust-reg.desk</string>
  <key>CFBundleExecutable</key>
  <string>reg-server</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleVersion</key>
  <string>$version</string>
  <key>CFBundleShortVersionString</key>
  <string>$version</string>
  <key>LSUIElement</key>
  <true/>
  <key>LSMinimumSystemVersion</key>
  <string>11.0</string>
</dict>
</plist>
EOF
    fi
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
    if [[ "$mac_cpu" == intel ]]; then
      asset="$release_dir/rust-reg-${version}-intel.pkg"
    else
      asset="$release_dir/rust-reg-${version}.pkg"
    fi
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
    if [[ "$with_desk" == 1 ]]; then
      cp "$release_dir/$desk_name" "$deb/opt/rust-reg/bin/$desk_name"
      chmod 755 "$deb/opt/rust-reg/bin/$desk_name"
      mkdir -p "$deb/usr/share/applications" "$deb/usr/share/icons/hicolor/scalable/apps"
      cat > "$deb/usr/share/applications/rust-reg.desktop" <<'EOF'
[Desktop Entry]
Type=Application
Name=reg-server
Comment=Registration server
Exec=/opt/rust-reg/bin/rust-reg-desk
Icon=rust-reg
Terminal=false
Categories=Utility;
StartupNotify=false
EOF
    cat > "$deb/usr/share/icons/hicolor/scalable/apps/rust-reg.svg" <<'EOF'
<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64" viewBox="0 0 64 64">
  <rect width="64" height="64" rx="12" fill="#1f6feb"/>
  <rect x="18" y="18" width="28" height="28" rx="4" fill="#ffffff"/>
</svg>
EOF
    fi
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
    if [[ "$with_desk" == 1 ]]; then
      cat > "$deb/DEBIAN/postinst" <<'EOF'
#!/bin/sh
set -e
if command -v systemctl >/dev/null 2>&1; then
  systemctl daemon-reload
  systemctl enable rust-reg.service
fi
if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database /usr/share/applications || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
  gtk-update-icon-cache -f /usr/share/icons/hicolor || true
fi
EOF
    else
      cat > "$deb/DEBIAN/postinst" <<'EOF'
#!/bin/sh
set -e
if command -v systemctl >/dev/null 2>&1; then
  systemctl daemon-reload
  systemctl enable rust-reg.service
fi
EOF
    fi
    chmod 755 "$deb/DEBIAN/postinst"
    asset="$release_dir/rust-reg_${version}_amd64.deb"
    dpkg-deb --root-owner-group --build "$deb" "$asset" >&2
    ;;
  windows)
    tree_version="$work/tree/versions/$version"
    version_dir="$tree_version"
    mkdir -p "$work/tree/bin" "$tree_version"
    assemble_version "$tree_version"
    cp "$release_dir/$run_name" "$work/tree/bin/$run_name"
    if [[ "$with_desk" == 1 ]]; then
      cp "$release_dir/$desk_name" "$work/tree/bin/$desk_name"
    fi
    tar -C "$work" -cf "$work/payload.tar" tree
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

if [[ "$os" == macos && "$mac_cpu" == intel ]]; then
  upgrade_zip="$release_dir/rust-reg-${version}-macos-intel.zip"
else
  upgrade_zip="$release_dir/rust-reg-${version}-${os}.zip"
fi
py=python3
if ! command -v python3 >/dev/null 2>&1; then
  py=python
fi
"$py" - "$version_dir" "$upgrade_zip" <<'PY'
import pathlib, sys, zipfile
root, dest = sys.argv[1:]
root = pathlib.Path(root)
with zipfile.ZipFile(dest, "w", compression=zipfile.ZIP_DEFLATED) as archive:
    for path in sorted(root.rglob("*")):
        if path.is_file():
            archive.write(path, path.relative_to(root).as_posix())
PY

print_sum() {
  local file=$1
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
  printf '%s  %s\n' "$hash" "$(basename "$file")"
}

print_sum "$asset"
print_sum "$upgrade_zip"
