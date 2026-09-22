#!/bin/bash
# Run INSIDE a clean Linux container/VM with networking disabled externally.
# Args: installed bundle launcher, prepared artifact, hello app.
set -euo pipefail
launcher=$1 artifact=$2 hello=$3
for tool in rustc cargo node deno; do
  if command -v "$tool" >/dev/null; then echo "unexpected system tool: $tool" >&2; exit 1; fi
done
echo 'Rust/cargo, Node, and system Deno absent'
ln -s "$launcher" /tmp/paraco-offline-launcher
cd /
child=
cleanup() { if [[ -n $child ]]; then kill -INT "$child" 2>/dev/null || true; wait "$child" 2>/dev/null || true; fi; }
trap cleanup EXIT
for mode in prepared prepared source; do
  if [[ $mode == prepared ]]; then args=(run-prepared "$artifact"); expected=dependency-ok
  else args=(run "$hello"); expected='Hello from Paraco'; fi
  PATH=/missing TMPDIR=/tmp /tmp/paraco-offline-launcher --log-dir /tmp/paraco-clean-logs "${args[@]}" --port 18781 >/tmp/paraco-clean.log 2>&1 & child=$!
  ready=0
  for ((attempt=0; attempt<200; attempt++)); do
    if grep -q 'listening on http://' /tmp/paraco-clean.log && { exec 3<>/dev/tcp/127.0.0.1/18781; } 2>/dev/null; then ready=1; break; fi
    if ! kill -0 "$child" 2>/dev/null; then cat /tmp/paraco-clean.log; exit 1; fi
    sleep .1
  done
  [[ $ready == 1 ]] || { cat /tmp/paraco-clean.log; exit 1; }
  printf 'GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n' >&3
  response=$(cat <&3); exec 3<&-; exec 3>&-
  [[ $response == *'200 OK'* && $response == *"$expected"* ]] || { echo "$response"; exit 1; }
  kill -INT "$child"
  for ((attempt=0; attempt<120; attempt++)); do
    kill -0 "$child" 2>/dev/null || break
    sleep .1
  done
  if kill -0 "$child" 2>/dev/null; then kill -KILL "$child"; echo 'shutdown timed out' >&2; exit 1; fi
  wait "$child"; child=
  printf '%s: HTTP 200, %s, clean exit; PATH=/missing; cwd=/; symlinked launcher\n' "$mode" "$expected"
done
