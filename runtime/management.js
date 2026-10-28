"use strict";
const summary = document.querySelector("#summary");
const message = document.querySelector("#message");
const list = document.querySelector("#apps");
let reading = false;
const pending = new Set();
const awaiting = new Map();
let connectionError = false;
let lastApps = "";
let inventory = null;
let readVersion = 0;
const rowCache = new Map();
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
  inventory = { apps };
  summary.textContent = `${apps.filter(app => app.state === "running").length} of ${apps.length} apps running`;
  const signature = JSON.stringify([apps, [...pending]]);
  if (signature === lastApps) return;
  lastApps = signature;
  const focused = document.activeElement?.getAttribute("aria-label");
  const rows = apps.map(app => {
    const rowSignature = JSON.stringify([app, pending.has(app.name)]);
    const cached = rowCache.get(app.name);
    if (cached?.signature === rowSignature) return cached.row;
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
      link.setAttribute("aria-label", `open ${app.name}`);
      link.href = app.url;
      link.target = "_blank";
      link.rel = "noopener noreferrer";
      detail.append(link);
    }
    if (app.scheduling_independent) detail.append(element("p", "Scheduling continues when the editor is stopped. Archive jobs in the app to pause future runs, or stop the host to stop all scheduling."));
    const controls = element("div", "", "controls");
    const actions = {
      running: [["stop", "Stop"], ["restart", "Restart"]],
      stopped: [["start", "Start"]],
      failed: [["start", "Start"], ["stop", "Stop"]],
      starting: [["stop", "Cancel start"]],
      backoff: [["stop", "Cancel retry"]],
      stopping: [],
    };
    for (const [action, label] of actions[app.state] || []) {
      const visibleLabel = app.scheduling_independent ? `${label} editor` : label;
      const button = element("button", visibleLabel);
      button.type = "button";
      button.disabled = pending.has(app.name);
      button.setAttribute("aria-label", `${visibleLabel.toLowerCase()} ${app.name}`);
      button.addEventListener("click", () => update({ action, app: app.name }));
      controls.append(button);
    }
    const logs = element("button", "View logs");
    logs.setAttribute("aria-label", `logs ${app.name}`);
    logs.addEventListener("click", () => showLogs(app.name, true));
    controls.append(logs);
    if (app.uninstallable) {
      const uninstall = element("button", "Uninstall");
      uninstall.type = "button";
      uninstall.setAttribute("aria-label", `uninstall ${app.name}`);
      uninstall.disabled = app.state !== "stopped" || pending.has(app.name);
      uninstall.setAttribute("data-stop-required", String(app.state !== "stopped"));
      uninstall.title = "Stop this plugin before uninstalling. Stored app data is retained.";
      uninstall.addEventListener("click", () => changePlugin({ action: "uninstall", name: app.name }));
      controls.append(uninstall);
    }
    row.append(detail, controls);
    rowCache.set(app.name, { signature: rowSignature, row });
    return row;
  });
  for (const name of rowCache.keys()) if (!apps.some(app => app.name === name)) rowCache.delete(name);
  list.replaceChildren(...(rows.length ? rows : [element("li", "No apps configured yet. Browse the plugin catalog below, or add a local app to your server configuration.", "empty")]));
  if (focused) {
    const controls = [...list.querySelectorAll("button"), ...list.querySelectorAll("a")];
    const focusedApp = apps.find(app => focused.endsWith(` ${app.name}`));
    const target = controls.find(node => node.getAttribute("aria-label") === focused && !node.disabled)
      || (focusedApp && controls.find(node => node.getAttribute("aria-label") === `logs ${focusedApp.name}`));
    target?.focus();
  }
}

async function update(command) {
  if (command) {
    if (pending.has(command.app)) return;
    pending.add(command.app);
    awaiting.delete(command.app);
    connectionError = false;
    ++readVersion;
    list.querySelectorAll("button").forEach(button => {
      if (button.getAttribute("aria-label")?.endsWith(` ${command.app}`) && !button.getAttribute("aria-label").startsWith("logs ")) button.disabled = true;
    });
    message.textContent = `Requesting ${command.action} for ${command.app}…`;
    try {
      await request(command);
      awaiting.set(command.app, command.action);
      message.textContent = `${command.action} requested for ${command.app}. Waiting for status.`;
    } catch (error) { message.textContent = error.message; }
    finally { pending.delete(command.app); rowCache.delete(command.app); lastApps = ""; if (inventory) render(inventory); }
  } else if (reading) return;
  clearTimeout(timer);
  reading = true;
  const version = ++readVersion;
  try {
    const data = await request();
    if (version === readVersion) {
      if (connectionError) { message.textContent = ''; connectionError = false; }
      for (const [name, action] of awaiting) {
        const app = data.apps.find(app => app.name === name);
        if (!app || app.state === 'failed' || app.state === (action === 'stop' ? 'stopped' : 'running')) {
          awaiting.delete(name);
          message.textContent = !app ? `${name} is no longer installed.` : app.state === 'failed' ? `${name} failed: ${app.error || 'See logs.'}` : `${name}: ${action} completed.`;
        }
      }
      render(data);
    }
  }
  catch (error) { if (version === readVersion) { connectionError = true; message.textContent = `Connection unavailable: ${error.message}`; } }
  finally {
    reading = false;
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
const logStatus = document.querySelector("#log-status");
async function showLogs(app = logApp, explicit = false) {
  if (app !== logApp) logLaunch.value = "";
  logApp = app;
  const version = ++logVersion;
  clearTimeout(logTimer);
  logPanel.hidden = false;
  logTitle.textContent = `Logs: ${app}`;
  if (explicit) {
    logLaunch.focus();
    logPanel.scrollIntoView?.({ behavior: "smooth", block: "start" });
  }
  if (!logOutput.textContent || app !== logOutput.dataset?.app) logOutput.textContent = "Loading logs…";
  if (logOutput.dataset) logOutput.dataset.app = app;
  try {
    const launch = logLaunch.value.trim();
    const data = await request(null, `/api/logs?app=${encodeURIComponent(app)}&limit=100${launch ? `&run_id=${encodeURIComponent(launch)}` : ""}`);
    if (version !== logVersion) return;
    logStatus.textContent = "";
    const loss = Object.entries(data.store_losses || {}).filter(([, value]) => value).map(([key, value]) => `${key}=${value}`).join(", ");
    const consoleLoss = Object.entries(data.console_losses || {}).filter(([, value]) => value).map(([key, value]) => `${key}=${value}`).join(", ");
    const records = data.records.length ? data.records.map(record =>
      `${new Date(record.timestamp_unix_ms).toISOString()} [${record.run_id}] ${record.stream}/${record.event}${record.truncated ? " [truncated]" : ""}: ${record.message}`
    ).join("\n") : "No retained logs match this app and launch.";
    const next = `${loss ? `Store loss counters (runtime scope): ${loss}\n` : ""}${consoleLoss ? `Console display counters (process scope): ${consoleLoss}\n` : ""}${records}`;
    const selection = document.getSelection?.();
    if (next !== logOutput.textContent && !(selection && !selection.isCollapsed && logOutput.contains(selection.anchorNode))) {
      const top = logOutput.scrollTop;
      const left = logOutput.scrollLeft;
      logOutput.textContent = next;
      logOutput.scrollTop = top;
      logOutput.scrollLeft = left;
    }
  } catch (error) {
    if (version === logVersion) logStatus.textContent = `Unable to load logs: ${error.message}`;
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
      install.disabled = plugin.installed || (plugin.installable === false && !plugin.setup_available);
      if (plugin.setup_available && !plugin.installed) install.textContent = "Set up and install";
      if (plugin.installed) install.textContent = "Installed";
      if (plugin.installable === false) {
        const reason = plugin.install_reason || "This plugin requires host configuration.";
        info.append(element("p", reason));
        install.title = reason;
      }
      install.addEventListener("click", () => {
        if (!plugin.setup_available) return changePlugin({ action: "install", name: plugin.name, version: plugin.version });
        const setup = {approved: true, task_authoring: plugin.capabilities.includes('tasks'), task_network: []};
        if (plugin.capabilities.includes('ai')) {
          const path = prompt('AI setup: enter the absolute path to your host AI routing configuration. Leave blank for local operation without a provider. Credentials stay in the host configuration.');
          if (path === null) return;
          if (path.trim()) setup.ai_config = path.trim();
        }
        if (setup.task_authoring) {
          const hosts = prompt('Task setup: enter allowed network hosts, separated by commas (for example api.example.com:443). Leave blank to deny task network access.');
          if (hosts === null) return;
          setup.task_network = hosts.split(',').map(host => host.trim()).filter(Boolean);
        }
        if (!confirm(`Install ${plugin.name} ${plugin.version}? Approve capabilities: ${plugin.capabilities.join(', ')}. Task network: ${setup.task_network.join(', ') || 'none'}. AI configuration: ${setup.ai_config || 'no provider configured'}. Published jobs may run while the editor is stopped.`)) return;
        return changePlugin({action: 'setup', name: plugin.name, version: plugin.version, setup});
      });
      row.append(info, install);
      return row;
    }));
    pluginMessage.textContent = catalog.apps.length ? "Choose a plugin to download and install." : "No plugins are available in the catalog.";
  } catch (error) { pluginMessage.textContent = error.message; }
}
async function changePlugin(command) {
  if (pluginBusy) return;
  pluginBusy = true;
  if (command.action === "uninstall") {
    pending.add(command.name);
    list.querySelectorAll("button").forEach(button => {
      if (button.getAttribute("aria-label")?.endsWith(` ${command.name}`) && !button.getAttribute("aria-label").startsWith("logs ")) button.disabled = true;
    });
  }
  pluginList.querySelectorAll("button").forEach(button => button.disabled = true);
  pluginMessage.textContent = command.action !== "uninstall" ? `Downloading and verifying ${command.name}…` : `Uninstalling ${command.name}…`;
  try {
    await request(command, "/api/plugins");
    pluginMessage.textContent = `${command.name} ${command.action !== "uninstall" ? "installed" : "uninstalled. Stored app data retained"}.`;
    await update();
    pluginBusy = false;
    await browsePlugins();
    pluginMessage.textContent = `${command.name} ${command.action !== "uninstall" ? "installed" : "uninstalled. Stored app data retained"}.`;
  } catch (error) {
    pluginBusy = false;
    const reason = error.message;
    await browsePlugins();
    pluginMessage.textContent = reason;
  }
  finally {
    pluginBusy = false;
    if (command.action === "uninstall") {
      pending.delete(command.name);
      rowCache.delete(command.name);
      lastApps = "";
      if (inventory) render(inventory);
      await update();
    }
  }
}
document.querySelector("#catalog-refresh")?.addEventListener("click", browsePlugins);
