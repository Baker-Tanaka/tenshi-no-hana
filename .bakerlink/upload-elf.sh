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
token=$(jq -er '.token' "$config") || {
    echo "Baker Link connection has no token." >&2
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
    -H "Authorization: Bearer $token" \
    -H "X-Content-SHA256: $checksum" \
    -H "Content-Type: application/octet-stream" \
    --upload-file "$elf" \
    "$url/v1/artifacts/tenshi-no-hana/debug"

echo
echo "ELF uploaded to Baker Link Env."