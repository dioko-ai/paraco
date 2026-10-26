#!/usr/bin/env bash
# Script contract fixture only; native compiled payload validation is bundle-smoke.sh.
set -euo pipefail
base=$(mktemp -d); trap 'rm -rf "$base"' EXIT
mkdir -p "$base/tools" "$base/target/fixture-target/release" "$base/input with spaces"
printf '#!/bin/sh\nexit 0\n' > "$base/tools/cargo"
printf '#!/bin/sh\necho workerd 2026-10-26\n' > "$base/input with spaces/workerd"
printf '#!/bin/sh\necho 0.28.2\n' > "$base/input with spaces/esbuild"
printf '#!/bin/sh\nexit 0\n' > "$base/target/fixture-target/release/paraco"
printf 'TEST NOTICE FIXTURE ONLY\n' > "$base/notice"
chmod +x "$base/tools/cargo" "$base/input with spaces/workerd" "$base/input with spaces/esbuild" "$base/target/fixture-target/release/paraco"
# Hide sha256sum so this fixture also exercises macOS's shasum fallback.
for tool in awk cat cp dirname du git grep gzip head ln mkdir mktemp mv perl rm rmdir sed shasum tar uname wc; do
  ln -s "$(command -v "$tool")" "$base/tools/$tool"
done
PATH="$base/tools" CARGO_TARGET_DIR="$base/target" scripts/build-bundle.sh --workerd "$base/input with spaces/workerd" --esbuild "$base/input with spaces/esbuild" --workerd-notice "$base/notice" --compiler-notice "$base/notice" --third-party-notice "$base/notice" --target fixture-target --output "$base/output with spaces"
version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -1)
archive="$base/output with spaces/paraco-$version-fixture-target.tar.gz"
perl scripts/validate-archive.pl "$archive" "$version"
checksum=$(shasum -a 256 "$archive" | awk '{print $1}')
PATH="$base/tools" /bin/bash scripts/install-archive.sh install --archive "$archive" --sha256 "$checksum" --version "$version" --prefix "$base/install with spaces"
[[ -x "$base/install with spaces/channels/stable/bin/paraco" ]]
python3 - "$archive" "$base/output with spaces/SHA256SUMS" <<'PY'
import hashlib, json, pathlib, sys, tarfile
archive = pathlib.Path(sys.argv[1])
assert hashlib.sha256(archive.read_bytes()).hexdigest() == pathlib.Path(sys.argv[2]).read_text().split()[0]
with tarfile.open(archive) as t:
    root = t.getnames()[0]
    metadata = json.load(t.extractfile(root + '/bundle.json'))
    assert metadata['target'] == 'fixture-target'
    assert metadata['workerdVersion'] == '2026-10-26'
    assert isinstance(metadata['sourceDirty'], bool)
    for path, expected in metadata['checksums'].items():
        assert hashlib.sha256(t.extractfile(root + '/' + path).read()).hexdigest() == expected
print('bundle fixture layout, metadata and all checksums verified')
PY
