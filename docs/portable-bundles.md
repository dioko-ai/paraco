# Portable bundles (local build tooling)

`scripts/build-bundle.sh --deno /absolute/path/to/deno --deno-notice NOTICE --third-party-notice NOTICE --target TARGET --output DIR`
creates an **unsigned local** archive only after accepting an explicit executable
Deno input. The archive contains `bin/paraco`, `libexec/paraco/deno`, notices,
`bundle.json` (Paraco/Deno versions, target, source revision, notices,
checksums, and provenance),
and `SHA256SUMS`. It refuses to build without complete supplied Deno and Rust
third-party redistribution notices.

A bundle archive has the root name `paraco-<version>-<target>`; the installer
accepts that layout and activates it under its versioned release directory. A
packaged executable resolves its real path through launcher symlinks and uses
only the private runtime beside that immutable release. It does not consult PATH
or CWD. `PARACO_DENO` is a development override and must name an executable
whose exact parsed version equals bundle metadata (or explicit
`PARACO_DENO_VERSION` in a source build). A running process resolves this once
at launch, so later activation changes cannot swap its runtime.

Checksums detect corruption; they do not authenticate a publisher. Trusted
release provenance/signatures, dependency vulnerability review, platform signing
and notarization, clean-machine/offline HTTP/shutdown smoke tests, and native
support evidence are publication blockers. This repository has not executed or
claimed those gates.
