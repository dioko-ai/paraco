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
That Linux-only validation did not establish macOS support. Native Mac results
from the subsequent investigation are recorded below.

A subsequent external Mac run of `1f54940` reported SIGABRT with empty stderr
for both native probe processes, leaving all 14 contained checks UNVERIFIED;
esbuild preparation also failed. The loader-mapping correction and improved
crash diagnostics require a new Mac run before this failure can be marked fixed.

Follow-up external logs reported denials for
`security.mac.lockdown_mode_state`, `kern.bootargs`, and `/` for both probe
processes (PIDs 13354 and 13359). Inspection of the supplied PID 13359 crash
report (macOS 27.0.1, build 26A434, ARM64) and esbuild excerpt shows the same
dyld cache initialization stack: `CacheFinder`, `ignite`, `boot_boot`, then
`ignition_halt` and `abort_with_reason`. No separate crash report was supplied
for PID 13354. Both reports give termination namespace `0x23` and code `2`.
Apple's published [XNU reason definitions](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/sys/reason.h)
map namespace 35 (`0x23`) to `OS_REASON_LIBIGNITION`; the namespace is distinct
from `OS_REASON_SANDBOX` and `OS_REASON_CODESIGNING`. The meaning of code `2`
within LIBIGNITION has not been established here. This supports a loader-startup
failure, but does not identify which denied operation caused the abort or prove
a signing failure.

### Native macOS investigation, 2026-10-28

Test host: MacBookPro18,1, ARM64, macOS **27.0.1 (26A434)**, SIP enabled;
Rust 1.96.1, esbuild 0.28.2, workerd 1.20261026.1. The starting checkout was
`2718e09d76177ccfdc23d226772b13ce5dd2daf4` on `main`. Every changed native
executable was rebuilt with `cargo build --locked` before testing. Tests used
`PARACO_EXPERIMENTAL_MACOS_SANDBOX=1` with `PARACO_ALLOW_UNSANDBOXED` unset,
except for explicitly labelled negative controls.

The agent execution sandbox itself rejects nested Seatbelt application:
PIDs 15995 and 15996 exited 71 with
`sandbox-exec: sandbox_apply: Operation not permitted`; unified logs recorded
`forbidden-sandbox-reinit`. `log show` also refuses to run there. Those results
are UNVERIFIED and are not evidence against Paraco's profile. Subsequent native
commands ran through the host execution facility outside that inherited sandbox.
No host security controls were disabled.

The fresh baseline reproduced the reported loader failure:

```text
2026-10-28 23:21:09.510 -0700 paraco(16104) deny(1) file-read-data /
2026-10-28 23:21:09.701 -0700 paraco(16106) deny(1) file-read-data /
```

The matching reports are `paraco-2026-10-28-232109.ips` and
`paraco-2026-10-28-232109.000.ips` in `~/Library/Logs/DiagnosticReports`.
Capture times are 23:21:09.5848 and 23:21:09.7468 -0700 respectively. Both
report SIGABRT, termination namespace `0x23`, code `2`, and the same
`CacheFinder → ignite → boot_boot → ignition_halt → abort_with_reason` stack.
Stderr was empty. All 14 contained checks remained UNVERIFIED. Code `2` has
not been assigned a symbolic meaning here; signing fields are not a diagnosis.

Changing only the root rule from metadata to metadata **and file-read-data**
passed the ignition stage. It exposed a separate Rust initialization abort in
PIDs 16155 and 16157: `failed to allocate a guard page: Invalid argument
(os error 22)`. The exact associated denials were:

```text
2026-10-28 23:21:32.465 -0700 paraco(16155) deny(1) sysctl-read hw.pagesize_compat
2026-10-28 23:21:32.766 -0700 paraco(16157) deny(1) sysctl-read hw.pagesize_compat
```

Adding only that exact sysctl read then produced all 15 PASS checks. Apple's
[getpagesize implementation](https://github.com/apple-oss-distributions/Libc/blob/main/gen/FreeBSD/getpagesize.c)
uses numeric `CTL_HW/HW_PAGESIZE` and returns -1 if denied;
[XNU registers HW_PAGESIZE as pagesize_compat](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/kern/kern_mib.c).
[Rust's stack guard code](https://github.com/rust-lang/rust/blob/1.96.1/library/std/src/sys/pal/unix/stack_overflow.rs)
uses the page size for its fixed mapping. This source and the controlled
experiment support the guard-page diagnosis. Other logged denials, including
Rosetta metadata, `/dev/dtracehelper`, `kern.osproductversion`,
`kern.iossupportversion`, `kern.osvariant_status`, `hw.ephemeral_storage` and
notification/libinfo Mach services, did not prevent the successful workloads;
they were not granted.

The final startup permission changes are:

```scheme
(allow sysctl-read (sysctl-name "hw.pagesize_compat"))
(allow file-read-metadata file-read-data (literal "/"))
```

The root rule permits opening **and listing the root directory itself**. It does
not permit reading child files, recursively listing other directories, writing
root, or mapping arbitrary executables. The two exact sysctl allowances from
`2718e09` are retained. There is no blanket filesystem, sysctl or network grant.

Real workload validation also exposed path/launch problems after dyld startup.
The profile canonicalized grants, but esbuild input/output, workerd config and
capability addresses could still use `/var` or other symlink aliases. Passing
canonical paths fixes these without granting reads/listings of their ancestors.
For example esbuild PID 16228 logged `file-read-metadata /var` and
`file-read-data /private` (23:22:01.124–.126 -0700). A single-file task entry
also caused esbuild to inspect its parent directory, outside its exact file
grant. Tasks now pass the authorized source file as stdin, keep all dependencies
external, disable tree shaking, and reject retained imports using output
metadata. No task code runs during compilation and no additional file grant is
needed.

A focused native regression found that the loopback `network-inbound` allowance
also permits creating a new listener unless `network-bind` is explicitly denied
on this OS. The runtime profile now includes `(deny network-bind)`; inherited
listener traffic and exact capability-socket connections are validated together
with forbidden fresh binds. Environment clearing, descriptor handling,
executable mapping restrictions, deny-default policy and fail-closed errors are
preserved. Startup failures still remain UNVERIFIED.

Native validation commands (run outside inherited execution sandboxes):

```sh
export PARACO_EXPERIMENTAL_MACOS_SANDBOX=1
export PATH="$PWD/node_modules/.bin:$PATH"
unset PARACO_ALLOW_UNSANDBOXED
cargo build --locked
./target/debug/paraco sandbox-check
npm run test:containment
npm run test:security
cargo test --locked --test runtime --test server -- --test-threads=1
cargo test --locked --bin paraco native_startup_and_literal_root_boundary -- --ignored
cargo test --locked --lib --bin paraco -- --test-threads=1
TMPDIR=/private/tmp npm run test:cron-runtime
```

The containment integration fixture deliberately uses a symlinked TMPDIR and
verifies the served `sandbox-probe` report with genuine host evidence, ordinary
launches without evidence remaining UNVERIFIED, and explicitly unsandboxed
native/served controls reporting FAIL. The focused native regression tests both
compiler and runtime Rust startup, root listing, denied reads/writes of a sibling
sentinel, permitted scratch writes and denied new loopback listener binds.
The runtime suite includes a million-iteration hot JavaScript workload with
normal workerd JIT enabled (no `--jitless`), permitted HTTP/capability traffic,
forced host death during import and blocked execution, timeout and child cleanup.
The hosted-server suite covers forced death followed by immediate restart and
cleanup of apps, guardians, listeners and temporary directories. The security
suite covers task network grants, forbidden destinations/redirects, guest globals
and outside-package imports. These tests exercise JIT-enabled operation; they do
not instrument or prove which V8 optimization tier ran.

Final native results: sandbox-check **15/15 PASS**; served probe containment
integration PASS; workerd security suite PASS; cron runtime suite PASS (including
SIGKILL/restart); Rust library **18/18**, binary **123/123**, runtime **20/20**,
hosted-server **37/37**. The one opt-in native unit regression was excluded from
the ordinary binary run and passed separately. Probe unit tests **8/8**, formatting
and Clippy with `-D warnings` passed. Explicit unsafe native and served controls
returned FAIL; disabling the experimental backend returned UNVERIFIED, exit 1.
No unsafe result was counted as secure validation.

Two test portability corrections were needed to run the full suite: the fake
Codex executable now locates Node through PATH rather than `/usr/bin/node`, and
the slow-stop test waits for app readiness before signalling its child (dashboard
readiness alone can still leave compilation/startup in progress). The forced-stop
regression then passed in isolation and in the complete hosted-server suite.

The tested debug binary SHA-256 is
`1ea71e5e677b52bf294aaee7abb91074f12c989900adbe598375f40ceeedc851`.
The local evidence bundle at `/private/tmp/paraco-seatbelt-evidence-20261028`
retains selected exact denial logs, original matching crash reports, final JSON
controls, source/binary hashes, test output and the final patch. Host crash
metadata is kept locally rather than committed. This document and the source
changes in its commit identify the tested revision relative to the base above.

The adapter remains **experimental**, with this OS/build and ARM64 as the only
Mac configuration validated here. Other macOS builds and Intel/Rosetta require
separate native validation. Deprecated sandbox-exec/SBPL, resource quotas,
denial-of-service resistance, exploits, covert channels and inherited-capability
limits remain release considerations. Unix socket paths must fit macOS's
`sun_path` limit; the integration fixtures use short `/private/tmp` paths because
nesting long system TMPDIR paths exceeded that limit. This change does not add a
general long-socket-path workaround or promote the adapter to release-supported.
