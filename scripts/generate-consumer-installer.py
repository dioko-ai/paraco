#!/usr/bin/env python3
"""Package trusted release inputs into one standalone, unpublished Bash installer."""
import argparse
import base64
import json
from pathlib import Path
import re
import shlex

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--manifest', required=True, type=Path)
parser.add_argument('--public-key', required=True, type=Path)
parser.add_argument('--output', required=True, type=Path)
args = parser.parse_args()
manifest = json.loads(args.manifest.read_text())
version = manifest['version']
if not re.fullmatch(r'[0-9][A-Za-z0-9._-]*', version):
    parser.error('invalid version')
supported = {'x86_64-unknown-linux-gnu', 'aarch64-unknown-linux-gnu',
             'x86_64-apple-darwin', 'aarch64-apple-darwin'}
assets = manifest['assets']
if not assets or not set(assets) <= supported:
    parser.error('provide at least one supported, natively verified target')
key = args.public_key.read_bytes()
if b'-----BEGIN PUBLIC KEY-----' not in key:
    parser.error('a PEM publisher public key is required')
parts = ['''#!/usr/bin/env bash
# Generated release installer. Trust this script as you would the release publisher.
set -euo pipefail
prefix="${HOME:?}/.local/share/paraco"
while (($#)); do
  case "$1" in
    --prefix) prefix=${2:?}; shift 2;;
    --help) echo 'Usage: bash install.sh [--prefix /absolute/path]'; exit 0;;
    *) echo "Unknown option: $1" >&2; exit 2;;
  esac
done
[[ $prefix == /* ]] || { echo 'An absolute installation prefix is required' >&2; exit 2; }
case "$(uname -s)/$(uname -m)" in
  Linux/x86_64) target=x86_64-unknown-linux-gnu;;
  Linux/aarch64|Linux/arm64) target=aarch64-unknown-linux-gnu;;
  Darwin/x86_64) target=x86_64-apple-darwin;;
  Darwin/arm64) target=aarch64-apple-darwin;;
  *) echo 'This release does not support your operating system or processor.' >&2; exit 1;;
esac
for tool in curl openssl perl gzip tar; do
  command -v "$tool" >/dev/null || { echo "Missing system utility: $tool" >&2; exit 1; }
done
base=$(mktemp -d); trap 'rm -rf "$base"' EXIT
''', 'version=' + shlex.quote(version) + '\ncase "$target" in\n']
for target, asset in sorted(assets.items()):
    if not re.fullmatch('[a-f0-9]{64}', asset['sha256']):
        parser.error('invalid SHA-256')
    for field in ('url', 'signatureUrl'):
        if not re.fullmatch(r'https://[^\s]+', asset[field]):
            parser.error('release URLs must use HTTPS')
    parts.append(f"  {target}) url={shlex.quote(asset['url'])}; signature_url={shlex.quote(asset['signatureUrl'])}; checksum={asset['sha256']};;\n")
parts.append('  *) echo "No published bundle for $target in this release." >&2; exit 1;;\nesac\n')
scripts = Path(__file__).resolve().parent
payloads = {'publisher.pem': key}
for name in ('install-release.sh', 'verify-release.sh', 'install-archive.sh', 'validate-archive.pl'):
    payloads[name] = (scripts / name).read_bytes()
for name, payload in payloads.items():
    encoded = base64.b64encode(payload).decode()
    parts.append(f'''perl -MMIME::Base64 -e 'local $/; print decode_base64(<STDIN>)' > "$base/{name}" <<'PARACO_PAYLOAD'
{encoded}
PARACO_PAYLOAD
''')
parts.append('''chmod +x "$base/"*.sh
# Repeating installation of an immutable version leaves its files untouched.
if [[ -e "$prefix/releases/$version" ]]; then
  echo "Release $version is already retained at $prefix. No files changed."
  exit 0
fi
"$base/install-release.sh" --url "$url" --signature-url "$signature_url" \\
  --public-key "$base/publisher.pem" --sha256 "$checksum" \\
  --version "$version" --target "$target" --prefix "$prefix"
echo
echo "Installed Paraco $version, including its private runtime."
printf 'Start with: %q cron\\n' "$prefix/channels/stable/bin/paraco"
echo 'To make paraco available in new terminals, add this line to your shell profile:'
printf 'export PATH=%q:"$PATH"\\n' "$prefix/channels/stable/bin"
echo 'Your shell profile was not changed. No background service was enabled.'
''')
with args.output.open('x') as output:
    output.write(''.join(parts))
args.output.chmod(0o755)
print(f'Created {args.output}. Review and test against final signed assets before publication.')
