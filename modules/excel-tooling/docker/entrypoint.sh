#!/bin/sh
set -eu

export HOME=/tmp/home
export XDG_CACHE_HOME=/tmp/cache
export XDG_CONFIG_HOME=/tmp/config
export TMPDIR=/tmp
export PYTHONDONTWRITEBYTECODE=1
mkdir -p "$HOME" "$XDG_CACHE_HOME" "$XDG_CONFIG_HOME"

rm -rf /tmp/libreoffice-profile
mkdir -p /tmp/libreoffice-profile
cp -a /opt/libreoffice-profile/. /tmp/libreoffice-profile/

soffice --headless --nologo --nofirststartwizard --norestore \
  -env:UserInstallation=file:///tmp/libreoffice-profile \
  --accept='socket,host=127.0.0.1,port=2002;urp;StarOffice.ComponentContext' &

ready=0
i=0
while [ "$i" -lt 90 ]; do
  if python3 -c 'import socket
try:
    socket.create_connection(("127.0.0.1", 2002), 1).close()
except OSError:
    raise SystemExit(1)'; then
    ready=1
    break
  fi
  i=$((i + 1))
  sleep 1
done
if [ "$ready" -ne 1 ]; then
  echo "soffice did not accept connections" >&2
  exit 1
fi

export PATH="/opt/venv/bin:${PATH}"
export EXCEL_LIBREOFFICE=1
cd /opt/excel-tooling
exec "$@"
