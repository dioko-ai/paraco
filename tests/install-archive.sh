#!/usr/bin/env bash
set -euo pipefail
base=$(mktemp -d); trap 'rm -rf "$base"' EXIT
mkdir -p "$base/paraco-1/bin"; printf '#!/bin/sh\n' > "$base/paraco-1/bin/paraco"; chmod +x "$base/paraco-1/bin/paraco"
tar -C "$base" -czf "$base/good.tgz" paraco-1
sum=$(sha256sum "$base/good.tgz"|awk '{print $1}')
scripts/install-archive.sh install --archive "$base/good.tgz" --sha256 "$sum" --version 1 --prefix "$base/prefix"
[[ -L "$base/prefix/channels/stable" ]]
printf x > "$base/evil"; tar -C "$base" -czf "$base/evil.tgz" --transform='s,^,../,' evil
bad=$(sha256sum "$base/evil.tgz"|awk '{print $1}')
! scripts/install-archive.sh install --archive "$base/evil.tgz" --sha256 "$bad" --version 2 --prefix "$base/prefix"
[[ -L "$base/prefix/channels/stable" ]] # failed validation did not alter active release
