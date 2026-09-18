"use strict";
// A fragment never reaches the HTTP server. Keep authorization out of cookies,
// links, and app origins; sessionStorage preserves it only in this browser tab.
const fragment = location.hash.slice(1);
history.replaceState(null, "", "/");
let token = fragment;
try {
  token ||= sessionStorage.getItem("paraco-token") || "";
  if (fragment) sessionStorage.setItem("paraco-token", fragment);
} catch { /* The current tab still works when browser storage is disabled. */ }
const summary = document.querySelector("#summary");
const message = document.querySelector("#message");
const list = document.querySelector("#apps");
let busy = false;
let timer;

function element(tag, text, className) {
  const node = document.createElement(tag);
  node.textContent = text;
  if (className) node.className = className;
  return node;
}

async function request(command, path = "/api/apps") {
  const response = await fetch(path, {
    method: command ? "POST" : "GET",
    headers: {
      Authorization: `Bearer ${token}`,
      ...(command ? { "Content-Type": "application/json" } : {}),
    },
    body: command ? JSON.stringify(command) : undefined,
    credentials: "omit",
    signal: AbortSignal.timeout(5000),
  });
  if (response.status === 401) {
    try { sessionStorage.removeItem("paraco-token"); } catch { /* Storage may be disabled. */ }
    token = "";
    throw new Error("Open the management link printed by the running server to authorize this tab.");
  }
  if (!response.ok) throw new Error(`Request failed (${response.status}): ${await response.text()}`);
  return await response.json();
}

function render({ apps }) {
  summary.textContent = `${apps.filter(app => app.state === "running").length} of ${apps.length} apps running`;
  const focused = document.activeElement?.getAttribute("aria-label");
  const rows = apps.map(app => {
    const row = element("li", "", "app");
    const title = element("div", "");
    title.append(element("h2", app.name), element("p", `Desired: ${app.desired}`, "path"));
    row.append(title, element("span", app.state, `status ${app.state}`));
    const detail = element("div", app.error || "", "detail");
    if (app.state === "running") {
      const link = element("a", "Open app ↗");
      link.href = app.url;
      link.target = "_blank";
      link.rel = "noopener noreferrer";
      detail.append(link);
    }
    const controls = element("div", "", "controls");
    for (const action of ["start", "stop", "restart"]) {
      const button = element("button", action[0].toUpperCase() + action.slice(1));
      button.type = "button";
      button.setAttribute("aria-label", `${action} ${app.name}`);
      button.addEventListener("click", () => update({ action, app: app.name }));
      controls.append(button);
    }
    const logs = element("button", "View logs");
    logs.setAttribute("aria-label", `logs ${app.name}`);
    logs.addEventListener("click", () => showLogs(app.name));
    controls.append(logs);
    row.append(detail, controls);
    return row;
  });
  list.replaceChildren(...(rows.length ? rows : [element("li", "No apps configured yet.", "empty")]));
  if (focused) {
    [...list.querySelectorAll("button")].find(button => button.getAttribute("aria-label") === focused)?.focus();
  }
}

async function update(command) {
  if (busy) return;
  busy = true;
  clearTimeout(timer);
  list.querySelectorAll("button").forEach(button => button.disabled = true);
  try {
    if (!token) throw new Error("Open the management link printed by the running server to authorize this tab.");
    if (command) {
      await request(command);
      message.textContent = `${command.action} requested for ${command.app}. Waiting for status.`;
    }
    render(await request());
    if (!command) message.textContent = "Status updated.";
  } catch (error) {
    message.textContent = error.message;
  } finally {
    busy = false;
    list.querySelectorAll("button").forEach(button => button.disabled = false);
    if (token) timer = setTimeout(() => update(), 2000);
  }
}
update();

let logApp = "";
let logVersion = 0;
let logTimer;
const logPanel = document.querySelector("#logs");
const logTitle = document.querySelector("#log-title");
const logOutput = document.querySelector("#log-output");
const logLaunch = document.querySelector("#log-launch");
async function showLogs(app = logApp) {
  if (app !== logApp) logLaunch.value = "";
  logApp = app;
  const version = ++logVersion;
  clearTimeout(logTimer);
  logPanel.hidden = false;
  logTitle.textContent = `Logs: ${app}`;
  logOutput.textContent = "Loading logs…";
  try {
    const launch = logLaunch.value.trim();
    const data = await request(null, `/api/logs?app=${encodeURIComponent(app)}&limit=100${launch ? `&run_id=${encodeURIComponent(launch)}` : ""}`);
    if (version !== logVersion) return;
    const loss = Object.entries(data.store_losses || {}).filter(([, value]) => value).map(([key, value]) => `${key}=${value}`).join(", ");
    const consoleLoss = Object.entries(data.console_losses || {}).filter(([, value]) => value).map(([key, value]) => `${key}=${value}`).join(", ");
    const records = data.records.length ? data.records.map(record =>
      `${new Date(record.timestamp_unix_ms).toISOString()} [${record.run_id}] ${record.stream}/${record.event}${record.truncated ? " [truncated]" : ""}: ${record.message}`
    ).join("\n") : "No retained logs match this app and launch.";
    logOutput.textContent = `${loss ? `Store loss counters (runtime scope): ${loss}\n` : ""}${consoleLoss ? `Console display counters (process scope): ${consoleLoss}\n` : ""}${records}`;
  } catch (error) {
    if (version === logVersion) logOutput.textContent = `Unable to load logs: ${error.message}`;
  } finally {
    if (version === logVersion && token) logTimer = setTimeout(() => showLogs(), 5000);
  }
}
document.querySelector("#log-refresh").addEventListener("click", () => showLogs());
logLaunch.addEventListener("change", () => showLogs());
