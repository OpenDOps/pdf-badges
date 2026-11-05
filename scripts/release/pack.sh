#!/usr/bin/env bash
# Build this machine's installer and three upgrade zips, and print SHA256SUMS lines.
# REGISTRATION_RELEASE_TAG, when unset, becomes v plus the Cargo.toml version.
# The desk tray is packed with the platform installer. REGISTRATION_SKIP_DESK=1
# leaves that wrapper out and ships the standalone server (no tray zip).
# Upgrade zips (step 16): server (rust-reg, admin/, form/, RELEASE), wrapper
# (rust-reg-run), tray (rust-reg-desk). SHA256SUMS lists each zip and each
# platform-qualified file digest (rust-reg-<platform>, …).
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"
# shellcheck source=upgrade-zips.sh
source "$root/scripts/release/upgrade-zips.sh"

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

# Debian arch for the .deb (native host). CI linux stays amd64; UTM arm64 guests get arm64.
deb_arch=""
if [[ "$os" == linux ]]; then
  case "$(uname -m)" in
    x86_64|amd64) deb_arch=amd64 ;;
    aarch64|arm64) deb_arch=arm64 ;;
    *)
      echo "pack: unsupported linux arch $(uname -m)" >&2
      exit 1
      ;;
  esac
  echo "pack: debian Architecture ${deb_arch}" >&2
fi

case "$os" in
  macos|linux) native_print_backend=cups ;;
  windows) native_print_backend=spooler ;;
esac
echo "pack: native print backend ${native_print_backend}" >&2
echo "pack: cargo +${toolchain} build --release $tag" >&2
cargo "+${toolchain}" build --release --lib --bin rust-reg --bin rust-reg-run >&2
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
  cp "$release_dir/$run_name" "$dest/$run_name"
  chmod 755 "$dest/$run_name" || true
  cp -R "$admin_dist/." "$dest/admin/"
  cp -R "$form_client/." "$dest/form/"
  # The client build copies public/dbenums next to the form. That dump is not part of the release.
  rm -rf "$dest/form/dbenums"
  printf '%s\n' "$tag" > "$dest/RELEASE"
}

# Finder icon for reg-server.app. The picture is the menu-bar pencil and person.
install_app_icon() {
  local bin=$1 dest=$2
  local png iconset
  png=$(mktemp "${TMPDIR:-/tmp}/reg-server-icon.XXXXXX")
  mv "$png" "$png.png"
  png="$png.png"
  iconset=$(mktemp -d "${TMPDIR:-/tmp}/reg-server-icon.XXXXXX")
  mv "$iconset" "${iconset}.iconset"
  iconset="${iconset}.iconset"
  "$bin" --write-icon "$png"
  mkdir -p "$dest"
  sips -z 16 16 "$png" --out "$iconset/icon_16x16.png" >/dev/null
  sips -z 32 32 "$png" --out "$iconset/icon_16x16@2x.png" >/dev/null
  sips -z 32 32 "$png" --out "$iconset/icon_32x32.png" >/dev/null
  sips -z 64 64 "$png" --out "$iconset/icon_32x32@2x.png" >/dev/null
  sips -z 128 128 "$png" --out "$iconset/icon_128x128.png" >/dev/null
  sips -z 256 256 "$png" --out "$iconset/icon_128x128@2x.png" >/dev/null
  sips -z 256 256 "$png" --out "$iconset/icon_256x256.png" >/dev/null
  sips -z 512 512 "$png" --out "$iconset/icon_256x256@2x.png" >/dev/null
  sips -z 512 512 "$png" --out "$iconset/icon_512x512.png" >/dev/null
  sips -z 1024 1024 "$png" --out "$iconset/icon_512x512@2x.png" >/dev/null
  iconutil -c icns "$iconset" -o "$dest/AppIcon.icns"
  rm -rf "$png" "$iconset"
}

# Disk image whose window is the app beside the Applications folder.
# Dragging the app across is the install. The app carries the wrapper and the
# version tree; the first open copies them to ~/Library/Application Support/rust-reg.
write_drag_dmg() {
  local app_src=$1 dmg=$2 version=$3 version_dir=$4 run_name=$5
  local stage="$work/dmg-stage"
  local prefix rw mount
  rm -rf "$stage"
  mkdir -p "$stage"
  cp -R "$app_src" "$stage/reg-server.app"
  prefix="$stage/reg-server.app/Contents/prefix"
  mkdir -p "$prefix/bin" "$prefix/updates" "$prefix/versions"
  cp -R "$version_dir" "$prefix/versions/$(basename "$version_dir")"
  cp "$version_dir/$run_name" "$prefix/bin/$run_name"
  chmod 755 "$prefix/bin/$run_name"
  printf '%s\n' "$version" > "$prefix/updates/live"
  ln -s /Applications "$stage/Applications"
  codesign --force --deep --sign - "$stage/reg-server.app"
  rw="$work/reg-server-rw.dmg"
  rm -f "$rw" "$dmg"
  # CI runners sometimes leave a previous "reg-server" volume attached.
  while diskutil list 2>/dev/null | grep -q 'reg-server'; do
    hdiutil detach "/Volumes/reg-server" -force 2>/dev/null || true
    sleep 1
  done
  local create_ok=0
  local attempt
  for attempt in 1 2 3 4 5; do
    if hdiutil create -volname "reg-server" -srcfolder "$stage" -ov -format UDRW "$rw"; then
      create_ok=1
      break
    fi
    echo "pack: hdiutil create attempt $attempt failed; retrying" >&2
    sleep 2
  done
  if [[ "$create_ok" -ne 1 ]]; then
    echo "pack: hdiutil create failed after retries" >&2
    exit 1
  fi
  mount=$(hdiutil attach "$rw" -readwrite -noverify -noautoopen | awk 'END {print $NF}')
  osascript <<'EOF' || true
tell application "Finder"
  tell disk "reg-server"
    open
    set current view of container window to icon view
    set toolbar visible of container window to false
    set statusbar visible of container window to false
    set the bounds of container window to {120, 120, 660, 420}
    set viewOptions to the icon view options of container window
    set arrangement of viewOptions to not arranged
    set icon size of viewOptions to 128
    set position of item "reg-server.app" of container window to {140, 150}
    set position of item "Applications" of container window to {400, 150}
    close
    open
    update without registering applications
    delay 1
  end tell
end tell
EOF
  sync
  hdiutil detach "$mount" || hdiutil detach "$mount" -force
  hdiutil convert "$rw" -format UDZO -ov -o "$dmg"
  rm -f "$rw"
}

case "$os" in
  macos)
    prefix="$work/root/Library/rust-reg"
    version_dir="$prefix/versions/$version"
    mkdir -p "$prefix/bin" "$(dirname "$work/root/Library/LaunchDaemons")"
    mkdir -p "$work/root/Library/LaunchDaemons"
    if [[ "$with_desk" == 1 ]]; then
      cp "$release_dir/$desk_name" "$prefix/bin/$desk_name"
      chmod 755 "$prefix/bin/$desk_name"
      app="$work/root/Applications/reg-server.app/Contents"
      mkdir -p "$app/MacOS"
      cp "$release_dir/$desk_name" "$app/MacOS/reg-server"
      chmod 755 "$app/MacOS/reg-server"
      install_app_icon "$app/MacOS/reg-server" "$app/Resources"
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
  <key>CFBundleIconFile</key>
  <string>AppIcon</string>
  <key>CFBundleIconName</key>
  <string>AppIcon</string>
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
    cp "$version_dir/$run_name" "$prefix/bin/$run_name"
    chmod 755 "$prefix/bin/$run_name"
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
    if [[ "$with_desk" == 1 ]]; then
      if [[ "$mac_cpu" == intel ]]; then
        dmg="$release_dir/rust-reg-${version}-intel.dmg"
      else
        dmg="$release_dir/rust-reg-${version}.dmg"
      fi
      write_drag_dmg "$work/root/Applications/reg-server.app" "$dmg" "$version" "$version_dir" "$run_name"
    fi
    ;;
  linux)
    deb="$work/deb"
    version_dir="$deb/opt/rust-reg/versions/$version"
    mkdir -p "$deb/opt/rust-reg/bin" "$deb/lib/systemd/system" "$deb/DEBIAN"
    if [[ "$with_desk" == 1 ]]; then
      cp "$release_dir/$desk_name" "$deb/opt/rust-reg/bin/$desk_name"
      chmod 755 "$deb/opt/rust-reg/bin/$desk_name"
      mkdir -p "$deb/usr/share/applications" "$deb/opt/rust-reg/share/icons" "$deb/usr/share/pixmaps"
    # Same mark as macOS (desk/icons from mac --write-icon + brand blue).
    # Prefer vendored PNGs so a Linux guest build matches the Mac menu-bar shape.
    # Desktop Icon= uses a content-hash filename so GNOME does not keep showing a
    # cached bitmap for the previous /opt/.../rust-reg.png path (Wayland cannot
    # hot-restart the shell; a same-path replace often needs logout otherwise).
    icons_src="$root/desk/icons"
    icon_src_128=""
    icon_tmp=""
    if [[ -f "$icons_src/rust-reg-128.png" ]]; then
      echo "pack: using vendored macOS desk icons from desk/icons" >&2
      icon_src_128="$icons_src/rust-reg-128.png"
      for edge in 22 24 32 48 64 128 256; do
        mkdir -p "$deb/usr/share/icons/hicolor/${edge}x${edge}/apps"
        src="$icons_src/rust-reg-${edge}.png"
        if [[ -f "$src" ]]; then
          cp -f "$src" "$deb/usr/share/icons/hicolor/${edge}x${edge}/apps/rust-reg.png"
        else
          cp -f "$icon_src_128" \
            "$deb/usr/share/icons/hicolor/${edge}x${edge}/apps/rust-reg.png"
        fi
      done
    else
      echo "pack: desk/icons missing; falling back to --write-icon" >&2
      icon_tmp=$(mktemp "${TMPDIR:-/tmp}/rust-reg-icon.XXXXXX.png")
      if "$release_dir/$desk_name" --write-icon "$icon_tmp" 128; then
        icon_src_128="$icon_tmp"
        for edge in 22 24 32 48 64 128 256; do
          mkdir -p "$deb/usr/share/icons/hicolor/${edge}x${edge}/apps"
          "$release_dir/$desk_name" --write-icon \
            "$deb/usr/share/icons/hicolor/${edge}x${edge}/apps/rust-reg.png" "$edge" \
            || cp "$icon_tmp" "$deb/usr/share/icons/hicolor/${edge}x${edge}/apps/rust-reg.png"
        done
      fi
    fi
    desktop_icon="/opt/rust-reg/share/icons/rust-reg.png"
    if [[ -n "$icon_src_128" && -f "$icon_src_128" ]]; then
      cp -f "$icon_src_128" "$deb/opt/rust-reg/share/icons/rust-reg.png"
      cp -f "$icon_src_128" "$deb/usr/share/pixmaps/rust-reg.png"
      icon_stamp=$(sha256sum "$icon_src_128" | awk '{ print substr($1, 1, 12) }')
      stamped="rust-reg-${icon_stamp}.png"
      cp -f "$icon_src_128" "$deb/opt/rust-reg/share/icons/$stamped"
      desktop_icon="/opt/rust-reg/share/icons/$stamped"
      echo "pack: desktop Icon=${desktop_icon}" >&2
    fi
    rm -f "${icon_tmp:-}"
      # Absolute hashed Icon= so Activities picks up a replaced mark without logout.
      cat > "$deb/usr/share/applications/rust-reg.desktop" <<EOF
[Desktop Entry]
Type=Application
Version=1.0
Name=reg-server
GenericName=Registration server
Comment=Registration server tray — start and manage the local server
# NO_DETACH: stay the main PID under GNOME's systemd scope (fork+exit looks like a crash).
Exec=env RUST_REG_DESK_NO_DETACH=1 /opt/rust-reg/bin/rust-reg-desk
TryExec=/opt/rust-reg/bin/rust-reg-desk
Icon=${desktop_icon}
Terminal=false
Categories=Network;Office;Utility;
Keywords=registration;server;tray;rust-reg;
StartupNotify=false
X-GNOME-UsesNotifications=true
EOF
    fi
    assemble_version "$version_dir"
    cp "$version_dir/$run_name" "$deb/opt/rust-reg/bin/$run_name"
    chmod 755 "$deb/opt/rust-reg/bin/$run_name"
    # Supervisor reads updates/live to choose versions/<live>/rust-reg (same as the macOS prefix).
    mkdir -p "$deb/opt/rust-reg/updates"
    printf '%s\n' "$version" > "$deb/opt/rust-reg/updates/live"
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
Architecture: $deb_arch
Maintainer: OpenDOps <opendops@users.noreply.github.com>
Description: Registration server
EOF
    if [[ "$with_desk" == 1 ]]; then
      cat > "$deb/DEBIAN/postinst" <<'EOF'
#!/bin/sh
set -e
case "$1" in
configure)
  if command -v systemctl >/dev/null 2>&1; then
    systemctl daemon-reload
    systemctl enable rust-reg.service
  fi
  # Older packs shipped a geometric SVG that outranked PNGs; drop leftovers.
  rm -f /usr/share/icons/hicolor/scalable/apps/rust-reg.svg
  rmdir /usr/share/icons/hicolor/scalable/apps 2>/dev/null || true
  rmdir /usr/share/icons/hicolor/scalable 2>/dev/null || true
  # Drop previous content-hash desktop icons; keep rust-reg.png + current Icon=.
  cur_icon=
  if [ -f /usr/share/applications/rust-reg.desktop ]; then
    cur_icon=$(sed -n 's/^Icon=//p' /usr/share/applications/rust-reg.desktop | head -n 1)
  fi
  for f in /opt/rust-reg/share/icons/rust-reg-*.png; do
    [ -e "$f" ] || continue
    [ -n "$cur_icon" ] && [ "$f" = "$cur_icon" ] && continue
    rm -f "$f"
  done
  # Force hicolor rebuild so tray theme name "rust-reg" picks up replaced PNGs.
  touch /usr/share/icons/hicolor /usr/share/applications/rust-reg.desktop 2>/dev/null || true
  rm -f /usr/share/icons/hicolor/icon-theme.cache
  if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -f -t /usr/share/icons/hicolor || true
  elif command -v update-icon-caches >/dev/null 2>&1; then
    update-icon-caches /usr/share/icons/hicolor || true
  fi
  if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database /usr/share/applications || true
  fi
  if command -v xdg-desktop-menu >/dev/null 2>&1; then
    xdg-desktop-menu forceupdate || true
  fi
  # Best-effort: poke logged-in sessions (GNOME Wayland still may need overview
  # closed/reopened; hashed Icon= avoids a full logout for launcher mark changes).
  for bus in /run/user/*/bus; do
    [ -S "$bus" ] || continue
    uid=$(stat -c %u "$(dirname "$bus")" 2>/dev/null) || continue
    runuser -u "#$uid" -- env "DBUS_SESSION_BUS_ADDRESS=unix:path=$bus" \
      xdg-desktop-menu forceupdate >/dev/null 2>&1 || true
  done
  ;;
esac
EOF
      cat > "$deb/DEBIAN/postrm" <<'EOF'
#!/bin/sh
set -e
case "$1" in
remove|purge)
  rm -f /usr/share/icons/hicolor/scalable/apps/rust-reg.svg
  rm -f /usr/share/icons/hicolor/icon-theme.cache
  if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -f -t /usr/share/icons/hicolor || true
  fi
  if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database /usr/share/applications || true
  fi
  ;;
esac
EOF
      chmod 755 "$deb/DEBIAN/postrm"
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
    asset="$release_dir/rust-reg_${version}_${deb_arch}.deb"
    dpkg-deb --root-owner-group --build "$deb" "$asset" >&2
    ;;
  windows)
    tree_version="$work/tree/versions/$version"
    version_dir="$tree_version"
    mkdir -p "$work/tree/bin" "$tree_version"
    assemble_version "$tree_version"
    cp "$tree_version/$run_name" "$work/tree/bin/$run_name"
    if [[ "$with_desk" == 1 ]]; then
      cp "$release_dir/$desk_name" "$work/tree/bin/$desk_name"
    fi
    mkdir -p "$work/tree/updates"
    printf '%s\n' "$version" > "$work/tree/updates/live"
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

# Platform tag for zip names and file lines (release step 16).
platform_tag=""
case "$os" in
  macos)
    if [[ "$mac_cpu" == intel ]]; then
      platform_tag=macos-intel
    else
      platform_tag=macos
    fi
    ;;
  linux)
    if [[ "${deb_arch:-}" == arm64 ]]; then
      platform_tag=linux-arm64
    else
      platform_tag=linux
    fi
    ;;
  windows) platform_tag=windows ;;
esac

desk_for_zip=""
if [[ "$with_desk" == 1 ]]; then
  desk_for_zip="$release_dir/$desk_name"
fi
# Drop the pre-step-16 single upgrade zip if a prior pack left one behind.
rm -f \
  "$release_dir/rust-reg-${version}-${os}.zip" \
  "$release_dir/rust-reg-${version}-macos-intel.zip" \
  "$release_dir/rust-reg-${version}-linux-arm64.zip"

print_sum_as "$asset" "$(basename "$asset")"
if [[ -n "${dmg:-}" ]]; then
  print_sum_as "$dmg" "$(basename "$dmg")"
fi
write_upgrade_zips \
  "$release_dir" \
  "$version" \
  "$platform_tag" \
  "$version_dir" \
  "$bin_name" \
  "$run_name" \
  "$desk_for_zip"
