#!/usr/bin/env bash
# Root-side helpers for three-zip step 11 (macOS proof). Run with sudo.
#
#   sudo bash scripts/release/mac-proof.sh install <rust-reg-X.pkg>
#       Install pkg, add prerelease env + stderr log to the daemon, (re)load it.
#   sudo bash scripts/release/mac-proof.sh restart
#       launchctl kickstart -k the daemon (picks up a replaced bin/rust-reg-run).
#
# Daemon stderr (upgrade-probe-* lines) goes to /Library/rust-reg/proof.log.
set -euo pipefail

label=com.opendops.rust-reg
plist=/Library/LaunchDaemons/$label.plist
log=/Library/rust-reg/proof.log
pb=/usr/libexec/PlistBuddy

[[ $EUID -eq 0 ]] || { echo "run with sudo" >&2; exit 1; }

set_key() { # key type value
  "$pb" -c "Delete :$1" "$plist" >/dev/null 2>&1 || true
  "$pb" -c "Add :$1 $2 $3" "$plist"
}

case "${1:-}" in
  install)
    pkg=${2:?pkg path}
    installer -pkg "$pkg" -target /
    "$pb" -c "Delete :EnvironmentVariables" "$plist" >/dev/null 2>&1 || true
    "$pb" -c "Add :EnvironmentVariables dict" "$plist"
    "$pb" -c "Add :EnvironmentVariables:REGISTRATION_UPDATE_PRERELEASE string 1" "$plist"
    set_key StandardErrorPath string "$log"
    set_key StandardOutPath string "$log"
    touch "$log" && chmod 644 "$log"
    launchctl bootout system/$label 2>/dev/null || true
    launchctl bootstrap system "$plist"
    launchctl print system/$label | grep -E 'state|pid' | head -3
    ;;
  restart)
    launchctl kickstart -k system/$label
    launchctl print system/$label | grep -E 'state|pid' | head -3
    ;;
  *)
    sed -n '2,9p' "$0" >&2
    exit 2
    ;;
esac
