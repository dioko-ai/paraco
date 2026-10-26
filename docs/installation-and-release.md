# Installation and release status

A public Paraco installer and Homebrew package are not available. The repository
contains unpublished bundle, verification, and local installer tooling; passing
synthetic tests does not establish a releasable artifact or supported platform.
See [release artifacts](release-artifacts.md) for the implemented contract and
[workerd migration limits](workerd-migration.md) for runtime constraints.

## Current runtime and bundle

Paraco is a Rust host for workerd `2026-10-26`. Release bundles are designed to
include `bin/paraco`, `libexec/paraco/workerd`, native esbuild `0.28.2` at
`libexec/paraco/esbuild`, redistribution notices for all components, and bundle
metadata with SHA-256 checksums covered by a detached archive signature. Source development uses `npm ci` and places
`node_modules/.bin` on `PATH`; end users of a completed bundle need no Node.js,
Deno, npm, or Rust installation.

The local build command accepts explicit executable and notice inputs:

```sh
scripts/build-bundle.sh \
  --workerd "$WORKERD" --esbuild "$ESBUILD" \
  --workerd-notice "$WORKERD_NOTICE" --compiler-notice "$ESBUILD_NOTICE" \
  --third-party-notice "$RUST_NOTICES" --target "$TARGET" --output "$OUTPUT"
```

This produces an unsigned local archive. `scripts/install-archive.sh` can install
an archive when its checksum is supplied as a trust input. `verify-release.sh`
and `install-release.sh` support detached signature checks with a separately
trusted public key, but this does not make the tooling a published installer.

## Release gates

Before publication, retain native evidence for each advertised platform and
architecture. This includes clean-machine installation, HTTP behavior, shutdown,
upgrade and recovery, dependency review, complete redistribution notices,
source provenance, publisher key custody, and platform signing. macOS secure
containment is not implemented, Linux cgroup memory quotas are not implemented,
and prepared-artifact compatibility must be verified against final bundles.
Successful synthetic archive tests and configured CI matrices do not close these
gates. Do not claim deployed Cloudflare compatibility.

The local installer activates immutable version directories behind an owned
channel pointer, preserves prior versions for rollback, and preserves user data
on uninstall. It does not register a service or guarantee health-checked rollback.
Package recipe generation is review-only and unpublished. The macOS verification
script checks signing for all bundled executables and the final notarized package;
no signing result is claimed here.

## Planned distribution

A manual archive and a user-owned shell installer are possible first channels,
followed by a project Homebrew tap and platform packages after their release gates
pass. No URLs, tap owner, or install command are reserved here. Any installer
should verify authenticity before activation, stage before replacing a working
version, avoid `sudo`, and preserve app data, configuration, and credentials.
Removing the program must not silently remove an opt-in user service or user data.

Service setup and app execution are separate. Linux app execution currently
requires `/usr/bin/bwrap` with unprivileged user and network namespaces. macOS
fails closed for normal execution until secure containment is available; the
explicit `PARACO_ALLOW_UNSANDBOXED=1` override is for trusted development code.
See [the runtime migration notes](workerd-migration.md) for the containment
boundary and remaining limits.
