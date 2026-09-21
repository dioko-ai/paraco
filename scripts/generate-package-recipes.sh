#!/usr/bin/env bash
# Generates reviewable, unpublished recipes. It neither downloads nor installs.
set -euo pipefail
usage(){ echo 'usage: generate-package-recipes.sh --bundle FILE --version V --url HTTPS_URL --sha256 HEX --target TARGET --output DIR' >&2; exit 2; }
bundle= version= url= sha= target= output=
while (($#)); do case $1 in --bundle)bundle=$2;shift 2;;--version) version=$2;shift 2;;--url)url=$2;shift 2;;--sha256)sha=$2;shift 2;;--target)target=$2;shift 2;;--output)output=$2;shift 2;;*)usage;;esac;done
[[ $version =~ ^[0-9][A-Za-z0-9._-]*$ && $url =~ ^https:// && $sha =~ ^[A-Fa-f0-9]{64}$ && $target =~ ^[A-Za-z0-9._-]+$ && -n $output ]] || { echo 'explicit version, HTTPS release URL, SHA-256, target, and output are required' >&2; exit 2; }
[[ ! -e $output ]] || { echo 'output must not already exist' >&2; exit 1; }
[[ -f $bundle ]] || { echo 'a locally verified bundle is required' >&2; exit 1; }
actual=$(sha256sum "$bundle" | awk '{print $1}'); [[ $actual == "$sha" ]] || { echo 'bundle checksum does not match recipe metadata' >&2; exit 1; }
mkdir -p "$output"
cat > "$output/paraco.rb" <<RUBY
# Generated review artifact only: do not publish this formula until release gates pass.
class Paraco < Formula
  desc "Local application host with a bundled private Deno runtime"
  homepage "https://github.com/dioko-ai/paraco"
  url "$url"
  version "$version"
  sha256 "$sha"
  def install
    libexec.install Dir["*"]
    bin.write_exec_script libexec/"bin/paraco"
  end
  def caveats; "Run \`paraco service setup\` explicitly to opt into a user service."; end
end
RUBY
mkdir -p "$output/linux-deb/DEBIAN" "$output/macos-pkg"
cat > "$output/linux-deb/DEBIAN/control" <<DEB
Package: paraco
Version: $version
Architecture: $target
Description: Paraco bundle (private bundled Deno; service opt-in)
DEB
cat > "$output/linux-deb/DEBIAN/postrm" <<'SH'
#!/bin/sh
# Deliberately preserve user state/configuration/credentials and do not remove
# or register services. Users opt in/out through Paraco service commands.
exit 0
SH
chmod 755 "$output/linux-deb/DEBIAN/postrm"
cat > "$output/macos-pkg/Distribution.xml" <<XML
<installer-gui-script minSpecVersion="1"><title>Paraco $version</title><options customize="never"/></installer-gui-script>
XML
cat > "$output/package-manifest.json" <<JSON
{"version":"$version","target":"$target","artifactUrl":"$url","sha256":"$sha","runtime":"bundled private Deno; system Deno is unsupported","service":"opt-in only","removal":"preserve user state/configuration/credentials","channel":"unsigned generated review artifact; unpublished"}
JSON
cat > "$output/RELEASE-GATES.md" <<'MD'
# Release acceptance gates

This directory is generated tooling, not a release, tap, package-manager action,
or support claim. Before publication, retain a reviewed trusted-provenance and
artifact-authenticity outcome, dependency-vulnerability review outcome, native
clean-machine/package evidence, and platform support evidence. macOS requires
successful verification of **both** bundled executables (`bin/paraco` and
`libexec/paraco/deno`) and the final signed/notarized package. Missing native
tools or credentials is a blocker, never a pass. Package removal must preserve
user state/configuration/credentials; service setup remains explicit opt-in.
MD
echo "generated unpublished Homebrew, Debian-layout, and macOS-package-layout recipes in $output"
