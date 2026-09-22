#!/bin/bash
set -uo pipefail
source /tmp/paraco-native-tools/env.sh
out=verification-artifacts/linux-native
run() { name=$1; shift; "$@" > "$out/$name.log" 2>&1; rc=$?; echo "$name=$rc" | tee -a "$out/exit-statuses.txt"; return "$rc"; }
printf 'VERIFICATION FIXTURE ONLY - NOT DISTRIBUTABLE. Not complete redistribution notices.\n' > /tmp/paraco-native-tools/fixture-notice
run installer npm run test:installer || exit $?
run bundle-build scripts/build-bundle.sh --deno /tmp/paraco-native-tools/deno/deno --deno-notice /tmp/paraco-native-tools/fixture-notice --third-party-notice /tmp/paraco-native-tools/fixture-notice --target x86_64-unknown-linux-gnu --output /tmp/paraco-native-tools/bundle-output || exit $?
run bundle-smoke tests/bundle-smoke.sh --deno /tmp/paraco-native-tools/deno/deno --deno-notice /tmp/paraco-native-tools/fixture-notice --third-party-notice /tmp/paraco-native-tools/fixture-notice --target x86_64-unknown-linux-gnu || exit $?
mkdir /tmp/paraco-native-tools/bundle
 tar -xzf /tmp/paraco-native-tools/bundle-output/*.tar.gz -C /tmp/paraco-native-tools/bundle
cp /tmp/paraco-native-tools/bundle-output/SHA256SUMS "$out/bundle-sha256.txt"
cp /tmp/paraco-native-tools/bundle/paraco-*/bundle.json "$out/bundle.json"
run systemd env PARACO_BIN=/tmp/paraco-native-tools/bundle/paraco-0.1.0-x86_64-unknown-linux-gnu/bin/paraco python3 scripts/verify-service.py
