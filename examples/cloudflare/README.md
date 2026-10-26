# Portable Worker example

`worker.ts` imports the same handler used by `paraco run examples/hello`.
From this directory, developers with a configured Cloudflare account can run
`npx wrangler deploy`. This creates/updates `paraco-portable-hello`.
No live deployment is implied by the checked-in example.

The adapter supports the optional `PARACO` service binding for scoped host
capabilities. A D1 storage service and per-installation credential-mediating binding
are now provided in `runtime/d1-service.ts` and `runtime/cloudflare-binding.ts`.
See [platform provisioning](../../docs/platform-evolution.md#cloud-only-provisioning)
and the configuration templates in `platform/`. AI needs a separate host-provided
service; task scheduling remains local-only and is rejected on Cloudflare.
