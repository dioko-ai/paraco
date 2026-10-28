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

Browse the approved app catalog and install Hello World:

```sh
paraco marketplace list
paraco marketplace install hello-world --output ./hello-world
paraco run ./hello-world
```

Open the local address printed in your terminal. Installing an app requires a
network connection; apps that use online services also need connectivity.

Already have a Paraco app? Run it with `paraco run /path/to/app`.

## Your data

Paraco stores its local data under `~/.paraco` by default. Set `--data-dir` or
`PARACO_DATA_DIR` to choose another location; `--log-dir` and `PARACO_LOG_DIR`
control logs independently. App
permissions and AI access are explicit. Removing the program through the
installer preserves your app data and configuration.

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
Plugins requiring AI or tasks cannot be installed from the dashboard yet. Use
`paraco marketplace install NAME --output DIRECTORY` and add the app to a
configured host with the required grants and AI settings. To remove a downloaded plugin, stop it,
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
