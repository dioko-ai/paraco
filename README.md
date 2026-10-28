# Paraco

Run apps and scheduled jobs on your own computer. Paraco hosts portable apps,
provides a local dashboard, and keeps your app data on your machine.

- **Run apps locally.** Start, stop, and restart them from your browser.
- **Schedule jobs.** Use the built-in cron app to create recurring tasks, review
  run history, and see notifications.
- **Use AI when you choose.** Connect a provider explicitly; local operation
  does not require a cloud account or paid AI.

## Install

Paraco is being prepared for public release. **A public installer and Homebrew
package are not available yet.** This README will include the working install
command when the first release is published.

The planned bundle contains the Rust host, workerd `2026-10-26`, and native
esbuild `0.28.2`. End users will **not** need Deno, Rust, Node.js, npm, or
Docker, or need to clone this repository. Installation is intended to use your
own account without `sudo`.

The planned installation options are a single download-and-run command for macOS
and Linux, and Homebrew for people who already use it. Each is intended to install the same Paraco bundle, including its private
runtime and compiler. npm is not required. Linux app execution requires
bubblewrap and unprivileged user/network namespaces. macOS secure containment is
not implemented yet, so normal app execution currently fails closed there.

## Get started

Once installed, start the built-in scheduler:

```sh
paraco cron
```

Open the dashboard link printed in your terminal and select **cron**. Create a
job, review its code, and publish a one-time or recurring schedule. AI drafting
is optional, and drafts are never published automatically.

Leave Paraco running while you use your apps or scheduled jobs. Press Ctrl+C to
stop it. Starting automatically at login is a separate, optional setup step.

## Add an app

With `paraco cron` running, open its dashboard (`paraco open`), choose **Browse
plugins**, then **Download and install** beside Hello World. Wait for **running**
and select **Open app**. The host registers the app, manages its lifecycle, and
loads it again on restart. No second process or port is needed. Downloads require
a network connection.

For local source apps, see [Hosting multiple apps](docs/local-hosting.md).

### Standalone development

`paraco run /path/to/app --port 3001` launches an independent development process
outside dashboard management. Use a free port (the host normally owns 3000), and
stop that process separately. For ordinary catalog use, install from the dashboard.

## Your data

Paraco stores its local data under `~/.paraco` by default. Set `--data-dir` or
`PARACO_DATA_DIR` to choose another location; `--log-dir` and `PARACO_LOG_DIR`
control logs independently. App
permissions and AI access are explicit. Removing the program through the
installer preserves your app data and configuration.

### Move or recover a host configuration

Stop the host, then run `paraco host-state --config /old/path/server.json`.
Copy the printed `"host_id"` field into the top level of that configuration before
moving or renaming it. Keep the same `--data-dir`, and update relative app and AI
paths to point to their original absolute locations. Start the moved configuration
with `paraco serve --config /new/path/server.json`. Installed plugins and data
continue using the original store. Do not run two hosts with the same identity.

Already moved the file? `paraco host-state` lists existing stores and identities
without modifying them. Select the original store, back it up with
`paraco backup --state STORE --output backup.sqlite3`, and add its identity as
`host_id` to the moved configuration. If you use a login service, update its
registration to the new configuration path. Older nested stores are listed too;
use their parent namespace's parent as `--data-dir` when explicitly selecting them.

## Learn more

- [Tasks and the cron app](docs/tasks-and-cron.md)
- [Hosting multiple apps](docs/local-hosting.md)
- [Approved apps](docs/marketplace.md)
- [AI configuration](docs/ai-routing.md)

## Contributing

Building Paraco from source requires development tools. These are for
contributors, not prerequisites for the packaged app.

See [development setup](docs/development.md), the [runtime guide](docs/runtime-guide.md),
and [release preparation](docs/installation-and-release.md).

## Portable datasets and artifacts

Applications can use versioned records, immutable output artifacts, and explicitly
shared local or cloud-backed datasets. See [platform evolution](docs/platform-evolution.md)
for contracts, permissions, Cloudflare D1 provisioning, and limits, or run the
[Artifact Notebook example](examples/artifacts/README.md).

### Dashboard plugin downloads

Open `http://localhost:3000/` or use `paraco open`. Select **Browse plugins**
to view the bundled catalog, versions, and requested capabilities. **Download and
install** verifies the catalog SHA-256 and manifest before starting the plugin.
Downloads persist in the server's private state directory and load on restart.
Configure AI providers and per-app overrides under **AI for your apps**.
Plugins requiring AI or tasks offer **Set up and install**: choose an existing
host AI configuration (optional), enter explicit task network hosts, and review
the capability approval before installation. Grants persist across host restarts. To remove a downloaded plugin, stop it,
wait for **stopped**, then select **Uninstall**. Its downloaded source is removed;
stored application data is retained and reattached when the same catalog plugin
is reinstalled. Apps supplied by the server configuration
remain managed through that configuration.

### Local URLs without a port

Run `paraco serve --config server.json --port 80` to use URLs such as
`http://hello-world.localhost`. Port 80 must be available on both IPv4 and IPv6
loopback, and your OS must permit binding it. Keep Paraco running as your normal
user; if privileged-port access is required, configure a local forwarding service
instead of running the runtime as root. The dashboard uses the same gateway port.
Other gateway ports remain explicit in URLs.
