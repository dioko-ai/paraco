#!/usr/bin/env bash
# Uses ephemeral local keys and synthetic payloads; no publisher credentials/network.
set -euo pipefail
base=$(mktemp -d); trap 'rm -rf "$base"' EXIT
reject(){ if "$@"; then echo "expected rejection: $*" >&2; exit 1; fi; }
hash(){ if command -v sha256sum >/dev/null; then sha256sum "$1"; else shasum -a 256 "$1"; fi | awk '{print $1}'; }
openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:2048 -out "$base/key.pem" 2>/dev/null
openssl pkey -in "$base/key.pem" -pubout -out "$base/public.pem" 2>/dev/null
python3 - "$base" <<'PY'
import hashlib, io, json, sys, tarfile
from pathlib import Path
base = Path(sys.argv[1])
for kind in ('good', 'dirty', 'fixture', 'bad-hash', 'bad-version', 'bad-target', 'no-notice', 'unrecorded'):
    files = {'bin/paraco':b'#!/bin/sh\nexit 0\n', 'libexec/paraco/deno':b'#!/bin/sh\nexit 0\n', 'notices/LICENSE':b'Copyright author. All rights reserved.\n', 'notices/DENO-NOTICE':b'Copyright runtime author. Permission granted.\n', 'notices/RUST-THIRD-PARTY-NOTICES':b'Copyright dependency authors. Permission granted.\n'}
    if kind == 'fixture': files['notices/DENO-NOTICE'] = b'TEST FIXTURE ONLY - NOT DISTRIBUTABLE\n'
    meta = dict(paracoVersion='1', denoVersion='2.2.5', target='x86_64-unknown-linux-gnu', sourceRevision='a'*40, sourceDirty=kind=='dirty', notices=[n for n in files if n.startswith('notices/')], checksums={n:hashlib.sha256(d).hexdigest() for n,d in files.items()})
    if kind == 'unrecorded': del meta['sourceDirty']
    if kind == 'bad-hash': meta['checksums']['bin/paraco'] = '0'*64
    if kind == 'bad-version': meta['paracoVersion'] = '2'
    if kind == 'bad-target': meta['target'] = 'aarch64-apple-darwin'
    if kind == 'no-notice': del files['notices/DENO-NOTICE']
    files['bundle.json'] = json.dumps(meta).encode()
    with tarfile.open(base / f'{kind}.tgz', 'w:gz', format=tarfile.USTAR_FORMAT) as t:
        for name,data in files.items():
            h=tarfile.TarInfo(f'paraco-1-x86_64-unknown-linux-gnu/{name}'); h.size=len(data); h.mode=0o755 if name.startswith(('bin/','libexec/')) else 0o644; t.addfile(h,io.BytesIO(data))
PY
for archive in "$base/"*.tgz; do openssl dgst -sha256 -sign "$base/key.pem" -out "$archive.sig" "$archive"; done
install(){ scripts/install-release.sh --archive "$base/$1.tgz" --signature "$base/$1.tgz.sig" --public-key "$base/public.pem" --sha256 "$(hash "$base/$1.tgz")" --version 1 --target x86_64-unknown-linux-gnu --prefix "$base/prefix"; }
verify(){ scripts/verify-release.sh --archive "$base/$1.tgz" --signature "$base/$1.tgz.sig" --public-key "$base/public.pem" --sha256 "$(hash "$base/$1.tgz")" --version 1 --target x86_64-unknown-linux-gnu; }
verify good
openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:2048 -out "$base/other-key.pem" 2>/dev/null
openssl pkey -in "$base/other-key.pem" -pubout -out "$base/other-public.pem" 2>/dev/null
reject scripts/verify-release.sh --archive "$base/good.tgz" --signature "$base/good.tgz.sig" --public-key "$base/other-public.pem" --sha256 "$(hash "$base/good.tgz")" --version 1 --target x86_64-unknown-linux-gnu
reject scripts/verify-release.sh --archive "$base/good.tgz" --signature "$base/good.tgz.sig" --public-key "$base/public.pem" --sha256 "$(printf '%064d' 0)" --version 1 --target x86_64-unknown-linux-gnu
install good
pointer=$(readlink "$base/prefix/channels/stable")
for kind in dirty fixture bad-hash bad-version bad-target no-notice unrecorded; do
 reject verify "$kind"
 reject install "$kind"
 [[ $(readlink "$base/prefix/channels/stable") == "$pointer" ]]
done
cp "$base/good.tgz" "$base/tampered.tgz"
printf 'tamper' >> "$base/tampered.tgz"
cp "$base/good.tgz.sig" "$base/tampered.tgz.sig"
reject verify tampered
reject install tampered
# Consumer installer embeds the trusted key/tools; users supply no release metadata.
python3 - "$base" <<'PYFIXTURE'
import hashlib, json, sys
from pathlib import Path
base = Path(sys.argv[1])
asset = dict(url='https://release.invalid/archive', signatureUrl='https://release.invalid/signature',
             sha256=hashlib.sha256((base/'good.tgz').read_bytes()).hexdigest())
(base/'consumer.json').write_text(json.dumps(dict(version='1', assets={'x86_64-unknown-linux-gnu':asset})))
PYFIXTURE
python3 scripts/generate-consumer-installer.py --manifest "$base/consumer.json" --public-key "$base/public.pem" --output "$base/consumer.sh"
bash -n "$base/consumer.sh"
mkdir "$base/mock-bin"
cat > "$base/mock-bin/uname" <<'SH'
#!/bin/sh
case "$1" in -s) echo Linux;; -m) echo x86_64;; esac
SH
cat > "$base/mock-bin/curl" <<'SH'
#!/bin/bash
set -eu
output=
while (($#)); do
 case "$1" in
 --output) output=$2; shift 2;;
 https://release.invalid/archive) cp "$PARACO_TEST_RELEASE/good.tgz" "$output"; exit;;
 https://release.invalid/signature) cp "$PARACO_TEST_RELEASE/good.tgz.sig" "$output"; exit;;
 *) shift;;
 esac
done
exit 1
SH
# Keep Linux target selection deterministic while supporting BSD mv on macOS.
cat > "$base/mock-bin/mv" <<'SH'
#!/bin/bash
if [[ ${1:-} == -Tf ]]; then
 shift
 python3 - "$@" <<'PYMOVE'
import os, sys
os.replace(sys.argv[1], sys.argv[2])
PYMOVE
else
 /bin/mv "$@"
fi
SH
chmod +x "$base/mock-bin/"*
consumer(){ PATH="$base/mock-bin:$PATH" PARACO_TEST_RELEASE="$base" bash "$base/consumer.sh" --prefix "$base/consumer prefix"; }
consumer
[[ -x "$base/consumer prefix/channels/stable/bin/paraco" ]]
consumer # Same immutable version is harmless.
cp "$base/good.tgz.sig" "$base/saved.sig"
printf 'invalid signature' > "$base/good.tgz.sig"
reject env PATH="$base/mock-bin:$PATH" PARACO_TEST_RELEASE="$base" bash "$base/consumer.sh" --prefix "$base/rejected"
[[ ! -e "$base/rejected/channels/stable" ]]
mv "$base/saved.sig" "$base/good.tgz.sig"
reject bash "$base/consumer.sh" --prefix relative
cat > "$base/mock-bin/uname" <<'SH'
#!/bin/sh
case "$1" in -s) echo Linux;; -m) echo riscv64;; esac
SH
reject consumer

printf 'invalid signature' > "$base/good.tgz.sig"
reject verify good
reject install good
[[ $(readlink "$base/prefix/channels/stable") == "$pointer" ]]
reject scripts/install-release.sh --url http://example.invalid/archive --signature-url https://example.invalid/signature --public-key "$base/public.pem" --sha256 "$(hash "$base/good.tgz")" --version 1 --target x86_64-unknown-linux-gnu --prefix "$base/prefix"
[[ $(readlink "$base/prefix/channels/stable") == "$pointer" ]]
echo 'local signed release verification, tamper rejection and activation preservation passed'
