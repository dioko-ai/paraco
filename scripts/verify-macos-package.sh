#!/usr/bin/env bash
# Fails closed: signing/notarization cannot be inferred from recipe generation.
set -euo pipefail
usage(){ echo 'usage: verify-macos-package.sh --bundle DIR --package PKG --identity ID --notary-profile PROFILE' >&2; exit 2; }
bundle= pkg= identity= profile=
while (($#)); do case $1 in --bundle)bundle=$2;shift 2;;--package)pkg=$2;shift 2;;--identity)identity=$2;shift 2;;--notary-profile)profile=$2;shift 2;;*)usage;;esac;done
[[ $(uname) == Darwin && -d $bundle && -f $pkg && -n $identity && -n $profile ]] || { echo 'macOS, final package, signing identity and notarization profile are required' >&2; exit 1; }
for f in "$bundle/bin/paraco" "$bundle/libexec/paraco/deno"; do codesign --verify --strict --verbose=2 "$f"; codesign -dvv "$f" 2>&1 | grep -F "Authority=$identity" >/dev/null || { echo "unexpected signing authority: $f" >&2; exit 1; }; done
pkgutil --check-signature "$pkg"
xcrun notarytool submit "$pkg" --keychain-profile "$profile" --wait
xcrun stapler validate "$pkg"
echo 'macOS binaries and final package verified'
