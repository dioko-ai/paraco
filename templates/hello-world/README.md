# Hello World for Paraco

A tiny starter app for Paraco. It serves a polished welcome page at `/` and a JSON health response at `/health`, with no external dependencies.

## Run locally

Install Paraco and [Deno](https://deno.com/), then run:

```sh
paraco run .
```

Then open <http://localhost:3000>. Use `--port` to select another port. Run the tests with:

```sh
deno test
```

## Publish a release

Create and push a version tag such as `v0.1.0`. The release workflow runs the tests, builds `dist/hello-world.paraco.json`, and attaches that package to a GitHub release. To package locally:

```sh
python3 scripts/package.py --version 0.1.0 --output dist/hello-world.paraco.json
```

The command prints the package SHA-256. The release package contains only `main.ts` and `paraco.json`; documentation, tests, and build files stay in the source repository.

When adapting this template, update the repository owner placeholder `dioko-ai/paraco-hello-world` in your release setup as needed. Publishing a GitHub release does not list the app in the Paraco catalog; catalog approval is a separate step.

## License

Apache-2.0. See [LICENSE](LICENSE).
