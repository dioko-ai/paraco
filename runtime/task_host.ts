// Fresh bounded task adapter. No host credentials or ambient capabilities.
const [entry, metadata] = Deno.args;
const run = JSON.parse(metadata);
const module = await import(entry);
if (typeof module.default !== "function") {
  throw new Error("Task must export a default async function");
}
let notifications = 0;
const ctx = Object.freeze({
  execution: Object.freeze(run),
  log: (...values: unknown[]) => console.log(...values),
  notify: (message: string) => {
    if (notifications >= 100) throw new Error("Task notification limit is 100 per run");
    notifications++;
    if (typeof message !== "string" || message.length > 2048) {
      throw new Error("Notification must be at most 2048 characters");
    }
    console.log("PARACO_NOTIFICATION:" + JSON.stringify({ message }));
  },
});
await module.default(ctx);
