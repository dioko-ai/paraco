// This adapter is embedded in the Rust binary. It owns Deno.serve so portable
// apps need only export a web-standard fetch handler.
const [entrypoint, portText, nonce] = Deno.args;
if (!entrypoint || !portText || !nonce) {
  throw new Error(
    "Paraco host requires an entrypoint, port, and readiness token",
  );
}

// Bootstrap is consumed before importing application code. No provider secrets
// are sent to this process; this token authorizes only this running app.
const hostConfig = JSON.parse(await new Response(Deno.stdin.readable).text());
const bootstrap = hostConfig.ai;
const connect = Deno.connect.bind(Deno);
const encoder = new TextEncoder();
const decoder = new TextDecoder();

async function readExact(conn: Deno.Conn, size: number): Promise<Uint8Array> {
  const bytes = new Uint8Array(size);
  let offset = 0;
  while (offset < size) {
    const count = await conn.read(bytes.subarray(offset));
    if (count === null) throw new Error("AI connection closed");
    offset += count;
  }
  return bytes;
}

const aiContext = Object.freeze(
  bootstrap
    ? {
      ai: Object.freeze({
        async complete(
          request: { provider?: string; model?: string; prompt: string },
        ) {
          const body = encoder.encode(
            JSON.stringify({ token: bootstrap.token, request }),
          );
          if (body.length > 65536) throw new Error("AI request exceeds 64 KiB");
          const conn = await connect({
            hostname: "127.0.0.1",
            port: Number(bootstrap.address.split(":")[1]),
          });
          const timer = setTimeout(() => {
            try {
              conn.close();
            } catch { /* closed */ }
          }, 3000);
          try {
            const frame = new Uint8Array(body.length + 4);
            new DataView(frame.buffer).setUint32(0, body.length);
            frame.set(body, 4);
            let offset = 0;
            while (offset < frame.length) {
              offset += await conn.write(frame.subarray(offset));
            }
            const header = await readExact(conn, 4);
            const length = new DataView(header.buffer).getUint32(0);
            if (length > 65536) throw new Error("AI response exceeds 64 KiB");
            const response = JSON.parse(
              decoder.decode(await readExact(conn, length)),
            );
            if (response.error) throw new Error(response.error);
            return response.result;
          } finally {
            clearTimeout(timer);
            try {
              conn.close();
            } catch { /* timed out */ }
          }
        },
      }),
    }
    : {},
);

const context = Object.freeze({ ...aiContext, basePath: hostConfig.basePath });

const app = (await import(entrypoint)).default;
if (!app || typeof app.fetch !== "function") {
  throw new TypeError(
    "app default export must provide fetch(request, context)",
  );
}

const port = Number(portText);
if (!Number.isInteger(port) || port < 0 || port > 65535) {
  throw new RangeError("port must be an integer from 0 through 65535");
}

const server = Deno.serve(
  {
    hostname: "127.0.0.1",
    port,
    onListen({ port }) {
      // onListen runs only after Deno has bound the TCP listener.
      console.log(`PARACO_READY:${nonce}:${port}`);
    },
  },
  async (request) => {
    try {
      const response = await app.fetch(request, context);
      if (!(response instanceof Response)) {
        throw new TypeError("fetch must return a Response");
      }
      return response;
    } catch (error) {
      console.error("Paraco app request failed:", error);
      return new Response("Internal Server Error", { status: 500 });
    }
  },
);

// Rust sends SIGTERM during Ctrl+C shutdown. Let Deno stop accepting requests
// before Rust's bounded forced-termination fallback runs.
if (Deno.build.os !== "windows") {
  Deno.addSignalListener("SIGTERM", () => {
    void server.shutdown();
  });
}
