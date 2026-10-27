"use strict";
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
      ...(command ? { "Content-Type": "application/json" } : {}),
    },
    body: command ? JSON.stringify(command) : undefined,
    credentials: "omit",
    signal: AbortSignal.timeout(path === "/api/plugins" && command ? 180000 : 5000),
  });
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
    const source = element("span", app.uninstallable ? "Dashboard install" : "Local install", "install-source");
    source.title = app.uninstallable
      ? "Downloaded through the dashboard. Stop this app to uninstall it here."
      : "Loaded from your server configuration. Remove it from that configuration to uninstall it.";
    title.append(source);
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
    const actions = {
      running: [["stop", "Stop"], ["restart", "Restart"]],
      stopped: [["start", "Start"]],
      failed: [["start", "Start"]],
      starting: [["stop", "Cancel start"]],
      backoff: [["stop", "Cancel retry"]],
      stopping: [],
    };
    for (const [action, label] of actions[app.state] || []) {
      const button = element("button", label);
      button.type = "button";
      button.setAttribute("aria-label", `${label.toLowerCase()} ${app.name}`);
      button.addEventListener("click", () => update({ action, app: app.name }));
      controls.append(button);
    }
    const logs = element("button", "View logs");
    logs.setAttribute("aria-label", `logs ${app.name}`);
    logs.addEventListener("click", () => showLogs(app.name));
    controls.append(logs);
    if (app.uninstallable) {
      const uninstall = element("button", "Uninstall");
      uninstall.type = "button";
      uninstall.setAttribute("aria-label", `uninstall ${app.name}`);
      uninstall.disabled = app.state !== "stopped";
      uninstall.setAttribute("data-stop-required", String(app.state !== "stopped"));
      uninstall.title = "Stop this plugin before uninstalling. Stored app data is retained.";
      uninstall.addEventListener("click", () => changePlugin({ action: "uninstall", name: app.name }));
      controls.append(uninstall);
    }
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
    list.querySelectorAll("button").forEach(button => { button.disabled = button.getAttribute("data-stop-required") === "true"; });
    timer = setTimeout(() => update(), 2000);
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
    if (version === logVersion) logTimer = setTimeout(() => showLogs(), 5000);
  }
}
document.querySelector("#log-refresh").addEventListener("click", () => showLogs());
logLaunch.addEventListener("change", () => showLogs());

let pluginBusy = false;
const pluginMessage = document.querySelector("#plugin-message");
const pluginList = document.querySelector("#plugins");
async function browsePlugins() {
  if (pluginBusy) return;
  try {
    const catalog = await request(null, "/api/plugins");
    pluginList.replaceChildren(...catalog.apps.map(plugin => {
      const row = element("li", "", "app");
      const info = element("div", "");
      info.append(element("h2", `${plugin.title} ${plugin.version}`), element("p", plugin.description), element("p", `Capabilities: ${plugin.capabilities.join(", ") || "none"}`));
      const install = element("button", "Download and install");
      install.type = "button";
      install.setAttribute("aria-label", `install ${plugin.name} ${plugin.version}`);
      install.addEventListener("click", () => changePlugin({ action: "install", name: plugin.name, version: plugin.version }));
      row.append(info, install);
      return row;
    }));
    pluginMessage.textContent = "Choose a plugin to download and install.";
  } catch (error) { pluginMessage.textContent = error.message; }
}
async function changePlugin(command) {
  if (pluginBusy) return;
  pluginBusy = true;
  pluginList.querySelectorAll("button").forEach(button => button.disabled = true);
  pluginMessage.textContent = command.action === "install" ? `Downloading and verifying ${command.name}…` : `Uninstalling ${command.name}…`;
  try {
    await request(command, "/api/plugins");
    pluginMessage.textContent = `${command.name} ${command.action === "install" ? "installed" : "uninstalled. Stored app data retained"}.`;
    await update();
  } catch (error) { pluginMessage.textContent = error.message; }
  finally {
    pluginBusy = false;
    pluginList.querySelectorAll("button").forEach(button => button.disabled = false);
  }
}
document.querySelector("#catalog-refresh")?.addEventListener("click", browsePlugins);
