#!/bin/sh
# Produces an unsigned local bundle. Caller supplies trusted workerd input.
set -eu
usage() { echo "usage: $0 --workerd PATH --esbuild PATH --workerd-notice PATH --compiler-notice PATH --third-party-notice PATH --target TARGET --output DIR" >&2; exit 2; }
WORKERD= ESBUILD= WORKERD_NOTICE= COMPILER_NOTICE= THIRD_PARTY_NOTICE= TARGET= OUTPUT=
while [ "$#" -gt 0 ]; do case "$1" in --workerd) WORKERD=${2:-}; shift 2;; --esbuild) ESBUILD=${2:-}; shift 2;; --workerd-notice) WORKERD_NOTICE=${2:-}; shift 2;; --compiler-notice) COMPILER_NOTICE=${2:-}; shift 2;; --third-party-notice) THIRD_PARTY_NOTICE=${2:-}; shift 2;; --target) TARGET=${2:-}; shift 2;; --output) OUTPUT=${2:-}; shift 2;; *) usage;; esac; done
[ -n "$WORKERD" ] && [ -n "$ESBUILD" ] && [ -n "$WORKERD_NOTICE" ] && [ -n "$COMPILER_NOTICE" ] && [ -n "$THIRD_PARTY_NOTICE" ] && [ -n "$TARGET" ] && [ -n "$OUTPUT" ] || usage
[ -x "$WORKERD" ] || { echo "trusted workerd input is not executable" >&2; exit 1; }
[ -x "$ESBUILD" ] || { echo "trusted esbuild input is not executable" >&2; exit 1; }
[ -s "$WORKERD_NOTICE" ] && [ -s "$COMPILER_NOTICE" ] && [ -s "$THIRD_PARTY_NOTICE" ] || { echo "complete workerd, esbuild, and Rust third-party notices are required" >&2; exit 1; }
WORKERD_VERSION=$("$WORKERD" --version | sed -n '1s/^workerd \([0-9][0-9-]*\).*/\1/p')
printf '%s' "$WORKERD_VERSION" | grep -Eq '^[0-9]{4}-[0-9]{2}-[0-9]{2}$' || { echo "invalid workerd version" >&2; exit 1; }
printf '%s' "$TARGET" | grep -Eq '^[A-Za-z0-9][A-Za-z0-9._-]*$' || { echo "invalid target" >&2; exit 1; }
hash() { if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1"; else shasum -a 256 "$1"; fi; }
REVISION=$(git rev-parse --verify HEAD)
SOURCE_DIRTY=false
[ -z "$(git status --porcelain)" ] || SOURCE_DIRTY=true
VERSION=$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -1)
[ -n "$VERSION" ] && [ ! -e "$OUTPUT" ] || { echo "missing version or output already exists" >&2; exit 1; }
cargo build --locked --release --target "$TARGET"
BIN="${CARGO_TARGET_DIR:-target}/$TARGET/release/paraco"; [ -x "$BIN" ] || { echo "missing release executable" >&2; exit 1; }
STAGE=$(mktemp -d); trap 'rm -rf "$STAGE"' EXIT
ROOT="$STAGE/paraco-$VERSION-$TARGET"; mkdir -p "$ROOT/bin" "$ROOT/libexec/paraco" "$ROOT/notices"
cp "$BIN" "$ROOT/bin/paraco"; cp "$WORKERD" "$ROOT/libexec/paraco/workerd"; cp "$ESBUILD" "$ROOT/libexec/paraco/esbuild"; cp LICENSE "$ROOT/notices/LICENSE"; cp "$WORKERD_NOTICE" "$ROOT/notices/WORKERD-NOTICE"; cp "$COMPILER_NOTICE" "$ROOT/notices/ESBUILD-NOTICE"; cp "$THIRD_PARTY_NOTICE" "$ROOT/notices/RUST-THIRD-PARTY-NOTICES"
PARACO_SHA=$(hash "$ROOT/bin/paraco" | awk '{print $1}'); WORKERD_SHA=$(hash "$ROOT/libexec/paraco/workerd" | awk '{print $1}'); ESBUILD_SHA=$(hash "$ROOT/libexec/paraco/esbuild" | awk '{print $1}'); LICENSE_SHA=$(hash "$ROOT/notices/LICENSE" | awk '{print $1}'); WORKERD_NOTICE_SHA=$(hash "$ROOT/notices/WORKERD-NOTICE" | awk '{print $1}'); COMPILER_NOTICE_SHA=$(hash "$ROOT/notices/ESBUILD-NOTICE" | awk '{print $1}'); RUST_NOTICE_SHA=$(hash "$ROOT/notices/RUST-THIRD-PARTY-NOTICES" | awk '{print $1}')
printf '{\n  "paracoVersion": "%s",\n  "workerdVersion": "%s",\n  "target": "%s",\n  "sourceRevision": "%s",\n  "sourceDirty": %s,\n  "notices": ["notices/LICENSE", "notices/WORKERD-NOTICE", "notices/ESBUILD-NOTICE", "notices/RUST-THIRD-PARTY-NOTICES"],\n  "checksums": {"bin/paraco": "%s", "libexec/paraco/workerd": "%s", "libexec/paraco/esbuild": "%s", "notices/LICENSE": "%s", "notices/WORKERD-NOTICE": "%s", "notices/ESBUILD-NOTICE": "%s", "notices/RUST-THIRD-PARTY-NOTICES": "%s"},\n  "trustedInputProvenance": "caller supplied --workerd and --esbuild; signing and publisher authenticity are not verified by this tool"\n}\n' "$VERSION" "$WORKERD_VERSION" "$TARGET" "$REVISION" "$SOURCE_DIRTY" "$PARACO_SHA" "$WORKERD_SHA" "$ESBUILD_SHA" "$LICENSE_SHA" "$WORKERD_NOTICE_SHA" "$COMPILER_NOTICE_SHA" "$RUST_NOTICE_SHA" > "$ROOT/bundle.json"
(cd "$STAGE" && COPYFILE_DISABLE=1 tar --format ustar -czf "paraco-$VERSION-$TARGET.tar.gz" "paraco-$VERSION-$TARGET")
mkdir "$OUTPUT"; mv "$STAGE/paraco-$VERSION-$TARGET.tar.gz" "$OUTPUT/"; (cd "$OUTPUT" && hash "paraco-$VERSION-$TARGET.tar.gz" > SHA256SUMS)
echo "unsigned bundle created at $OUTPUT; signing, provenance and native smoke tests remain publication gates"
