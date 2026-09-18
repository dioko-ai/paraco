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
  for (const id of ['logs', 'log-title', 'log-output', 'log-launch', 'log-refresh']) { nodes['#' + id] = new Element('div'); nodes['#' + id].value = ''; }
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
        gateway: 'http://127.0.0.1:3000',
        apps: [{ name: 'hello', desired: 'running', state: error ? 'failed' : 'running', error }],
      }) };
    },
  });
  vm.runInContext(source, context);
  return { nodes, requests, timers, history, storage, document, context };
}
const flush = () => new Promise(resolve => setImmediate(resolve));

test('fragment authorization, lifecycle buttons, polling and keyboard focus', async () => {
  const b = browser();
  await flush();
  assert.equal(b.history[0][2], '/');
  assert.equal(b.storage.get('paraco-token'), 'secret');
  assert.equal(b.requests[0].headers.Authorization, 'Bearer secret');
  assert.equal(b.requests[0].credentials, 'omit');
  const buttons = b.nodes['#apps'].querySelectorAll('button');
  assert.equal(buttons.length, 4);
  const link = b.nodes['#apps'].querySelectorAll('a')[0];
  assert.equal(link.href, 'http://127.0.0.1:3000/apps/hello/');
  assert.equal(link.rel, 'noopener noreferrer');
  buttons[1].focus();
  await buttons[1].listeners.click();
  const post = b.requests.find(request => request.method === 'POST');
  assert.deepEqual(JSON.parse(post.body), { action: 'stop', app: 'hello' });
  assert.equal(post.headers['Content-Type'], 'application/json');
  assert.equal(b.document.activeElement.getAttribute('aria-label'), 'stop hello');
  assert.match(b.nodes['#message'].textContent, /stop requested/);
  assert.equal(b.requests.at(-1).method, 'GET');
});

test('missing or revoked credentials stop polling and explain authorization', async () => {
  const missing = browser({ hash: '' });
  const revoked = browser({ hash: '', stored: 'expired', status: 401 });
  await flush();
  assert.equal(missing.requests.length, 0);
  for (const b of [missing, revoked]) {
    assert.match(b.nodes['#message'].textContent, /management link/);
    assert.equal(b.timers.length, 0);
  }
  assert.equal(revoked.storage.has('paraco-token'), false);
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
  await b.nodes['#apps'].querySelectorAll('button')[3].listeners.click();
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
