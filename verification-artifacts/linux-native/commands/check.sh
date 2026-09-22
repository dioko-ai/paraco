#!/bin/bash
set -uo pipefail
source /tmp/paraco-native-tools/env.sh
out=verification-artifacts/linux-native
{ date -u; uname -a; cat /etc/os-release; getconf GNU_LIBC_VERSION; lscpu | head -25; rustc --version; cargo --version; rustup component list --installed; deno --version; node --version; systemctl --version; systemctl --user show-environment >/dev/null; echo "user bus exit: $?"; loginctl show-user "$(id -un)" -p Linger -p State; printf 'session type: %s\n' "${XDG_SESSION_TYPE:-unset}"; git rev-parse HEAD; git status --short; git diff --binary | sha256sum; } > "$out/environment.txt" 2>&1
run() { name=$1; shift; "$@" > "$out/$name.log" 2>&1; rc=$?; echo "$name=$rc" | tee -a "$out/exit-statuses.txt"; return "$rc"; }
run npm-ci npm ci || exit $?
run playwright-install npx playwright install chromium || exit $?
run check npm run check
