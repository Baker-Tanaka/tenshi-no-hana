#!/bin/sh
set -eu

project_root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
cd "$project_root"

config=.bakerlink/connection.json
elf=target/thumbv6m-none-eabi/debug/tenshi-no-hana

if [ ! -r "$config" ]; then
    echo "Baker Link connection is missing. Open this project from Baker Link Env." >&2
    exit 1
fi
if [ ! -r "$elf" ]; then
    echo "Debug ELF was not produced: $elf" >&2
    exit 1
fi

url=$(jq -er '.uploadUrl' "$config") || {
    echo "Baker Link connection has no uploadUrl." >&2
    exit 1
}
checksum=$(sha256sum "$elf" | cut -d ' ' -f 1)

curl \
    --fail-with-body \
    --silent \
    --show-error \
    --retry 5 \
    --retry-connrefused \
    --retry-delay 1 \
    -X PUT \
    -H "X-Content-SHA256: $checksum" \
    -H "Content-Type: application/octet-stream" \
    --upload-file "$elf" \
    "$url/v1/artifacts/tenshi-no-hana/debug" || {
    status=$?
    echo >&2
    if [ "$status" -eq 7 ]; then
        echo "Cannot connect to Baker Link ELF upload server at $url." >&2
        echo "Start Baker Link Env on the host with Run. Bind IP must be reachable from this container (e.g. 0.0.0.0, not 127.0.0.1)." >&2
    else
        echo "ELF upload to $url failed (curl exit $status). See the error above and the Baker Link Env log." >&2
    fi
    exit "$status"
}

echo
echo "ELF uploaded to Baker Link Env."