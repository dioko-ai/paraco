interface Context {
  tasks: { command(action: string, body: unknown): Promise<unknown> };
  ai: { complete(body: { prompt: string }): Promise<{ text: string }> };
}
const assets = new Map([
  ["/", ["index.html", "text/html; charset=utf-8"]],
  ["/app.js", ["app.js", "text/javascript; charset=utf-8"]],
]);
const actions = new Set(["list", "draft", "publish", "archive", "unarchive", "run", "cancel", "history", "logs", "revisions", "revision", "notifications", "read_notification", "preview", "status"]);
async function jsonBody(request: Request): Promise<Record<string, unknown>> {
  if (!request.headers.get("content-type")?.startsWith("application/json")) throw new Error("invalid request");
  const reader = request.body?.getReader();
  if (!reader) throw new Error("invalid request");
  const chunks: Uint8Array[] = []; let size = 0;
  let timer: ReturnType<typeof setTimeout> | undefined;
  const expired = new Promise<never>((_, reject) => {
    timer = setTimeout(() => { reject(new Error("request body timed out")); void reader.cancel().catch(() => {}); }, 5000);
  });
  try {
    for (;;) {
      const {done, value} = await Promise.race([reader.read(), expired]); if (done) break;
      size += value.byteLength;
      if (size > 48_000) { void reader.cancel().catch(() => {}); throw new Error("request too large"); }
      chunks.push(value);
    }
  } finally { clearTimeout(timer); reader.releaseLock(); }
  const bytes = new Uint8Array(size); let at = 0;
  for (const chunk of chunks) { bytes.set(chunk, at); at += chunk.byteLength; }
  const value = JSON.parse(new TextDecoder("utf-8", {fatal: true}).decode(bytes));
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error("invalid request");
  return value;
}
function safeError(error: unknown): {error: string; status: number} {
  const raw = error instanceof Error ? error.message : String(error);
  // Only task validation and known policy failures are suitable for the browser.
  const known = /^(publication conflict:|task validation failed:|dependency validation failed[:;]|task requests network access|task authoring grant|job is |global concurrency|unknown (job|run|revision)|name required|timeout must|network grants|cron |invalid (cron|timezone|time|request)|request (too large|body timed out)|schedule required|expected_current_revision|restore creates|prepared task|cannot resolve|once |source |task source|task limit|revision limit|deployment task|task scope quota|timezone must|Task dependencies are not supported yet)/i;
  let message = known.test(raw) ? raw : "Request failed. Check the fields and host configuration, then refresh before retrying.";
  message = message.replace(/\x1b\[[0-9;]*m/g, "")
    .replace(/file:\/\/[^\s)\]"']+/g, "[task source]")
    .replace(/(?:\/[A-Za-z0-9_.-]+){2,}(?::\d+(?::\d+)?)?/g, "[task source]")
    .slice(0, 2000);
  const status = /publication conflict|job is already running|global concurrency/.test(raw) ? 409
    : /grant|outside deployment/.test(raw) ? 403
    : /^unknown (job|run|revision)/.test(raw) ? 404
    : /body timed out/.test(raw) ? 408 : 400;
  return {error: message, status};
}
const headers = {"Cache-Control": "no-store", "X-Content-Type-Options": "nosniff", "Content-Security-Policy": "default-src 'self'; script-src 'self'; style-src 'unsafe-inline'; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'"};
export default {
  async fetch(request: Request, context: Context): Promise<Response> {
    const url = new URL(request.url);
    if (request.method === "GET" && assets.has(url.pathname)) {
      const [file, type] = assets.get(url.pathname)!;
      return new Response(await Deno.readTextFile(new URL(file, import.meta.url)), {headers: {...headers, "Content-Type": type}});
    }
    if (request.method !== "POST" || !url.pathname.startsWith("/api/")) return new Response("Not found", {status: 404, headers});
    if (request.headers.get("origin") !== url.origin) return Response.json({error: "Same-origin request required"}, {status: 403, headers});
    try {
      const body = await jsonBody(request);
      const action = url.pathname.slice(5);
      if (action === "ai") {
        if (typeof body.prompt !== "string" || body.prompt.length > 16_000) throw new Error("invalid request");
        const result = await context.ai.complete({prompt: body.prompt});
        return Response.json({text: result.text}, {headers});
      }
      if (!actions.has(action)) return new Response("Not found", {status: 404, headers});
      return Response.json(await context.tasks.command(action, body), {headers});
    } catch (error) {
      const safe = safeError(error);
      return Response.json({error: safe.error}, {status: safe.status, headers});
    }
  },
};
