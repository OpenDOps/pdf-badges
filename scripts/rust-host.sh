#!/usr/bin/env bash
# Print the rustup toolchain for this CPU, and install it when it is missing.
# Apple Silicon uses stable-aarch64-apple-darwin. An Intel Mac uses
# stable-x86_64-apple-darwin. Linux and Windows keep stable.
# On a Mac, plain cargo in this checkout is pointed at that toolchain too.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)

if [[ "$(uname -s)" != Darwin ]]; then
  printf '%s\n' stable
  exit 0
fi

if arch -arm64 /usr/bin/true >/dev/null 2>&1; then
  toolchain=stable-aarch64-apple-darwin
else
  toolchain=stable-x86_64-apple-darwin
fi

if ! rustup toolchain list | awk '{print $1}' | grep -qx "$toolchain"; then
  echo "rust-host: installing ${toolchain}" >&2
  rustup toolchain install "$toolchain" >&2
fi

(cd "$root" && rustup override set "$toolchain" >&2)
printf '%s\n' "$toolchain"
