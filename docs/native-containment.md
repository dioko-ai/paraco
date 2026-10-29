# Native containment and the sandbox probe

Paraco uses native processes, not a VM. `src/containment` owns platform policy
for both esbuild preparation (apps and tasks) and workerd execution. The host
continues to own capability grants, credentials, lifecycle and persistent data.

## Try it

With Paraco and its pinned workerd/esbuild tools installed:

```sh
paraco sandbox-check
paraco sandbox-probe --port 3000
```

The first command prints JSON and exits nonzero unless every native check passes.
The second serves the bundled probe plugin at `http://127.0.0.1:3000` with native
evidence collected by the host before startup. Its state/log directories are
temporary and removed on orderly shutdown. On a source checkout, build first
and put `node_modules/.bin` on PATH, then use `target/debug/paraco`.

The plugin also exists in `examples/sandbox-probe`. Running that example normally
does not inject host evidence: it reports UNVERIFIED rather than inventing a
green OS result. Query parameters and application manifests cannot supply host
evidence. The report has no elevated capability grants.

Green means **all listed checks passed**, not that no possible sandbox escape
exists. Red means an operation succeeded that should have been blocked, or a
positive control failed. Amber means a required test could not establish a result.
Unknown errors, missing evidence, malformed results and timeouts never count as
successful containment.

## What is tested

An independent native child, launched through the same policy builder as real
workloads, attempts to read/open-for-write a disposable file outside its grants,
connect to a live host loopback listener, launch `/bin/sh`, and read a known
inherited environment canary. Allowed file reads and scratch writes are positive
controls. A separate probe verifies that readable input files cannot be opened
for writing. Both compiler and runtime policies are exercised. No personal data is
read, and no destructive or remote probes are performed.

The plugin separately checks for exposed Node/Deno/Bun globals, filesystem and
subprocess APIs, undeclared capabilities, and host globals through dynamic code.
An optional user-supplied loopback fixture tests direct guest fetch. A failed
fetch is UNVERIFIED because failure alone does not identify its cause.

Native evidence is a startup snapshot, not continuous monitoring. Pressing Run
probes repeats the guest checks; restart the command to recollect native evidence.
The native probe substitutes its own executable and scratch fixtures for the
application. It tests the policy mechanisms, not every possible runtime/library
behavior. Separate workerd security tests exercise actual runtime capability and
network restrictions. Tests do not prove resistance to kernel/runtime exploits,
cross-plugin covert channels, resource exhaustion, or every inherited-handle case.

## Platforms

Linux uses bubblewrap with private namespaces, selected read-only libraries and
inputs, explicit writable outputs, and a cleared environment. It no longer mounts
all of `/usr`. Unrelated descriptors are marked close-on-exec before launch;
workerd receives only its explicitly passed HTTP listener in addition to standard
streams. This requires Linux `close_range(CLOSE_RANGE_CLOEXEC)` support (Linux
5.11 or newer); failures abort launch rather than dropping descriptor protection.
Existing namespace and bubblewrap prerequisites still apply. Hard cgroup quotas
remain a release gate and are not claimed by the green probe result.

macOS has an **experimental Seatbelt prototype**, behind
`PARACO_EXPERIMENTAL_MACOS_SANDBOX=1`. It uses `/usr/bin/sandbox-exec`, a deny-default
per-execution profile, selected read/write paths, the exact executable, and the
specific capability Unix socket. It does not enable general outbound networking
or subprocess forking. The runtime accepts requests on the inherited loopback
listener. It is not a signed App Sandbox/XPC implementation: shared helper identity
alone does not establish per-plugin isolation. `sandbox-exec`/SBPL are deprecated
or undocumented interfaces and require validation on each supported macOS release.

```sh
PARACO_EXPERIMENTAL_MACOS_SANDBOX=1 paraco sandbox-check
PARACO_EXPERIMENTAL_MACOS_SANDBOX=1 paraco sandbox-probe
```

No macOS hardware validation is implied by Linux tests or profile-string tests.
The experimental flag is propagated to app and task guardians. Without it macOS
still fails closed; profile failures never trigger an unsandboxed retry. The
separate trusted-development escape hatch remains available and always makes the
probe report FAIL. Real Mac validation must include workerd JIT, compilation,
capability sockets, permitted listener traffic, forbidden access and host death.

If every contained check reports SIGABRT, the two native probe processes failed
before completing their checks; this is not fourteen observed escape attempts.
The profile explicitly grants executable mapping for the selected binary and
system loader paths, including macOS Cryptex locations. This corrects an omitted
loader permission, but does not by itself establish the cause of a particular
Mac's startup crash or verify the backend.

When stderr is empty, the report includes the failed child's PID. On the Mac,
record `sw_vers` and `uname -m`, then inspect the matching Paraco/esbuild crash
report in `~/Library/Logs/DiagnosticReports` and sandbox denial logs. The useful
fields are the termination reason, exception details and crashed thread backtrace.
Retain these results before changing permissions further. Do not enable trusted
development mode to count a failed secure startup as a pass.

Windows remains unsupported. The common execution specification uses paths and
workload types; platform-specific code is isolated behind its builder. Adding a
Windows backend will also require replacing existing Unix listener/bridge and
guardian assumptions. This change does not claim that the entire crate builds
on Windows or that Job Objects alone establish an access boundary.

## Verification

```sh
npm run test:sandbox-probe
npm run test:containment
npm run test:security
```

The containment integration test requires working native containment, checks a
fully green served report, verifies missing evidence is amber, and deliberately
runs the harmless canaries in trusted-development mode to require a red result.
Run on a real Mac with the experimental flag to validate that backend; do not use
trusted-development CI results as macOS containment evidence.

### Observed in this implementation

Linux validation passed: all 15 native checks, the full served probe, the
unsandboxed negative control and missing-evidence case, and the existing workerd
security suite. The Rust library/binary, runtime and hosted-server suites passed
(196 tests before the additional controller-schema unit test; that test passed
separately). Formatting, Clippy and public TypeScript declarations passed.
Chromium displayed a green PASS summary with 15 host checks and no page errors.
No real macOS execution was available in this session; the prototype is still
experimental and macOS release validation remains outstanding.

A subsequent external Mac run of `1f54940` reported SIGABRT with empty stderr
for both native probe processes, leaving all 14 contained checks UNVERIFIED;
esbuild preparation also failed. The loader-mapping correction and improved
crash diagnostics require a new Mac run before this failure can be marked fixed.
