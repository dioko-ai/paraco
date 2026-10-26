import type { AppContext, ArtifactReference, ParacoApp } from "../../runtime/paraco";

const page = `<!doctype html>
<html lang="en">
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Artifact Notebook</title>
<style>
  body{font:16px system-ui,sans-serif;max-width:760px;margin:2rem auto;padding:0 1rem;color:#18212b}
  label{display:block;margin:.8rem 0}input,textarea,button{font:inherit;padding:.55rem}input,textarea{box-sizing:border-box;width:100%}
  textarea{min-height:6rem}button{cursor:pointer}li{margin:.6rem 0}pre{white-space:pre-wrap;overflow-wrap:anywhere;background:#f2f4f7;padding:1rem}
  .row{display:flex;gap:.6rem;align-items:center}.row input{flex:1}#message{min-height:1.5em}
</style>
<h1>Artifact Notebook</h1>
<p>Publish reports, evaluate the latest report, and inspect artifact lineage in a dataset.</p>
<label>Dataset alias <input id="dataset" value="private" autocomplete="off"></label>
<form id="report-form">
  <label>Report content <textarea id="report" required maxlength="60000" placeholder="Write a report to publish"></textarea></label>
  <button type="submit">Publish report</button>
</form>
<p><button id="evaluate" type="button">Evaluate latest report</button> <button id="refresh" type="button">Refresh artifacts</button></p>
<p id="message" role="status" aria-live="polite"></p>
<h2>Artifacts</h2><ol id="artifacts"></ol>
<h2>Lineage</h2><pre id="lineage">Select an artifact to inspect its lineage.</pre>
<script>
(() => {
  const dataset = document.querySelector('#dataset');
  const report = document.querySelector('#report');
  const form = document.querySelector('#report-form');
  const message = document.querySelector('#message');
  const list = document.querySelector('#artifacts');
  const lineage = document.querySelector('#lineage');
  const alias = () => dataset.value.trim() || 'private';
  const api = async (path, options = {}) => {
    const url = new URL(path, location.href);
    url.searchParams.set('dataset', alias());
    const response = await fetch(url, { ...options, credentials: 'same-origin', headers: {
      ...(options.body ? { 'Content-Type': 'application/json' } : {}), ...options.headers,
    }});
    const body = await response.json();
    if (!response.ok) throw new Error(body.error || 'Request failed');
    return body;
  };
  const showLineage = async id => {
    try {
      const data = await api('/api/lineage', { method: 'POST', body: JSON.stringify({ id }) });
      lineage.textContent = JSON.stringify(data, null, 2);
    } catch (error) { message.textContent = error.message; }
  };
  const refresh = async () => {
    try {
      const { artifacts } = await api('/api/artifacts');
      list.replaceChildren();
      for (const artifact of artifacts) {
        const item = document.createElement('li');
        const button = document.createElement('button');
        button.type = 'button';
        button.textContent = artifactLabel(artifact);
        button.addEventListener('click', () => { void showLineage(artifact.id); });
        item.append(button);
        list.append(item);
      }
      if (!artifacts.length) {
        const empty = document.createElement('li');
        empty.textContent = 'No artifacts in this dataset yet.';
        list.append(empty);
      }
    } catch (error) { message.textContent = error.message; }
  };
  const artifactLabel = artifact => {
    const date = new Date(artifact.createdAt).toLocaleString();
    return '#' + artifact.sequence + ' ' + artifact.output + ' · ' + date + ' · ' + artifact.id;
  };
  form.addEventListener('submit', async event => {
    event.preventDefault();
    message.textContent = 'Publishing report…';
    try {
      const result = await api('/api/reports', { method: 'POST', body: JSON.stringify({ content: report.value }) });
      report.value = '';
      message.textContent = 'Published report ' + result.artifact.id;
      await refresh();
    } catch (error) { message.textContent = error.message; }
  });
  document.querySelector('#evaluate').addEventListener('click', async () => {
    message.textContent = 'Resolving and evaluating the latest report…';
    try {
      const result = await api('/api/evaluate', { method: 'POST', body: '{}' });
      message.textContent = 'Published evaluation ' + result.artifact.id + ' from report ' + result.report.id;
      await refresh();
      await showLineage(result.artifact.id);
    } catch (error) { message.textContent = error.message; }
  });
  document.querySelector('#refresh').addEventListener('click', () => { void refresh(); });
  dataset.addEventListener('change', () => { void refresh(); });
  const initial = new URL(location.href).searchParams.get('dataset');
  if (initial) dataset.value = initial;
  void refresh();
})();
</script>`;

function json(value: unknown, status = 200): Response {
  return new Response(JSON.stringify(value), { status, headers: { "Content-Type": "application/json; charset=utf-8", "Cache-Control": "no-store" } });
}

function asObject(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value) ? value as Record<string, unknown> : null;
}

async function body(request: Request): Promise<Record<string, unknown> | null> {
  try { return asObject(await request.json()); } catch { return null; }
}

function datasetAlias(url: URL): string {
  return url.searchParams.get("dataset")?.trim() || "private";
}

function escapedId(id: string): boolean {
  return id.length > 0 && new TextEncoder().encode(id).byteLength <= 128;
}

const app: ParacoApp = {
  async fetch(request: Request, context: AppContext): Promise<Response> {
    const url = new URL(request.url);
    if (request.method === "GET" && url.pathname === "/") {
      return new Response(page, { headers: { "Content-Type": "text/html; charset=utf-8", "Cache-Control": "no-store" } });
    }
    if (!["GET", "POST"].includes(request.method) || !url.pathname.startsWith("/api/")) return json({ error: "Not found" }, 404);
    if (request.method === "POST" && request.headers.get("origin") !== url.origin) return json({ error: "Same-origin request required" }, 403);
    if (!context.datasets) return json({ error: "Storage capability is unavailable" }, 503);

    try {
      const dataset = context.datasets.open(datasetAlias(url));
      if (request.method === "GET" && url.pathname === "/api/artifacts") {
        const artifacts = await dataset.artifacts.list({ limit: 100 });
        return json({ artifacts: artifacts.slice().reverse() });
      }
      if (request.method !== "POST") return json({ error: "Not found" }, 404);
      const value = await body(request);
      if (!value) return json({ error: "Expected a JSON object" }, 400);
      if (url.pathname === "/api/reports") {
        if (typeof value.content !== "string" || !value.content.trim() || new TextEncoder().encode(value.content).byteLength > 60 * 1024) {
          return json({ error: "Report content must contain 1–60 KiB of text" }, 400);
        }
        const artifact = await dataset.artifacts.publish({
          output: "report", mediaType: "text/plain", content: value.content,
          project: "artifact-notebook", tags: ["report"], status: "complete",
        });
        return json({ artifact }, 201);
      }
      if (url.pathname === "/api/evaluate") {
        const reference: ArtifactReference = { kind: "dynamic", query: { output: "report", project: "artifact-notebook", limit: 1 } };
        const executionId = crypto.randomUUID();
        const [report] = await dataset.artifacts.resolve(reference, { executionId, slot: "latest-report" });
        if (!report) return json({ error: "Publish a report before evaluating" }, 404);
        const content = typeof report.content === "string" ? report.content : JSON.stringify(report.content);
        const artifact = await dataset.artifacts.publish({
          output: "evaluation", mediaType: "application/json",
          content: { reportId: report.id, executionId, characters: content.length, nonEmpty: content.trim().length > 0, preview: content.slice(0, 160) },
          project: "artifact-notebook", tags: ["evaluation"], status: "complete",
          inputs: [report.id], executionId,
        });
        return json({ artifact, report }, 201);
      }
      if (url.pathname === "/api/lineage") {
        if (typeof value.id !== "string" || !escapedId(value.id)) return json({ error: "Artifact ID is required" }, 400);
        return json(await dataset.artifacts.lineage(value.id));
      }
      return json({ error: "Not found" }, 404);
    } catch (error) {
      return json({ error: error instanceof Error ? error.message : String(error) }, 400);
    }
  },
};

export default app;
