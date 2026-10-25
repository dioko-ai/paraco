#!/usr/bin/env bash
# Authenticated release acquisition; supplied public key is never downloaded.
set -euo pipefail
usage(){ echo "usage: $0 (--archive FILE --signature FILE | --url HTTPS_URL --signature-url HTTPS_URL) --public-key PEM --sha256 HEX --version VERSION --target TARGET --prefix DIR [--channel stable]" >&2; exit 2; }
archive= signature= url= signature_url= public_key= checksum= version= target= prefix= channel=stable
while (($#)); do case $1 in
 --archive) archive=${2:?}; shift 2;; --signature) signature=${2:?}; shift 2;;
 --url) url=${2:?}; shift 2;; --signature-url) signature_url=${2:?}; shift 2;;
 --public-key) public_key=${2:?}; shift 2;; --sha256) checksum=${2:?}; shift 2;;
 --version) version=${2:?}; shift 2;; --target) target=${2:?}; shift 2;;
 --prefix) prefix=${2:?}; shift 2;; --channel) channel=${2:?}; shift 2;; *) usage;; esac; done
[[ -f $public_key && -n $checksum && -n $version && -n $target && $prefix == /* ]] || usage
base=$(mktemp -d); trap 'rm -rf "$base"' EXIT
if [[ -n $url || -n $signature_url ]]; then
 [[ -z $archive && -z $signature && $url == https://* && $signature_url == https://* ]] || usage
 curl --fail --silent --show-error --location --proto '=https' --proto-redir '=https' --connect-timeout 15 --max-time 600 --max-filesize 1100000000 --output "$base/archive.tar.gz" "$url"
 curl --fail --silent --show-error --location --proto '=https' --proto-redir '=https' --connect-timeout 15 --max-time 60 --max-filesize 1048576 --output "$base/signature" "$signature_url"
else
 [[ -f $archive && -f $signature ]] || usage
 [[ $(wc -c < "$archive") -le 1100000000 && $(wc -c < "$signature") -le 1048576 ]] || { echo 'release input size rejected' >&2; exit 1; }
 cp "$archive" "$base/archive.tar.gz"; cp "$signature" "$base/signature"
fi
scripts_dir=$(cd "$(dirname "$0")" && pwd)
"$scripts_dir/verify-release.sh" --archive "$base/archive.tar.gz" --signature "$base/signature" --public-key "$public_key" --sha256 "$checksum" --version "$version" --target "$target"
"$scripts_dir/install-archive.sh" install --archive "$base/archive.tar.gz" --sha256 "$checksum" --version "$version" --prefix "$prefix" --channel "$channel"
