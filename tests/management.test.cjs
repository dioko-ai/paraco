// Run with node --test tests/management.test.cjs. No npm dependencies required.
const { test } = require('node:test');
const assert = require('node:assert/strict');
const vm = require('node:vm');
const fs = require('node:fs');
const source = fs.readFileSync(new URL('../runtime/management.js', `file://${__filename}`), 'utf8');

function browser({ hash = '#secret', status = 200, error = null, stored = null, storageDisabled = false } = {}) {
  const requests = [], timers = [], history = [], storage = new Map(stored ? [['paraco-token', stored]] : []);
  let document;
  class Element {
    constructor(tag) { this.tag = tag; this.children = []; this.attributes = {}; this.listeners = {}; this.dataset = {}; }
    append(...nodes) { this.children.push(...nodes); }
    replaceChildren(...nodes) { this.children = nodes; }
    setAttribute(name, value) { this.attributes[name] = value; }
    getAttribute(name) { return this.attributes[name]; }
    addEventListener(name, fn) { this.listeners[name] = fn; }
    querySelectorAll(tag) { return this.children.flatMap(child => [...(child.tag === tag ? [child] : []), ...child.querySelectorAll(tag)]); }
    focus() { document.activeElement = this; }
  }
  const nodes = { '#summary': new Element('p'), '#message': new Element('p'), '#apps': new Element('ul') };
  for (const id of ['logs', 'log-title', 'log-output', 'log-status', 'log-launch', 'log-refresh', 'catalog-refresh', 'plugin-message', 'plugins']) { nodes['#' + id] = new Element('div'); nodes['#' + id].value = ''; }
  document = { querySelector: id => nodes[id], createElement: tag => new Element(tag), activeElement: null };
  const context = vm.createContext({
    document, location: { hash }, history: { replaceState: (...args) => history.push(args) },
    sessionStorage: {
      getItem: key => { if (storageDisabled) throw Error('disabled'); return storage.get(key); },
      setItem: (key, value) => { if (storageDisabled) throw Error('disabled'); storage.set(key, value); },
      removeItem: key => storage.delete(key),
    },
    AbortSignal, setTimeout: fn => timers.push(fn), clearTimeout() {},
    fetch: async (url, options) => {
      requests.push({ url, ...options });
      return { status, ok: status === 200, text: async () => 'Denied', json: async () => ({
        apps: [{ name: 'hello', url: 'http://hello.localhost:3000/', desired: 'running', state: error ? 'failed' : 'running', error }],
      }) };
    },
  });
  vm.runInContext(source, context);
  return { nodes, requests, timers, history, storage, document, context };
}
const flush = () => new Promise(resolve => setImmediate(resolve));

test('plain dashboard loads lifecycle buttons, polls and preserves keyboard focus', async () => {
  const b = browser();
  await flush();
  assert.equal(b.requests[0].headers.Authorization, undefined);
  assert.equal(b.requests[0].credentials, 'omit');
  const buttons = b.nodes['#apps'].querySelectorAll('button');
  assert.equal(buttons.length, 3);
  const link = b.nodes['#apps'].querySelectorAll('a')[0];
  assert.equal(link.href, 'http://hello.localhost:3000/');
  assert.equal(link.rel, 'noopener noreferrer');
  buttons[0].focus();
  await buttons[0].listeners.click();
  const post = b.requests.find(request => request.method === 'POST');
  assert.deepEqual(JSON.parse(post.body), { action: 'stop', app: 'hello' });
  assert.equal(post.headers['Content-Type'], 'application/json');
  assert.equal(b.document.activeElement.getAttribute('aria-label'), 'stop hello');
  assert.match(b.nodes['#message'].textContent, /stop requested/);
  assert.equal(b.requests.at(-1).method, 'GET');
});

test('plain URL works without a token and errors allow polling to recover', async () => {
  const plain = browser({hash: '', storageDisabled: true});
  const failed = browser({hash: '', status: 503});
  await flush();
  assert.equal(plain.requests.length, 1);
  assert.equal(plain.nodes['#apps'].querySelectorAll('button').length, 3);
  assert.match(failed.nodes['#message'].textContent, /503/);
  assert.equal(failed.timers.length, 1);
});

test('error output is rendered as text and storage-disabled tabs still work', async () => {
  const error = '<img src=x onerror=alert(1)>';
  const b = browser({ error, storageDisabled: true });
  await flush();
  assert.equal(b.requests.length, 1);
  const detail = b.nodes['#apps'].children[0].children[2];
  assert.equal(detail.textContent, error);
  assert.equal(detail.children.length, 0);
});

test('logs render hostile content as text, filter launches, refresh and show empty/error states', async () => {
  const b = browser();
  await flush();
  vm.runInContext(`request = async (_, path) => { globalThis.logPath = path; return {records: [{timestamp_unix_ms: 0, run_id: "abc", stream: "stderr", event: "output", message: "<script>alert(1)</script>"}]}; }`, b.context);
  await b.nodes['#apps'].querySelectorAll('button')[2].listeners.click();
  assert.match(b.nodes['#log-output'].textContent, /<script>/);
  assert.equal(b.nodes['#log-output'].children.length, 0);
  b.nodes['#log-launch'].value = 'a'.repeat(32);
  await b.nodes['#log-refresh'].listeners.click();
  assert.match(b.context.logPath, /run_id=aaaaaaaa/);
  vm.runInContext('request = async () => ({records: []})', b.context);
  await b.nodes['#log-refresh'].listeners.click();
  assert.match(b.nodes['#log-output'].textContent, /No retained logs/);
  vm.runInContext('request = async () => { throw Error("offline"); }', b.context);
  await b.nodes['#log-refresh'].listeners.click();
  assert.match(b.nodes['#log-status'].textContent, /Unable to load logs: offline/);
  assert.match(b.nodes['#log-output'].textContent, /No retained logs/);
});

test('catalog displays capabilities and installs the selected exact version', async () => {
  const b = browser();
  await flush();
  vm.runInContext(`request = async (command, path) => {
    if (command) { globalThis.pluginCommand = command; return {}; }
    if (path === "/api/plugins") return {apps: [{name: "sample", title: "Sample", version: "1.2.3", description: "<script>unsafe</script>", capabilities: ["system"]}]};
    return {apps: []};
  }`, b.context);
  await b.nodes['#catalog-refresh'].listeners.click();
  assert.equal(b.nodes['#plugins'].children[0].children[0].children[1].textContent, '<script>unsafe</script>');
  assert.match(b.nodes['#plugins'].children[0].children[0].children[2].textContent, /system/);
  await b.nodes['#plugins'].querySelectorAll('button')[0].listeners.click();
  assert.deepEqual(JSON.parse(JSON.stringify(b.context.pluginCommand)), {action: 'install', name: 'sample', version: '1.2.3'});
  assert.match(b.nodes['#plugin-message'].textContent, /installed/);
});

test('uninstall requires a stopped downloaded plugin and reports failures', async () => {
  const b = browser();
  await flush();
  vm.runInContext('render({apps: [{name: "sample", desired: "running", state: "running", uninstallable: true}]})', b.context);
  assert.equal(b.nodes['#apps'].querySelectorAll('button').at(-1).disabled, true);
  vm.runInContext('render({apps: [{name: "sample", desired: "stopped", state: "stopped", uninstallable: true}]}); request = async () => { throw Error("Disk unavailable"); }', b.context);
  const uninstall = b.nodes['#apps'].querySelectorAll('button').at(-1);
  assert.equal(uninstall.disabled, false);
  await uninstall.listeners.click();
  assert.match(b.nodes['#plugin-message'].textContent, /Disk unavailable/);
});

test('install source labels distinguish local configuration from dashboard downloads', async () => {
  const b = browser();
  await flush();
  for (const uninstallable of [false, true]) {
    vm.runInContext(`render({apps: [{name: "sample", desired: "stopped", state: "stopped", uninstallable: ${uninstallable}}]})`, b.context);
    const label = b.nodes['#apps'].querySelectorAll('span').find(node => node.className === 'install-source');
    assert.equal(label.textContent, uninstallable ? 'Dashboard install' : 'Local install');
    assert.match(label.title, uninstallable ? /uninstall it here/ : /server configuration/);
    const buttons = b.nodes['#apps'].querySelectorAll('button');
    assert.equal(buttons.some(button => button.textContent === 'Uninstall'), uninstallable);
  }
});

test('lifecycle controls match running, stopped, failed, and transitional states', async () => {
  const b = browser();
  await flush();
  for (const [state, labels] of Object.entries({
    running: ['Stop', 'Restart', 'View logs'],
    stopped: ['Start', 'View logs'],
    failed: ['Start', 'Stop', 'View logs'],
    starting: ['Cancel start', 'View logs'],
    backoff: ['Cancel retry', 'View logs'],
    stopping: ['View logs'],
  })) {
    vm.runInContext(`render({apps: [{name: "sample", desired: "running", state: "${state}"}]})`, b.context);
    assert.deepEqual(b.nodes['#apps'].querySelectorAll('button').map(button => button.textContent), labels, state);
  }
  vm.runInContext('render({apps: [{name: "sample", state: "starting"}]}); update = async command => { globalThis.cancelCommand = command; }', b.context);
  await b.nodes['#apps'].querySelectorAll('button')[0].listeners.click();
  assert.deepEqual(JSON.parse(JSON.stringify(b.context.cancelCommand)), {action: 'stop', app: 'sample'});
});


test('polling preserves unchanged rows and open link focus', async () => {
  const b = browser();
  await flush();
  const row = b.nodes['#apps'].children[0];
  const link = b.nodes['#apps'].querySelectorAll('a')[0];
  link.focus();
  await vm.runInContext('update()', b.context);
  assert.equal(b.nodes['#apps'].children[0], row);
  assert.equal(b.document.activeElement, link);
});

test('catalog explains unsupported plugins and marks installed plugins', async () => {
  const b = browser();
  await flush();
  vm.runInContext(`request = async () => ({apps: [
    {name: "a", title: "A", version: "1", description: "", capabilities: ["ai"], installable: false, install_reason: "Configure AI on the host"},
    {name: "b", title: "B", version: "1", description: "", capabilities: [], installable: true, installed: true}
  ]})`, b.context);
  await b.nodes['#catalog-refresh'].listeners.click();
  const buttons = b.nodes['#plugins'].querySelectorAll('button');
  assert.equal(buttons[0].disabled, true);
  assert.match(buttons[0].title, /Configure AI/);
  assert.equal(buttons[1].textContent, 'Installed');
  assert.equal(buttons[1].disabled, true);
});

test('failed downloaded plugin can be stopped before uninstall', async () => {
  const b = browser();
  await flush();
  vm.runInContext('render({apps: [{name: "sample", desired: "running", state: "failed", uninstallable: true}]}); update = async command => { globalThis.command = command; }', b.context);
  const buttons = b.nodes['#apps'].querySelectorAll('button');
  assert.equal(buttons.at(-1).disabled, true);
  await buttons.find(button => button.textContent === 'Stop').listeners.click();
  assert.deepEqual(JSON.parse(JSON.stringify(b.context.command)), {action: 'stop', app: 'sample'});
});

test('dashboard interactions survive polling and failed mutations in Chromium', {skip: !process.env.RUN_DASHBOARD_BROWSER}, async () => {
  const { chromium } = require('@playwright/test');
  const html = fs.readFileSync(new URL('../runtime/dashboard.html', `file://${__filename}`), 'utf8').replace('<script src="/dashboard.js" defer></script>', '');
  const browser = await chromium.launch({headless: true});
  try {
    const page = await browser.newPage();
    await page.setContent(html);
    await page.evaluate(() => {
      window.fixture = {
        apps: [{name:'hello', state:'running', desired:'running', url:'http://hello.localhost/', uninstallable:true}, {name:'other', state:'stopped', desired:'stopped'}],
        plugins: [{name:'ai', title:'AI', version:'1', description:'', capabilities:['ai'], installable:false, install_reason:'Configure AI on the host'}, {name:'hello', title:'Hello', version:'1', description:'', capabilities:[], installed:true}, {name:'new', title:'New', version:'1', description:'', capabilities:[], installable:true}],
        mutation: null, reads: 0, logMessage:'initial log', failReads:false, failLogs:false,
      };
      window.fetch = async (path, options) => {
        if (options.method === 'POST') {
          await new Promise((resolve, reject) => { window.fixture.mutation = {resolve, reject}; });
          return {ok:true, json:async()=>({})};
        }
        if (path === '/api/plugins') return {ok:true, json:async()=>({apps:window.fixture.plugins})};
        if (path.startsWith('/api/logs') && window.fixture.failLogs) throw Error('logs offline');
        if (path.startsWith('/api/logs')) return {ok:true, json:async()=>({records:[{timestamp_unix_ms:0, run_id:'x', stream:'stdout', event:'output', message:window.fixture.logMessage}]})};
        window.fixture.reads++;
        if (window.fixture.failReads) throw Error('status offline');
        return {ok:true, json:async()=>({apps:structuredClone(window.fixture.apps)})};
      };
    });
    await page.addScriptTag({content: source});
    const link = page.getByRole('link', {name:'open hello'});
    await link.waitFor();
    await link.focus();
    await page.waitForFunction(() => window.fixture.reads >= 2, null, {timeout:5000});
    assert.equal(await link.evaluate(node => document.activeElement === node), true);
    await page.evaluate(() => { window.fixture.apps[1].state = 'failed'; });
    await page.waitForFunction(() => document.querySelector('[aria-label="stop other"]'), null, {timeout:5000});
    assert.equal(await link.evaluate(node => document.activeElement === node), true);
    await page.getByRole('button', {name:'stop other'}).focus();
    await page.evaluate(() => { window.fixture.apps[1].state = 'stopped'; });
    await page.waitForFunction(() => document.activeElement?.getAttribute('aria-label') === 'logs other', null, {timeout:5000});
    await page.getByRole('button', {name:'Browse plugins'}).click();
    assert.equal(await page.getByRole('button', {name:'install ai 1'}).isDisabled(), true);
    assert.match(await page.locator('#plugins').innerText(), /Configure AI on the host/);
    assert.equal(await page.getByRole('button', {name:'install hello 1'}).innerText(), 'Installed');
    // Check actual layout: absence of overflow alone missed the unusably narrow cards.
    await page.setViewportSize({width:390,height:844});
    const mobileCard = page.locator('#plugins .app').last();
    const infoBox = await mobileCard.locator('div').first().boundingBox();
    const installBox = await mobileCard.locator('button').boundingBox();
    assert.ok(infoBox.width > 280, 'catalog text uses the mobile card width');
    assert.ok(installBox.y >= infoBox.y + infoBox.height, 'install action sits below its description');
    assert.ok(installBox.height < 80, 'install action does not stretch to card height');
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false);
    await page.setViewportSize({width:1280,height:720});
    const stop = page.getByRole('button', {name:'stop hello'});
    await stop.click();
    assert.equal(await stop.isDisabled(), true);
    assert.equal(await page.getByRole('button', {name:'start other'}).isDisabled(), false);
    await page.evaluate(() => { window.fixture.failReads = true; });
    await page.evaluate(() => window.fixture.mutation.reject(new Error('Timed out')));
    await page.getByText('status offline').waitFor();
    await page.waitForFunction(() => !document.querySelector('[aria-label="stop hello"]').disabled);
    await page.evaluate(() => { window.fixture.failReads = false; });
    await stop.click();
    await page.evaluate(() => window.fixture.mutation.resolve());
    await page.getByText('stop requested for hello', {exact:false}).waitFor();
    const install = page.getByRole('button', {name:'install new 1'});
    await install.click();
    assert.equal(await install.isDisabled(), true);
    await page.evaluate(() => window.fixture.mutation.reject(new Error('Download timed out')));
    await page.getByText('Download timed out').waitFor();
    assert.equal(await install.isDisabled(), false);
    await install.click();
    await page.evaluate(() => { window.fixture.plugins[2].installed = true; window.fixture.mutation.resolve(); });
    await page.getByText('new installed.', {exact:true}).waitFor();
    assert.equal(await page.getByRole('button', {name:'install new 1'}).innerText(), 'Installed');
    await page.getByRole('button', {name:'logs hello'}).click();
    assert.equal(await page.locator('#log-launch').evaluate(node => document.activeElement === node), true);
    await page.getByText(/initial log/).waitFor();
    await page.evaluate(() => {
      const output = document.querySelector('#log-output');
      const range = document.createRange(); range.selectNodeContents(output);
      const selection = window.getSelection(); selection.removeAllRanges(); selection.addRange(range);
      window.fixture.logMessage = 'updated log';
    });
    await page.evaluate(() => { window.fixture.failLogs = true; });
    await page.getByRole('button', {name:'Refresh logs'}).click();
    await page.getByText('Unable to load logs: logs offline').waitFor();
    assert.match(await page.locator('#log-output').innerText(), /initial log/);
    assert.match(await page.evaluate(() => window.getSelection().toString()), /initial log/);
    await page.evaluate(() => { window.fixture.failLogs = false; });
    await page.getByRole('button', {name:'Refresh logs'}).click();
    assert.match(await page.locator('#log-output').innerText(), /initial log/);
    await page.evaluate(() => window.getSelection().removeAllRanges());
    await page.getByRole('button', {name:'Refresh logs'}).click();
    await page.getByText(/updated log/).waitFor();
  } finally { await browser.close(); }
});


test('mutation controls recover even when the status refresh fails', async () => {
  const b = browser();
  await flush();
  vm.runInContext('request = async (command) => { if (command) throw Error("mutation failed"); throw Error("status failed"); }', b.context);
  await b.nodes['#apps'].querySelectorAll('button')[0].listeners.click();
  assert.equal(b.nodes['#apps'].querySelectorAll('button')[0].disabled, false);
  assert.match(b.nodes['#message'].textContent, /status failed/);
});

test('focus falls back to logs when an action disappears', async () => {
  const b = browser();
  await flush();
  b.nodes['#apps'].querySelectorAll('button')[0].focus();
  vm.runInContext('render({apps: [{name:"hello", desired:"stopped", state:"stopped"}]})', b.context);
  assert.equal(b.document.activeElement.getAttribute('aria-label'), 'logs hello');
});

test('reconnection clears offline messages and lifecycle completion clears waiting', async () => {
  const b = browser(); await flush();
  vm.runInContext('request = async () => { throw Error("status offline"); }; update()', b.context);
  await flush();
  assert.match(b.nodes['#message'].textContent, /offline/);
  vm.runInContext('request = async () => ({apps:[{name:"hello",state:"running",desired:"running"}]}); update()', b.context);
  await flush();
  assert.equal(b.nodes['#message'].textContent, '');
  vm.runInContext('update({app:"hello",action:"start"})', b.context);
  await flush();
  assert.match(b.nodes['#message'].textContent, /completed/);
  assert.doesNotMatch(b.nodes['#message'].textContent, /Waiting/);
});

test('task app controls explain independent scheduling', async () => {
  const b = browser(); await flush();
  vm.runInContext('render({apps:[{name:"cron",state:"running",desired:"running",scheduling_independent:true}]})', b.context);
  assert.ok(b.nodes['#apps'].querySelectorAll('button').some(node => node.textContent === 'Stop editor'));
  assert.ok(b.nodes['#apps'].querySelectorAll('p').some(node => /Scheduling continues/.test(node.textContent)));
});

test('guided setup requires approval and submits only the reviewed grants', async () => {
  const b = browser(); await flush();
  vm.runInContext(`request = async () => ({apps:[{name:'tasks',title:'Tasks',version:'1',description:'',capabilities:['tasks'],installable:false,setup_available:true}]}); prompt = () => 'example.com:443'; confirm = () => false; changePlugin = async command => {globalThis.setupCommand = command;}`, b.context);
  await b.nodes['#catalog-refresh'].listeners.click();
  const install = b.nodes['#plugins'].querySelectorAll('button')[0];
  assert.equal(install.disabled, false);
  assert.equal(install.textContent, 'Set up and install');
  await install.listeners.click();
  assert.equal(b.context.setupCommand, undefined);
  vm.runInContext('confirm = () => true', b.context);
  await install.listeners.click();
  assert.deepEqual(JSON.parse(JSON.stringify(b.context.setupCommand)), {action:'setup',name:'tasks',version:'1',setup:{approved:true,task_authoring:true,task_network:['example.com:443']}});
});
