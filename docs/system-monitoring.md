# Read-only system monitoring

Local applications can declare `"capabilities": ["system"]` and call
`await context.system.snapshot()`. The Linux host supplies a point-in-time
snapshot with aggregate CPU ticks, total/available memory and swap in bytes,
load averages, uptime, logical CPU count, hostname, and listening TCP/bound UDP
sockets. It exposes no process command lines, environment, arbitrary file reads,
process control or active network probes.

The host bridge authenticates each call and checks the installation's `system`
request. Applications without this capability receive no `context.system` API.
The existing capability baseline protects upgrades: adding `system` to an
installed app requires explicit `paraco approve-capabilities` while stopped.
Removing it removes the API on the next launch. Marketplace catalogs also declare
this capability. Installing a new app requesting it permits inspection of local
host metrics and socket addresses; review that access as part of installation.

The public shape is `SystemSnapshot` in `runtime/paraco.d.ts`. CPU total excludes
recounting guest ticks; idle includes iowait. Clients calculate utilization from
two consecutive snapshots and should display no utilization for the initial
sample or a non-positive tick delta. Memory uses Linux `MemAvailable`. Socket
entries include protocol, address, numeric port and state; at most 2,048 entries
are returned. Unreadable socket tables and truncation produce warnings. CPU and
memory read errors fail the snapshot rather than fabricating zero values.

The first implementation requires Linux procfs. Other platforms return an explicit
unsupported-platform error. `system` defaults the manifest target to local and
is rejected for Cloudflare. The shared Cloudflare adapter rejects it as an
unsupported requirement. Existing runtime containment remains unchanged: only
the Rust guardian reads procfs, not the sandboxed app.

`npm run test:system-runtime` verifies metrics, discovery of a real listening
socket, capability removal and upgrade rejection. Unit tests verify Linux address
byte order and guest CPU accounting. The System Vitals plugin in the sibling
`paraco-plugins` folder consumes this API.
