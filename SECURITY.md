# Security policy

Please report a suspected vulnerability privately to the repository owner using
the contact method listed by its hosting service. Do not publish exploit details
until maintainers have had a reasonable opportunity to investigate and respond.

Linux execution requires namespace containment by default. This migration has
not yet established hardened hostile-code hosting: per-app memory/CPU quotas,
macOS containment and broader escape testing remain release gates. The explicit
`PARACO_ALLOW_UNSANDBOXED=1` mode is for trusted development only.

See [the current threat model and limits](docs/workerd-migration.md).
