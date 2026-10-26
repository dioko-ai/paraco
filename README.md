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

The release download will include everything Paraco needs. You will **not** need
to install Deno, Rust, Node.js, npm, or Docker, or clone this repository.
Installation will use your own account without `sudo`.

The planned installation options are a single download-and-run command for macOS
and Linux, and Homebrew for people who already use it. Each installs the same
Paraco bundle, including its private runtime. npm is not required.

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

Paraco stores its local state and logs under `~/.paraco` by default. App
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
