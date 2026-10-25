#!/usr/bin/env bash
set -euo pipefail
base=$(mktemp -d); trap 'rm -rf "$base"' EXIT
reject(){ if "$@"; then echo "expected rejection: $*" >&2; exit 1; fi; }
hash(){ if command -v sha256sum >/dev/null; then sha256sum "$1"; else shasum -a 256 "$1"; fi | awk '{print $1}'; }
# Python is only a test-fixture dependency; the installer uses stock Perl/gzip/tar.
python3 - "$base" <<'PY'
import gzip, io, sys, tarfile
from pathlib import Path
base = Path(sys.argv[1])
for version in ('1', '2', '3'):
    root = f'paraco-{version}-test'
    with tarfile.open(base / f'{version}.tgz', 'w:gz', format=tarfile.USTAR_FORMAT) as t:
        for name, data, mode in [('bin/paraco', b'#!/bin/sh\n', 0o755), ('libexec/paraco/deno', b'#!/bin/sh\n', 0o755), ('bundle.json', b'{}\n', 0o644)]:
            h = tarfile.TarInfo(f'{root}/{name}'); h.size=len(data); h.mode=mode; t.addfile(h, io.BytesIO(data))
raw = gzip.decompress((base / '3.tgz').read_bytes())
corrupt = bytearray(raw); corrupt[0] ^= 1
(base / 'corrupt.tgz').write_bytes(gzip.compress(corrupt))
(base / 'trailer.tgz').write_bytes(gzip.compress(raw + b'not padding'))
for kind in ('traversal', 'symlink', 'hardlink', 'fifo', 'duplicate', 'oversize', 'absolute', 'extension', 'truncated'):
    with tarfile.open(base / f'{kind}.tgz', 'w:gz', format=tarfile.USTAR_FORMAT) as t:
        h = tarfile.TarInfo('paraco-3-test/evil')
        if kind == 'absolute': h.name = '/escaped'
        if kind == 'extension': h.type = tarfile.XHDTYPE
        if kind == 'truncated':
            h.size = 512; t.fileobj.write(h.tobuf(format=tarfile.USTAR_FORMAT)); continue
        if kind == 'traversal': h.name = '../escaped'
        if kind == 'symlink': h.type = tarfile.SYMTYPE; h.linkname = '../escaped'
        if kind == 'hardlink': h.type = tarfile.LNKTYPE; h.linkname = '../escaped'
        if kind == 'fifo': h.type = tarfile.FIFOTYPE
        if kind == 'oversize':
            # Header advertises an oversized payload without allocating it.
            h.size = 1073741825; t.fileobj.write(h.tobuf(format=tarfile.USTAR_FORMAT)); continue
        t.addfile(h)
        if kind == 'duplicate': t.addfile(h)
PY
install(){ scripts/install-archive.sh install --archive "$base/$1.tgz" --sha256 "$(hash "$base/$1.tgz")" --version "${2:-$1}" --prefix "$base/prefix"; }
assert_active(){ [[ $(readlink "$base/prefix/channels/stable") == "../releases/$1" && -x "$base/prefix/channels/stable/bin/paraco" ]]; }
head -c 20 "$base/1.tgz" > "$base/truncated.tgz"
reject install truncated 3
install 1; assert_active 1
install 2; assert_active 2
reject install 2; assert_active 2 # immutable slot
for kind in traversal symlink hardlink fifo duplicate oversize absolute extension corrupt trailer; do
  reject install "$kind" 3
  assert_active 2
  [[ ! -e "$base/prefix/releases/3" ]]
done
reject scripts/install-archive.sh install --archive "$base/1.tgz" --sha256 "$(printf '%064d' 0)" --version 3 --prefix "$base/prefix"
assert_active 2
scripts/install-archive.sh rollback --version 1 --prefix "$base/prefix"; assert_active 1
reject scripts/install-archive.sh rollback --version missing --prefix "$base/prefix"; assert_active 1
mkdir -p "$base/prefix/state"; printf '2\n' > "$base/prefix/state/format"
reject scripts/install-archive.sh rollback --version 2 --prefix "$base/prefix"; assert_active 1
printf '1\n' > "$base/prefix/state/format"
scripts/install-archive.sh rollback --version 2 --prefix "$base/prefix"; assert_active 2
printf 'preserve\n' > "$base/prefix/state/data"
touch "$base/prefix/state/running.pid"
reject scripts/install-archive.sh uninstall --version 2 --prefix "$base/prefix"; assert_active 2
rm "$base/prefix/state/running.pid"
scripts/install-archive.sh uninstall --version 2 --prefix "$base/prefix"
[[ ! -L "$base/prefix/channels/stable" && -f "$base/prefix/state/data" && -x "$base/prefix/releases/1/bin/paraco" ]]
mkdir -p "$base/foreign/channels"
ln -s /nonexistent "$base/foreign/channels/stable"
reject scripts/install-archive.sh install --archive "$base/1.tgz" --sha256 "$(hash "$base/1.tgz")" --version 1 --prefix "$base/foreign"
[[ $(readlink "$base/foreign/channels/stable") == /nonexistent ]]
echo 'installer upgrade, failed upgrade, rollback, state compatibility and removal checks passed'
