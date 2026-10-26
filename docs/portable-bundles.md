# Portable bundles (local build tooling)

`scripts/build-bundle.sh --workerd /absolute/path/to/workerd --esbuild /absolute/path/to/esbuild --workerd-notice NOTICE --compiler-notice NOTICE --third-party-notice NOTICE --target TARGET --output DIR`
creates an **unsigned local** archive from explicit executable inputs. The
archive contains `bin/paraco`, private `libexec/paraco/workerd` and
`libexec/paraco/esbuild` executables, redistribution notices for workerd,
esbuild, and Rust dependencies, plus `bundle.json` with versions, target, source
revision, checksums, and provenance. The workerd version uses `YYYY-MM-DD`; the
compiler is esbuild 0.28.2. The output directory also contains the `SHA256SUMS` sidecar.

A bundle archive has the root name `paraco-<version>-<target>`; the local
installer accepts that layout and activates it under its versioned release
directory. The host uses the private runtimes beside the immutable release and
does not require Node.js or Deno from end users. This packaging is not a
published installer or a claim of deployed Cloudflare compatibility.

Checksums detect corruption; they do not authenticate a publisher. Trusted
release provenance/signatures, dependency vulnerability review, platform signing
and notarization, clean-machine/offline HTTP/shutdown smoke tests, Linux
cgroup-based memory quotas, macOS sandbox verification, and native support
evidence remain release gates. This repository has not executed or claimed all
those gates.
