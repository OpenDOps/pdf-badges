#!/bin/sh
# Bring the server up healthy, print the queued kinds, and confirm the image
# has no Python or LibreOffice.
set -eu
cd "$(dirname "$0")/../../.."

docker compose up --build --wait google-tables
docker compose run --rm --no-deps --entrypoint /google-tables google-tables smoke

cid="$(docker create google-tables)"
if docker export "$cid" | tar -t | grep -E '(^|/)(python|soffice)([0-9.]*)?$'; then
    docker rm "$cid" >/dev/null
    echo "image contains python or soffice" >&2
    exit 1
fi
docker rm "$cid" >/dev/null
