# Installation and release plan

## Local archive installer (unpublished)

`scripts/install-archive.sh` is an intentionally local, unpublished installer
for a user-supplied archive and explicit SHA-256. It requires an absolute
prefix, channel, and version, serializes ownership with a prefix lock, rejects
traversal, duplicates, links, special files and oversized extraction, stages
before an atomic channel-pointer switch, and retains old releases. Failed
validation therefore cannot change the active channel. `uninstall` removes only
the channel pointer and refuses a state-marked running runtime; it preserves
state, configuration, credentials and releases. The script does not download,
publish, register services, or claim health-checked service rollback.

A checksum establishes integrity of the selected bytes, **not publisher
authenticity**. The user must supply trusted local provenance. Signing,
notarization, remote release provenance, native upgrade/rollback/service smoke
tests, prepared-artifact compatibility checks, and schema-downgrade recovery
remain release gates rather than claimed support.

## Package recipe tooling (unpublished)

`scripts/generate-package-recipes.sh` generates a Homebrew formula and package
metadata only when given concrete version, HTTPS artifact URL, SHA-256, and
target values **and a local bundle whose SHA-256 matches those inputs**. It does
not supply placeholder release metadata, publish a tap, download an artifact,
or change a package manager. Generated formulas install
the bundle's private Deno runtime and do not depend on system Deno; package
installation does not register a service and removal must preserve user data.

`scripts/verify-macos-package.sh` fails closed unless running on macOS with a
final package, signing identity, and notarization keychain profile. It verifies
both bundled executables and the final package/notarization. Authenticity,
dependency-vulnerability review, signing/notarization, and native package
evidence remain required release acceptance gates; none is claimed here.

This document records the installation direction discussed for Paraco and guides
future implementation. It is a plan, not documentation of a shipped installer.
Use it alongside [the product spec](../spec.md) and
[the first-runtime milestone](first-runtime.md).

## Current state

The repository contains a Rust CLI that supervises a Deno subprocess. The Deno
host adapter is embedded in the Rust binary, but the Deno executable is currently
resolved from `PATH`. Building from source requires Rust, and running an app
requires a separate Deno installation. There are no release installers or
published installation commands yet.

The initial runtime was verified on Linux. Do not interpret target platforms in
this plan as platforms already tested or supported by a release.

## User experience requirement

Users should install Paraco once and receive everything required to run it.
They must not need to install Rust, Deno, Node.js, npm, or Docker separately.
Installation must not require a Paraco cloud account.

The target is an easy CLI installation on macOS and Linux, followed later by
graphical installation options. Installing the runtime and managing apps without
a terminal are separate milestones: the latter needs a local management
interface and startup integration.

## Release bundle

The proposed packaging foundation is a platform-specific bundle containing:

- the compiled Rust `paraco` executable
- a private Deno executable pinned to an exact, tested version
- required license and third-party notices
- release metadata identifying the Paraco version, Deno version, and target

Paraco should locate its bundled Deno by absolute path relative to the installed
bundle, independently of the working directory or a user's system Deno. Resolve
launcher symlinks correctly, including those installed by Homebrew. The exact
directory layout remains to be chosen.

Deno remains a separate supervised process. Bundling does not require embedding
V8 in Rust or compiling each user application into its own executable. A larger
download is an accepted tradeoff for a complete installation.

Update the private Deno runtime as part of a tested Paraco release. Do not
independently auto-upgrade it or silently substitute a system Deno in packaged
installations. Any development override should be explicit and documented.

Target native builds for macOS and Linux, each on ARM64 and x86-64. Before claiming
support, choose and test minimum macOS versions and the Linux distribution/libc
baseline for both executables. A Linux archive is not automatically compatible
with every distribution.

## Installation channels and priority

These are the recommended channels from the planning discussion. No package
names, download domains, tap ownership, or release hosting locations are reserved
by this document.

| Channel | Intended role | Sequence |
| --- | --- | --- |
| Portable release archive | Shared bundle, manual installation, and CI | First |
| Shell installer | One-command installation on supported macOS/Linux systems | First |
| Homebrew tap | Installation, upgrades, and removal for Brew users | Next |
| macOS `.pkg` | Graphical installer containing the same runtime bundle | Later |
| Linux `.deb` and `.rpm` | Native packages for selected distributions | Later |
| npm | Possible convenience wrapper for existing Node users | Deferred |

Do not make npm the primary installation path: it would introduce a Node/npm
prerequisite for this Rust/Deno runtime. If added later, it should install the
same tested release payload rather than introduce a separate runtime versioning
scheme.

### Shell installer

Offer a short HTTPS download-and-run command once a real release endpoint exists.
Also document downloading and inspecting the script before running it. Do not
publish illustrative URLs as working installation commands.

The installer should:

- detect OS and CPU architecture and reject unsupported targets clearly
- support an explicit release version for reproducible installation and CI
- download the matching prebuilt bundle and verify its published checksum
- install into a user-owned location without requiring `sudo`
- provide clear PATH instructions and make any shell-profile edits explicit
- stage and validate a new installation before replacing a working version
- support repeat installation and document upgrade and removal procedures
- leave application data, configuration, and secrets separate from program files

Checksum verification detects corruption; it is not a substitute for release
authenticity. Choose and document the trusted distribution and signing mechanism
before publishing releases. The exact install root, command options, and PATH
editing behavior are implementation decisions still to be made.

After installation, starting Paraco and the dependency-free hello example should
not download Deno or require a network connection. Apps with external dependencies
or external services have their own connectivity requirements.

### Homebrew

Start with a project-maintained tap; acceptance into Homebrew's main repository
is not a prerequisite. The eventual command follows the pattern
`brew install <owner>/<tap>/paraco`; this is a placeholder, not an available
command.

The package should install the tested Paraco/Deno combination, expose `paraco`
on PATH, and keep the bundled Deno private. Reuse the release artifacts where
appropriate. Confirm the concrete formula/cask implementation against Homebrew's
current guidance when implementing it.

Homebrew owns upgrades and removal for Homebrew installations. A direct
installer or future self-update command must not overwrite a Brew-managed copy.
Document how installation ownership is detected and how users can deliberately
migrate between channels without losing data.

### Graphical and native packages

For macOS, plan a signed and notarized `.pkg` containing the bundle. Signing and
notarization require release infrastructure and Apple credentials that are not
currently configured. Verify the bundled executables and final distributed
package as part of the release process.

For Linux, add `.deb` and `.rpm` packages once the supported distribution baseline
is established. Retain the portable archive for manual installation and CI.

Service registration, start-at-login behavior, and a graphical management app are
separate work. Do not silently add an always-running service as part of the first
CLI installer.

## Incremental implementation and verification

1. Define the bundle layout and pin Deno. Teach the launcher to locate its private
   runtime. Verify invocation from another directory and through a symlink.
2. Build a release archive for one target. On a clean machine without Rust or
   Deno installed, unpack it, run the hello app, request its HTTP response, and
   confirm clean shutdown. Repeat with an unrelated Deno on PATH to prove that
   the bundled version is used.
3. Implement the shell installer using that archive. Verify first installation,
   version pinning, repeat installation, failed download/checksum handling,
   upgrade, and removal. Failed upgrades must preserve the working installation;
   removal must preserve user data unless deletion is explicitly requested.
4. Expand build and smoke-test coverage to the supported macOS/Linux architecture
   matrix. Record minimum OS requirements and any unsupported combinations.
5. Add the Homebrew tap and verify installation, upgrade, and removal using the
   same release version. Check that channels do not overwrite one another.
6. Add signed/notarized macOS installers and selected Linux native packages.

For each published release, record the source revision, exact Deno version,
supported targets, verification results, checksums, and release notes. Build and
test the payload before publishing it or updating installer/tap references. Keep
older versioned artifacts available for pinned installations and recovery.

## Open decisions for future agents

- Release host, download domain, tap owner, and package names.
- Installation paths, bundle layout, and development runtime overrides.
- Minimum supported OS versions and Linux libc/distribution baseline.
- Release signing, checksum distribution, and macOS signing credentials.
- Update command design, rollback policy, and channel ownership detection.
- Project license and third-party redistribution notices; no project license
  has been selected yet.
- Timing and design of service startup and the local management interface.

Implement these in small, verifiable steps. Do not describe future channels as
available until their artifacts exist and their installation flow has been tested.

## Reference documentation

- [Deno installation](https://docs.deno.com/runtime/getting_started/installation/)
- [Homebrew taps](https://docs.brew.sh/Taps)
- [Maintaining a Homebrew tap](https://docs.brew.sh/How-to-Create-and-Maintain-a-Tap)
- [Apple distribution packaging](https://developer.apple.com/documentation/xcode/packaging-mac-software-for-distribution)

Consult current upstream documentation when implementing provider-specific
packaging details; this document records Paraco's intended product experience.
