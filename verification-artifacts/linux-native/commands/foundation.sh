#!/bin/bash
set -uo pipefail
source /tmp/paraco-native-tools/env.sh
out=verification-artifacts/linux-native
{ deno --version; node --version; sha256sum /tmp/paraco-native-tools/deno/deno; } > "$out/pinned-tools.txt"
for name in foundation prepared; do
  PARACO_BIN="$PWD/target/debug/paraco" python3 "scripts/verify-$name.py" > "$out/$name.log" 2>&1
  echo "$name=$?" | tee -a "$out/exit-statuses.txt"
done
