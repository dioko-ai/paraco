#!/usr/bin/env bash
# Exercises the one supported portable distribution contract end to end.
set -euo pipefail
usage() { echo "usage: $0 --deno PATH --deno-notice PATH --third-party-notice PATH --target TARGET" >&2; exit 2; }
deno= deno_notice= third_party_notice= target=
while (($#)); do case $1 in
  --deno) deno=${2:?}; shift 2;; --deno-notice) deno_notice=${2:?}; shift 2;;
  --third-party-notice) third_party_notice=${2:?}; shift 2;; --target) target=${2:?}; shift 2;; *) usage;;
esac; done
[[ -x $deno && -f $deno_notice && -f $third_party_notice && -n $target ]] || usage
base=$(mktemp -d); child=
stop_child() {
  [[ -n $child ]] && kill -0 "$child" 2>/dev/null || return 0
  kill -INT "$child" 2>/dev/null || true
  deadline=$((SECONDS + 9))
  while kill -0 "$child" 2>/dev/null && (( SECONDS < deadline )); do sleep 0.05; done
  kill -0 "$child" 2>/dev/null || return 0
  kill -TERM "$child" 2>/dev/null || true
  deadline=$((SECONDS + 2))
  while kill -0 "$child" 2>/dev/null && (( SECONDS < deadline )); do sleep 0.05; done
  kill -0 "$child" 2>/dev/null && kill -KILL "$child" 2>/dev/null || true
}
cleanup() {
  stop_child; wait "$child" 2>/dev/null || true
  rm -rf "$base"
}
trap cleanup EXIT
scripts/build-bundle.sh --deno "$deno" --deno-notice "$deno_notice" --third-party-notice "$third_party_notice" --target "$target" --output "$base/build"
archive=$(find "$base/build" -name '*.tar.gz' -print -quit)
checksum=$(sha256sum "$archive" | awk '{print $1}')
scripts/install-archive.sh install --archive "$archive" --sha256 "$checksum" --version "$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -1)" --prefix "$base/prefix"
launcher="$base/prefix/channels/stable/bin/paraco"
[[ -x $launcher ]]
port=$((20000 + RANDOM % 20000))
"$launcher" --log-dir "$base/logs" run examples/hello --port "$port" >"$base/stdout" 2>"$base/stderr" & child=$!
deadline=$((SECONDS + 15))
until grep -q 'listening on http://' "$base/stdout"; do
  kill -0 "$child" 2>/dev/null || { cat "$base/stderr" >&2; exit 1; }
  (( SECONDS < deadline )) || { echo 'installed bundle did not become ready' >&2; exit 1; }
  sleep 0.05
done
response=$(curl --fail --silent --max-time 3 "http://127.0.0.1:$port/")
[[ $response == *'Hello from Paraco'* ]]
stop_child
if kill -0 "$child" 2>/dev/null; then echo 'installed bundle did not exit' >&2; exit 1; fi
wait "$child"; child=
