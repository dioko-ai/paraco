#!/usr/bin/env bash
# Deliberately local/offline installer for an already trusted Paraco archive.
# A checksum proves bytes, not publisher identity.
set -euo pipefail
usage(){ echo "usage: $0 install|rollback|uninstall --archive FILE --sha256 HEX --version VERSION --prefix DIR [--channel stable]" >&2; exit 2; }
cmd=${1:-}; shift || true
archive= checksum= version= prefix= channel=stable
while (($#)); do case $1 in
  --archive) archive=${2:?}; shift 2;; --sha256) checksum=${2:?}; shift 2;;
  --version) version=${2:?}; shift 2;; --prefix) prefix=${2:?}; shift 2;;
  --channel) channel=${2:?}; shift 2;; *) usage;; esac; done
[[ $cmd == install || $cmd == rollback || $cmd == uninstall ]] || usage
[[ $prefix = /* && $version =~ ^[A-Za-z0-9._-]+$ && $channel =~ ^[A-Za-z0-9._-]+$ ]] || { echo 'absolute prefix and safe version/channel required' >&2; exit 2; }
root="$prefix"; releases="$root/releases"; active="$root/channels/$channel"; lock="$root/.installer.lock"
owner="$root/channels/.paraco-$channel.owner"
mkdir -p "$releases" "$(dirname "$active")"
if ! mkdir "$lock" 2>/dev/null; then echo 'another installer owns this prefix' >&2; exit 1; fi
trap 'rmdir "$lock"' EXIT
hash(){ if command -v sha256sum >/dev/null; then sha256sum "$1" | awk '{print $1}'; else shasum -a 256 "$1" | awk '{print $1}'; fi; }
if [[ -e $active && (! -f $owner || $(cat "$owner") != paraco-channel-v1) ]]; then
  echo 'refusing foreign or unowned channel pointer' >&2; exit 1
fi
if [[ $cmd == uninstall ]]; then
  [[ ! -e "$root/state/running.pid" ]] || { echo 'refusing uninstall while runtime state indicates a running service; stop it first' >&2; exit 1; }
  rm -f "$active" # Data, state, credentials, and releases are intentionally retained.
  rm -f "$owner"
  echo 'channel removed; data/configuration/credentials/releases preserved'; exit 0
fi
if [[ $cmd == rollback ]]; then
  [[ -d "$releases/$version" && -x "$releases/$version/bin/paraco" ]] || { echo 'retained release is unavailable; rollback cannot recover it' >&2; exit 1; }
  # State migrations are intentionally not reversed: callers must recover or
  # explicitly re-prepare if an older binary cannot read current host state.
  [[ ! -e "$root/state/format" ]] || [[ $(cat "$root/state/format") == 1 ]] || { echo 'rollback refused: newer state format requires recovery' >&2; exit 1; }
  printf 'paraco-channel-v1\n' > "$owner"
  tmp="$active.new.$$"; ln -s "../releases/$version" "$tmp"; mv -Tf "$tmp" "$active"
  echo "rolled back $channel -> $version"; exit 0
fi
[[ -f $archive && $checksum =~ ^[A-Fa-f0-9]{64}$ ]] || { echo 'local archive and explicit SHA-256 are required trust inputs' >&2; exit 2; }
[[ $(hash "$archive") == "$checksum" ]] || { echo 'archive checksum mismatch' >&2; exit 1; }
# Reject unsafe member names before extraction. The builder owns the exact root
# name: paraco-<version>-<target>. The installer maps that immutable bundle to
# its versioned release slot.
mapfile -t members < <(tar -tzf "$archive")
((${#members[@]} > 0 && ${#members[@]} <= 10000)) || { echo 'archive entry count rejected' >&2; exit 1; }
declare -A seen=()
archive_root=
for p in "${members[@]}"; do
 [[ $p != /* && $p != *'..'* ]] || { echo "unsafe archive path: $p" >&2; exit 1; }
 top=${p%%/*}
 [[ $top =~ ^paraco-${version}-[A-Za-z0-9._-]+$ ]] || { echo "unsafe archive path: $p" >&2; exit 1; }
 [[ -z $archive_root || $archive_root == "$top" ]] || { echo "archive has multiple roots" >&2; exit 1; }; archive_root=$top
 [[ -z ${seen[$p]+x} ]] || { echo "duplicate archive path: $p" >&2; exit 1; }; seen[$p]=1
done
stage=$(mktemp -d "$root/.stage.XXXXXX"); trap 'rm -rf "$stage"; rmdir "$lock"' EXIT
tar -xzf "$archive" -C "$stage" --no-same-owner --no-same-permissions
[[ ! -L "$stage/$archive_root" ]] && ! find "$stage" -type l -o -type b -o -type c -o -type p -o -type s | grep -q . || { echo 'links or special files rejected' >&2; exit 1; }
[[ -x "$stage/$archive_root/bin/paraco" && -x "$stage/$archive_root/libexec/paraco/deno" && -f "$stage/$archive_root/bundle.json" ]] || { echo 'archive lacks the Paraco bundle contract' >&2; exit 1; }
[[ $(du -sk "$stage" | awk '{print $1}') -le 1048576 ]] || { echo 'extraction size rejected' >&2; exit 1; }
target="$releases/$version"
[[ ! -e $target ]] || { echo 'release version already exists (immutable)' >&2; exit 1; }
mv "$stage/$archive_root" "$target"
# Atomic pointer replacement. Old release remains for processes and rollback.
tmp="$active.new.$$"; ln -s "../releases/$version" "$tmp"; mv -Tf "$tmp" "$active"
printf 'paraco-channel-v1\n' > "$owner"
echo "activated $channel -> $version"
