#!/usr/bin/env bash
# Offline publisher-authenticity and release-content gate. Does not execute payloads.
set -euo pipefail
usage(){ echo "usage: $0 --archive FILE --signature FILE --public-key PEM --sha256 HEX --version VERSION [--target TARGET]" >&2; exit 2; }
archive= signature= public_key= checksum= version= target=
while (($#)); do case $1 in
 --archive) archive=${2:?}; shift 2;; --signature) signature=${2:?}; shift 2;;
 --public-key) public_key=${2:?}; shift 2;; --sha256) checksum=${2:?}; shift 2;;
 --version) version=${2:?}; shift 2;; --target) target=${2:?}; shift 2;; *) usage;; esac; done
[[ -f $archive && -f $signature && -f $public_key && $version =~ ^[A-Za-z0-9][A-Za-z0-9._-]*$ && $checksum =~ ^[a-f0-9]{64}$ ]] || usage
hash(){ if command -v sha256sum >/dev/null; then sha256sum "$1"; else shasum -a 256 "$1"; fi | awk '{print $1}'; }
[[ $(wc -c < "$archive") -le 1100000000 && $(wc -c < "$signature") -le 1048576 && $(wc -c < "$public_key") -le 1048576 ]] || { echo 'compressed archive size rejected' >&2; exit 1; }
base=$(mktemp -d); trap 'rm -rf "$base"' EXIT
cp "$archive" "$base/archive.tar.gz"
[[ $(hash "$base/archive.tar.gz") == "$checksum" ]] || { echo 'release checksum mismatch' >&2; exit 1; }
# The public key must be supplied through an independently trusted channel.
# RSA/ECDSA SHA-256 detached signatures, made with openssl dgst -sha256 -sign.
openssl dgst -sha256 -verify "$public_key" -signature "$signature" "$base/archive.tar.gz" >/dev/null
root=$(perl "$(dirname "$0")/validate-archive.pl" "$base/archive.tar.gz" "$version")
mkdir "$base/content"
tar -xzf "$base/archive.tar.gz" -C "$base/content" --no-same-owner --no-same-permissions
perl - "$base/content/$root" "$root" "$version" "$target" "$checksum" "$(hash "$public_key")" <<'PERL'
use strict;
use warnings;
use JSON::PP;
use Digest::SHA;
my ($dir, $root, $version, $target, $checksum, $public_key_hash) = @ARGV;
sub bytes { my ($p) = @_; -f "$dir/$p" && -s "$dir/$p" <= 1048576 or die "missing or oversized metadata/notice: $p\n"; open my $f, '<', "$dir/$p" or die "missing release file: $p\n"; binmode $f; local $/; return <$f>; }
my $raw = bytes('bundle.json');
length($raw) <= 65536 or die "release metadata too large\n";
my $m = decode_json($raw);
ref($m) eq 'HASH' or die "invalid release metadata\n";
($m->{paracoVersion} // '') eq $version or die "release version mismatch\n";
my $actual = $m->{target} // '';
$actual =~ /\A(?:x86_64|aarch64)-(?:unknown-linux-gnu|apple-darwin)\z/ or die "unsupported release target\n";
(!$target || $actual eq $target) or die "release target mismatch\n";
$root eq "paraco-$version-$actual" or die "release root/metadata mismatch\n";
($m->{sourceRevision} // '') =~ /\A(?:[a-f0-9]{40}|[a-f0-9]{64})\z/ or die "invalid source revision\n";
exists($m->{sourceDirty}) && ref($m->{sourceDirty}) eq 'JSON::PP::Boolean' && !$m->{sourceDirty} or die "release source is dirty or cleanliness is unrecorded\n";
($m->{workerdVersion} // '') =~ /\A[0-9]{4}-[0-9]{2}-[0-9]{2}\z/ or die "invalid runtime version\n";
my @notices = ('notices/LICENSE', 'notices/WORKERD-NOTICE', 'notices/ESBUILD-NOTICE', 'notices/RUST-THIRD-PARTY-NOTICES');
ref($m->{notices}) eq 'ARRAY' && join("\0",sort @{$m->{notices}}) eq join("\0",sort @notices) or die "required notices not declared\n";
ref($m->{checksums}) eq 'HASH' or die "missing file checksums\n";
my @required = ('bin/paraco', 'libexec/paraco/workerd', 'libexec/paraco/esbuild', @notices);
for my $p (@required) {
    -f "$dir/$p" && -s "$dir/$p" or die "missing or empty release file: $p\n";
    open my $f, '<', "$dir/$p" or die "cannot hash release file: $p\n";
    binmode $f;
    my $digest = Digest::SHA->new(256)->addfile($f)->hexdigest;
    close $f;
    ($m->{checksums}{$p} // '') eq $digest or die "release file checksum mismatch: $p\n";
    if ($p =~ m{\Anotices/}) {
        my $data = bytes($p);
        $data !~ /fixture|non[- ]distributable|not (?:a |for )?distribut(?:able|ion)|incomplete (?:redistribution |third.party )?notices|placeholder/i or die "non-distributable notice rejected: $p\n";
    }
}
for my $p (keys %{$m->{checksums}}) {
    grep($_ eq $p, @required) or die "unexpected checksum member: $p\n";
}
-x "$dir/bin/paraco" && -x "$dir/libexec/paraco/workerd" && -x "$dir/libexec/paraco/esbuild" or die "release executables lack execute permission\n";
print encode_json({verified=>JSON::PP::true, sha256=>$checksum, publicKeyPemSha256=>$public_key_hash, paracoVersion=>$version, target=>$actual, sourceRevision=>$m->{sourceRevision}, workerdVersion=>$m->{workerdVersion}, sourceDirty=>JSON::PP::false, signatureAlgorithm=>'OpenSSL SHA-256 detached signature', scope=>'publisher signature and signed metadata; native execution and notice completeness require separate review'}), "\n";
PERL
