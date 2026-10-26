> Runtime migration update: the supported backend is now workerd. See [current implementation and release gates](workerd-migration.md). Deno-specific decisions and test results below describe the previous runtime and are historical.

# Foundation decisions

These decisions define the first foundation slice. They do not claim that the
later milestones in [the foundation plan](foundation-and-cron-plan.md) are
implemented.

## AD-001: release baseline and verification evidence

The intended first-release targets are x86-64 and ARM64 Linux and macOS. The
provisional compatibility floor is Linux distributions with glibc 2.35 or newer
(the Ubuntu 22.04 generation) and macOS 13 or newer. These are planning targets,
not verified support claims: every OS/architecture combination remains unverified
until it has native launch/request/shutdown evidence. CI configuration is evidence
of an intended check, not proof of a tested target.
The current crate requires Rust 1.96 or newer; the exact Deno pin belongs to the
repeatable-verification slice.

## AD-002: trusted local application boundary

The current runtime accepts code installed or reviewed by its owner. Its
loopback-only gateway, restricted Deno launch permissions, and separate app
processes reduce accidental exposure, but do not isolate hostile code. Hosted
applications use distinct deployment-specific `.localhost` browser origins, with
a separate authenticated management origin. Marketplace,
third-party hosting, and hostile-code isolation require a separate security
decision.

## AD-003: stable identifiers and draft manifest v1

Future runtime, deployment, task, revision, occurrence, and execution IDs are
opaque stable identifiers; display names and URLs are mutable labels. This
records their intended use only and adds no persistent scheduler state. The
public manifest schema is `docs/paraco-manifest.schema.json`. Existing
unversioned manifests remain valid; an explicit `schemaVersion` must be `1`.
Structural validation rejects unknown fields and duplicate capabilities. Rust
also validates the filesystem-dependent entrypoint existence, file type, and
canonical containment after schema validation.

## AD-004: license

Paraco uses Apache-2.0. This permissive license includes patent terms and is the
selected option from the foundation plan; `LICENSE` is the authoritative text.

## AD-005: portable datasets and immutable output artifacts

Use installation-scoped private SQLite/D1 namespaces plus explicitly granted shared
dataset aliases. Expose versioned JSON records rather than arbitrary SQL. Publish
bounded artifact content and metadata atomically in the same database, with immutable
identities, deterministic publication sequence, and saved execution input references.
Keep configuration, credentials, and scheduling private. Large object storage and
replication are deferred. See [platform evolution](platform-evolution.md) for the
implemented contract, deployment configuration, limits, and verification scope.
