# Approved app marketplace

## Design and rollout

The first marketplace is a reviewed YAML catalog in this repository. Membership
approves one app version and its exact SHA-256 package digest. A GitHub repository
or tag is provenance, not approval: publishing a release cannot silently change
what users install. Catalog maintainers review source, package contents, requested
capabilities, license and the pinned source commit before merging an entry.

The initial implementation provides a bundled catalog, listing and explicit
installation, a portable package format, and a complete Hello World repository
template. `install` automatically downloads the selected approved asset. It does
not run build hooks, start the app, edit hosting configuration, grant capabilities,
or silently upgrade existing installations. The catalog itself is a host trust
input; `--catalog` selects a local replacement explicitly. The bundled catalog is
updated by rebuilding Paraco, and works independently of the current directory.

Next steps, in order:

1. Protect catalog changes with required maintainer review and CI. Add more apps
   through reviewed entries, retaining approved older versions where appropriate.
2. Add an authenticated dashboard installation flow using this same validator,
   explicit capability grants, and durable deployment identities.
3. Design signed remote catalog refresh with expiry and rollback protection before
   allowing unattended catalog updates. HTTPS plus a checksum in an untrusted
   catalog is not an independent trust guarantee.
4. Add explicit upgrade/rollback transactions preserving deployment data and
   identity; define revocation behavior for already installed/running versions.
5. Expand runtime compatibility metadata and package limits based on real apps.
   The v1 catalog targets the current Paraco v1 application contract.

## Use

```sh
paraco marketplace list
paraco marketplace install hello-world --output ./installed/hello-world
paraco run ./installed/hello-world
```

For a checkout, prefix these commands with `cargo run --`. Installation creates a
new app directory suitable for `run` or an existing `serve` configuration's `path`.
Use `--version 0.1.0` when more than one approved version exists. There is no
implicit “latest.” Use `--catalog /trusted/catalog.yaml` to select a different
host-owned catalog. A previously downloaded package can be supplied with
`--package /path/hello-world.paraco.json`; all approval checks still apply.

The installed `paraco-install.json` records the approval and provenance. It is a
receipt, not a signature or a launch-time integrity guarantee. Installed sources
remain editable, like other local apps. For verified offline runtime artifacts,
use the existing `paraco prepare` and `paraco run-prepared` commands after install.
Source packages do not bundle Deno or dependency caches. Dependencies should be
vendored for reproducibility; installation does not resolve or execute imports.

## Catalog v1

See [the catalog](../marketplace/catalog.yaml) for the complete initial entry.
The top-level fields are `schemaVersion: 1` and `apps`. Each entry requires:

Editor schemas are provided for the [catalog](marketplace.schema.json) and
[package](app-package.schema.json). The CLI additionally enforces cross-field
identity, release URLs, path safety, uniqueness and byte-size limits.

| Field | Meaning |
| --- | --- |
| `name` | Canonical Paraco app slug; matches the runtime manifest |
| `title`, `description` | Human-readable catalog metadata |
| `repository` | HTTPS GitHub owner/repository URL |
| `version` | Three numeric segments; no leading zeros, at most nine digits each |
| `commit` | Lowercase 40-character source Git commit; reviewed provenance |
| `package` | `REPOSITORY/releases/download/vVERSION/NAME.paraco.json` |
| `sha256` | Lowercase SHA-256 of the entire downloaded file |
| `capabilities` | Exact set from `paraco.json`; requests never grant permission |

Unknown fields, duplicate app/version pairs and unsupported capabilities fail
validation. Catalogs are limited to 1 MiB and 1,000 entries. Downloads use HTTPS,
a 120-second timeout and at most five redirects. A package has a 32 MiB encoded
size limit and at most 1,024 files. Public GitHub releases are supported initially;
private repository authentication is not yet part of the downloader.

## Package v1

An app release is a UTF-8 JSON envelope named `NAME.paraco.json`:

```json
{
  "schemaVersion": 1,
  "name": "hello-world",
  "version": "0.1.0",
  "files": [
    {"path": "main.ts", "content": "BASE64_FILE_BYTES"},
    {"path": "paraco.json", "content": "BASE64_FILE_BYTES"}
  ]
}
```

This deliberately small envelope supports text and binary assets without archive
links, executable modes or extraction metadata. Base64 adds roughly one third to
file size, an acceptable initial tradeoff for bounded small apps. Files are
regular files only; portable relative paths use ASCII letters, digits, `.`, `_`,
and `-` separated by `/`. Hidden paths, traversal, reserved Windows devices,
case collisions, duplicate files, file/directory conflicts and the reserved
receipt path are rejected. `paraco.json` and its entrypoint must be present.

The installer checks the hash before parsing, validates all paths, writes into a
private temporary directory, validates the runtime manifest and exact capability
set, writes the receipt, then renames into the new destination. Failed staging is
cleaned up. A parent-directory lock serializes cooperating installers. A crash
may leave `.paraco-install.lock`; after confirming no installer is active, remove
that lock before retrying. Existing directories and symlinks are refused. The
destination parent must be host-controlled; protection against a malicious local
user racing writes in a shared destination directory is outside this contract.

## App repositories and release workflow

Copy [templates/hello-world](../templates/hello-world) into a new repository. Keep
`paraco.json`, the entrypoint, tests, license, documentation and packaging script
in source control. Update the package name, manifest and explicit file allowlist
when making another app. Never package `.git`, credentials, caches or arbitrary
working-directory contents. Include any required asset and dependency files in
the explicit allowlist.

The template packager sorts paths, encodes the allowlisted bytes and emits stable
JSON, so the same source/version produces byte-identical packages. The tag
workflow tests the app and publishes its package as a GitHub release asset.
Publishing is separate from catalog approval. Review the source commit, reproduce
the package, then add its digest and capability set to the catalog in a reviewed
change. The checksum verifies the approved bytes; the installer does not prove
that a publisher built those bytes from the declared source commit.

The sibling repository is `../paraco-hello-world`, published at
[dioko-ai/paraco-hello-world](https://github.com/dioko-ai/paraco-hello-world).
Its [v0.1.0 release](https://github.com/dioko-ai/paraco-hello-world/releases/tag/v0.1.0)
is live. The public asset was downloaded through `paraco marketplace install`,
verified against the catalog digest, and run successfully with welcome-page and
health-endpoint checks. Use the template's local package with `--package` for
offline installation testing.

## Verification

```sh
cargo test --bin paraco marketplace
python3 scripts/verify-marketplace.py
```

The integration check packages the template, installs through the public CLI,
launches the installed app, probes its page and health endpoint, and verifies that
checksum corruption and reinstall attempts fail. It uses temporary directories
and requires Deno on PATH. GitHub download verification is separate and requires
the published release.
