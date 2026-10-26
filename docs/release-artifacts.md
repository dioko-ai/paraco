# Authenticated release artifacts

The repository can build, sign, verify, and locally install an archive without a
release host or account. No release domain, publisher key, or signing credentials
are configured by these scripts. They do not publish artifacts; Paraco has no
public installer yet.

Keep `install-release.sh`, `verify-release.sh`, `install-archive.sh`, and
`validate-archive.pl` together in a trusted directory. Obtain the scripts and the
publisher PEM public key through independently trusted channels. The installer
requires Bash (including macOS Bash 3.2), Perl with core JSON::PP/Digest::SHA,
gzip, tar, OpenSSL, and `shasum` or `sha256sum`; remote acquisition also needs
curl. Python is used only by test fixtures.

## Prepare and sign a release

Build from a clean reviewed checkout using workerd `2026-10-26`, native esbuild
`0.28.2`, and complete reviewed notices for workerd, esbuild, and Rust
dependencies:

```sh
scripts/build-bundle.sh \
  --workerd "$WORKERD" --esbuild "$ESBUILD" \
  --workerd-notice "$WORKERD_NOTICE" --compiler-notice "$ESBUILD_NOTICE" \
  --third-party-notice "$RUST_NOTICES" --target "$TARGET" --output "$OUTPUT"
```

The archive contains `bin/paraco`, `libexec/paraco/workerd`,
`libexec/paraco/esbuild`, required notices, `bundle.json`, and `SHA256SUMS`.
Metadata hashes all payload and notice files and records workerd's `YYYY-MM-DD`
version. Sign the exact compressed archive bytes with the release owner's RSA or
ECDSA private key:

```sh
openssl dgst -sha256 -sign "$PRIVATE_KEY" -out "$ARCHIVE.sig" "$ARCHIVE"
```

Private keys remain outside the repository and distribution artifacts. The
verifier accepts OpenSSL SHA-256 detached RSA/ECDSA signatures, not Ed25519, GPG,
or Sigstore. Export the corresponding PEM public key with
`openssl pkey -in "$PRIVATE_KEY" -pubout`. The checksum detects accidental byte
changes; the signature binds the bytes to the independently trusted publisher
key.

## Verify or install

For an offline authenticity and content check:

```sh
scripts/verify-release.sh --archive "$ARCHIVE" --signature "$ARCHIVE.sig" \
  --public-key "$TRUSTED_PUBLIC_KEY" --sha256 "$SHA256" \
  --version "$VERSION" --target "$TARGET"
```

Verification checks the supplied SHA-256, detached signature, bounded ustar
layout, version/target agreement, source revision and cleanliness, required
executables and notices, all file checksums, and executable permissions. Obvious
fixture and incomplete-notice labels are rejected; that check cannot establish
legal completeness. Verification does not execute the payload or establish
native compatibility.

For HTTPS acquisition and activation:

```sh
scripts/install-release.sh --url "$ARCHIVE_HTTPS_URL" \
  --signature-url "$SIGNATURE_HTTPS_URL" --public-key "$TRUSTED_PUBLIC_KEY" \
  --sha256 "$SHA256" --version "$VERSION" --target "$TARGET" --prefix "$PREFIX"
```

HTTPS and normal TLS verification are required, including for redirects. The
public key is not downloaded. Signature and content checks finish before the
staged installer can switch an owned channel pointer. Offline installation uses
`--archive "$ARCHIVE" --signature "$ARCHIVE.sig"`. The selected version, target,
and checksum must come from the release the user intends to trust.

`install-archive.sh rollback` switches to a retained version and refuses an
unsupported state-format downgrade. It does not perform service-health checks or
schema recovery. Uninstall preserves data, credentials, and retained releases.

## Remaining publication gates

The native validation workflow uses non-distributable notice fixtures. Replace
them with complete reviewed notices for real artifacts. Validate final signed
payloads on minimum OS/libc baselines, including HTTP, shutdown, upgrade and
recovery behavior. Native builds on newer CI runners do not establish older OS
support. Publisher key custody, dependency review, provenance, macOS signing and
notarization, clean-machine evidence, Linux cgroup memory quotas, and secure
macOS containment remain release gates. Do not claim deployed Cloudflare
compatibility.

Run `bash tests/release-verification.sh` for synthetic signature, metadata,
tampering, malformed signature, fixture rejection, and active-installation
preservation checks. These tests do not constitute a signed release or native
execution proof.
