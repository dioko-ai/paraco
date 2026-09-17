// This adapter is embedded in the Rust binary. It owns Deno.serve so portable
// apps need only export a web-standard fetch handler.
const [entrypoint, portText, nonce] = Deno.args;
if (!entrypoint || !portText || !nonce) {
  throw new Error(
    "Paraco host requires an entrypoint, port, and readiness token",
  );
}

const app = (await import(entrypoint)).default;
if (!app || typeof app.fetch !== "function") {
  throw new TypeError(
    "app default export must provide fetch(request, context)",
  );
}

const port = Number(portText);
if (!Number.isInteger(port) || port < 1 || port > 65535) {
  throw new RangeError("port must be an integer from 1 through 65535");
}

const server = Deno.serve(
  {
    hostname: "127.0.0.1",
    port,
    onListen() {
      // onListen runs only after Deno has bound the TCP listener.
      console.log(`PARACO_READY:${nonce}`);
    },
  },
  async (request) => {
    try {
      const response = await app.fetch(request, Object.freeze({}));
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
