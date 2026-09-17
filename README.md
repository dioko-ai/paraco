# paraco

Paraco is a local-first runtime for portable applications. This repository is
starting with a deliberately small execution slice: Rust supervises a Deno
subprocess that serves a TypeScript HTTP application on loopback.

## Try it

Prerequisites are a current [Rust toolchain](https://rustup.rs/) and
[Deno](https://docs.deno.com/runtime/) available on `PATH`.

```sh
cargo run -- run ./examples/hello
curl http://127.0.0.1:3000
```

The response is `Hello from Paraco`. Use `--port` to select another loopback
port, for example `cargo run -- run ./examples/hello --port 8787`. Press Ctrl+C
to stop the application; Paraco sends it a graceful termination signal, then
forces termination after five seconds if needed.

## Application contract

An app directory contains `paraco.json` and its source. The currently supported
manifest schema is intentionally explicit:

```json
{
  "name": "hello",
  "entrypoint": "main.ts",
  "capabilities": []
}
```

`name` is 1–63 lowercase ASCII letters, digits, or hyphens and starts with a
letter. `entrypoint` must be a relative file contained in the application
directory. `capabilities` is required, but this first runtime supports none, so
it must be an empty array. Unknown fields and requested capabilities fail
validation.

The entrypoint exports a default object whose `fetch` method receives web
standard `Request` and returns a `Response` (or a promise for one):

```ts
export default {
  fetch(_request: Request, _context: object): Response {
    return new Response("Hello from Paraco");
  },
};
```

The Deno adapter calls `Deno.serve` and binds only `127.0.0.1`. The Rust binary
embeds the adapter, so `paraco` does not rely on the shell working directory to
find it. It grants Deno read access to the app directory and network access only
to its selected loopback listener. It clears inherited environment variables
apart from `PATH`, and supplies a temporary `DENO_DIR`; no ambient credential
environment variables are passed to apps. These permissions are only the narrow
ones needed for this example and are not a complete security sandbox for
untrusted code.

## Verify

```sh
cargo fmt --check
cargo test
cargo run -- run ./examples/hello
```

See [the first-runtime brief](docs/first-runtime.md) for the acceptance criteria
and intentionally deferred work.

## Current scope

This is an early local runtime, not a daemon, container runtime, cloud deployer,
database, management UI, installer, or AI proxy. AI routing and the
OpenAI-compatible endpoint are planned as the next slice after the HTTP runtime
is proven.
