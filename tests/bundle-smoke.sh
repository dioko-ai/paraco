#!/usr/bin/env bash
# Exercises the one supported portable distribution contract end to end.
set -euo pipefail
usage() { echo "usage: $0 --workerd PATH --workerd-notice PATH --esbuild PATH --compiler-notice PATH --third-party-notice PATH --target TARGET [--output DIR]" >&2; exit 2; }
workerd= workerd_notice= esbuild= compiler_notice= third_party_notice= target= output=
while (($#)); do case $1 in
  --workerd) workerd=${2:?}; shift 2;; --workerd-notice) workerd_notice=${2:?}; shift 2;;
  --esbuild) esbuild=${2:?}; shift 2;; --compiler-notice) compiler_notice=${2:?}; shift 2;;
  --third-party-notice) third_party_notice=${2:?}; shift 2;; --target) target=${2:?}; shift 2;; --output) output=${2:?}; shift 2;; *) usage;;
esac; done
[[ -x $workerd && -f $workerd_notice && -x $esbuild && -f $compiler_notice && -f $third_party_notice && -n $target ]] || usage
repo_root=$PWD
node_bin=$(command -v node)
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
output=${output:-$base/build}
scripts/build-bundle.sh --workerd "$workerd" --workerd-notice "$workerd_notice" --esbuild "$esbuild" --compiler-notice "$compiler_notice" --third-party-notice "$third_party_notice" --target "$target" --output "$output"
archive=$(find "$output" -name '*.tar.gz' -print -quit)
checksum=$(if command -v sha256sum >/dev/null; then sha256sum "$archive"; else shasum -a 256 "$archive"; fi | awk '{print $1}')
scripts/install-archive.sh install --archive "$archive" --sha256 "$checksum" --version "$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -1)" --prefix "$base/prefix"
launcher="$base/prefix/channels/stable/bin/paraco"
[[ -x $launcher ]]
# Run outside the source tree with an intentionally broken system workerd.
# Success proves the installed private runtime is used.
app="$PWD/examples/hello"
mkdir "$base/foreign-path"
printf '#!/bin/sh\necho unexpected system workerd >&2\nexit 99\n' > "$base/foreign-path/workerd"
chmod +x "$base/foreign-path/workerd"
unset PARACO_WORKERD PARACO_WORKERD_VERSION PARACO_ESBUILD
export PATH="$base/foreign-path:$PATH"
cd "$base"
port=$((20000 + RANDOM % 20000))
"$launcher" --data-dir "$base" --log-dir "$base/logs" run "$app" --port "$port" >"$base/stdout" 2>"$base/stderr" & child=$!
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

# The installed scheduler must compile and execute tasks with no developer tools on PATH.
PARACO_BIN="$launcher" PARACO_TEST_EMPTY_PATH=1 "$node_bin" "$repo_root/tests/cron-runtime.cjs"
