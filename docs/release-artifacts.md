# Authenticated release artifacts

The repository can build, sign, verify and install an archive without a release
host or account. No release domain, publisher key or signing credentials are
configured by these scripts. A release owner must choose those inputs and obtain
native validation evidence before publishing. The scripts do not publish.

Keep `install-release.sh`, `verify-release.sh`, `install-archive.sh` and
`validate-archive.pl` together in the same trusted directory. Obtain these tools
and the publisher's PEM public key through an independently trusted channel;
never fetch an untrusted key alongside an archive and treat it as authenticated.
The installer requires Bash (including macOS Bash 3.2), Perl with its core
JSON::PP/Digest::SHA modules, gzip, tar, OpenSSL, and either shasum or sha256sum.
Remote acquisition also requires curl. Python 3 is used only by test fixtures.

## Prepare and sign a release

Build from a clean, reviewed source checkout, using the tested Deno executable
and complete, reviewed Deno/Rust redistribution notices:

```sh
scripts/build-bundle.sh --deno "$DENO" --deno-notice "$DENO_NOTICE" \
  --third-party-notice "$RUST_NOTICES" --target "$TARGET" --output "$OUTPUT"
```

`$ARCHIVE` identifies the resulting versioned archive. Sign its exact compressed
bytes with the release owner's existing RSA or ECDSA private key:

```sh
openssl dgst -sha256 -sign "$PRIVATE_KEY" -out "$ARCHIVE.sig" "$ARCHIVE"
```

Private keys remain outside the repository and distribution artifacts. The
verifier accepts OpenSSL SHA-256 detached RSA/ECDSA signatures; it does not accept
Ed25519, GPG or Sigstore formats. The corresponding trusted public key can be
exported by the release owner using `openssl pkey -in "$PRIVATE_KEY" -pubout`.
Publish the archive, detached signature and SHA256SUMS only after the remaining
release gates pass. The checksum detects accidental byte changes; the signature
binds those bytes to the independently trusted publisher key.

## Verify or install

For an offline authenticity and content check:

```sh
scripts/verify-release.sh --archive "$ARCHIVE" --signature "$ARCHIVE.sig" \
  --public-key "$TRUSTED_PUBLIC_KEY" --sha256 "$SHA256" \
  --version "$VERSION" --target "$TARGET"
```

Verification checks the supplied SHA-256, detached signature, strict bounded
ustar layout, version/target agreement, source revision, recorded clean source,
all required executable/notice checksums, executable permissions and notice
presence. Missing cleanliness metadata and obvious fixture, placeholder,
incomplete or non-distributable notice labels fail the release gate. This
label check cannot establish legal completeness; notice review remains a
release-owner responsibility. It emits a JSON record containing the verified
archive checksum, supplied public-key PEM fingerprint and signed source, target and Deno-version metadata. It does
not run the binaries or establish their native compatibility.

For HTTPS acquisition and activation, provide concrete archive and signature
URLs, an independently trusted local public key, checksum, version and target:

```sh
scripts/install-release.sh --url "$ARCHIVE_HTTPS_URL" \
  --signature-url "$SIGNATURE_HTTPS_URL" --public-key "$TRUSTED_PUBLIC_KEY" \
  --sha256 "$SHA256" --version "$VERSION" --target "$TARGET" --prefix "$PREFIX"
```

Only HTTPS is allowed, including redirects; ordinary TLS verification remains
enabled. The public key is never downloaded. Both downloads go to temporary
storage. The signature and content gate completes before the existing staged
installer can create or replace a channel pointer. Offline installation uses
`--archive "$ARCHIVE" --signature "$ARCHIVE.sig"` instead of the URL arguments.
Versions, targets and checksums must come from the release selection the user
intends to trust, preventing substitution with another correctly signed version.

Use `install-archive.sh rollback --version "$OLDER_VERSION" --prefix "$PREFIX"`
for retained local releases; it preserves state and refuses unsupported state
format downgrade. It is a pointer rollback, not automatic service-health or
schema recovery. Uninstall preserves data, credentials and retained releases.

## Remaining publication gates

Run the native validation workflow and preserve its logs for each advertised
platform/architecture. Its notice fixtures are explicitly non-distributable and
will be rejected by this release verifier. Replace them with reviewed complete
notices for real release artifacts. Test the final signed payload on the chosen
minimum OS/libc baselines, without a separate Deno installation, and confirm HTTP,
shutdown, upgrade and recovery behavior. Native builds on newer CI runners do
not establish older OS support.

Publisher key custody/distribution, real hosting, dependency review, reviewed
license notices, native platform evidence and platform-specific signing remain
external release-owner gates. This detached-signature flow does not replace
macOS code signing, notarization or final package verification. No credentials,
host ownership, native results or published releases are inferred by the tooling.

Run `bash tests/release-verification.sh` to exercise ephemeral local signing,
metadata and artifact tampering, malformed signatures, dirty/fixture rejection,
and preservation of an already active installation. These are synthetic local
fixtures and do not constitute a signed Paraco release or native execution proof.
