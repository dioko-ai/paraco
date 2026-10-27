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
    constructor(tag) { this.tag = tag; this.children = []; this.attributes = {}; this.listeners = {}; }
    append(...nodes) { this.children.push(...nodes); }
    replaceChildren(...nodes) { this.children = nodes; }
    setAttribute(name, value) { this.attributes[name] = value; }
    getAttribute(name) { return this.attributes[name]; }
    addEventListener(name, fn) { this.listeners[name] = fn; }
    querySelectorAll(tag) { return this.children.flatMap(child => [...(child.tag === tag ? [child] : []), ...child.querySelectorAll(tag)]); }
    focus() { document.activeElement = this; }
  }
  const nodes = { '#summary': new Element('p'), '#message': new Element('p'), '#apps': new Element('ul') };
  for (const id of ['logs', 'log-title', 'log-output', 'log-launch', 'log-refresh', 'catalog-refresh', 'plugin-message', 'plugins']) { nodes['#' + id] = new Element('div'); nodes['#' + id].value = ''; }
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
  assert.match(b.nodes['#log-output'].textContent, /Unable to load logs: offline/);
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
    failed: ['Start', 'View logs'],
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
