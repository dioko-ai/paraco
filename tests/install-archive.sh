#!/usr/bin/env bash
set -euo pipefail
base=$(mktemp -d); trap 'rm -rf "$base"' EXIT
root=paraco-1-x86_64-unknown-linux-gnu
mkdir -p "$base/$root/bin" "$base/$root/libexec/paraco"; printf '#!/bin/sh\n' > "$base/$root/bin/paraco"; printf '#!/bin/sh\n' > "$base/$root/libexec/paraco/deno"; chmod +x "$base/$root/bin/paraco" "$base/$root/libexec/paraco/deno"; printf '{}\n' > "$base/$root/bundle.json"
tar -C "$base" -czf "$base/good.tgz" "$root"
sum=$(sha256sum "$base/good.tgz"|awk '{print $1}')
scripts/install-archive.sh install --archive "$base/good.tgz" --sha256 "$sum" --version 1 --prefix "$base/prefix"
[[ -L "$base/prefix/channels/stable" ]]
[[ -x "$base/prefix/channels/stable/bin/paraco" ]]
printf x > "$base/evil"; tar -C "$base" -czf "$base/evil.tgz" --transform='s,^,../,' evil
bad=$(sha256sum "$base/evil.tgz"|awk '{print $1}')
! scripts/install-archive.sh install --archive "$base/evil.tgz" --sha256 "$bad" --version 2 --prefix "$base/prefix"
[[ -L "$base/prefix/channels/stable" ]] # failed validation did not alter active release
