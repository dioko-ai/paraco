const test = require('node:test');
const assert = require('node:assert/strict');
const esbuild = require('esbuild');
const Module = require('node:module');
const path = require('node:path');
const source = path.join(__dirname, '../examples/sandbox-probe/main.ts');
const built = esbuild.buildSync({entryPoints: [source], bundle: true, write: false, platform: 'node', format: 'cjs'}).outputFiles[0].text;
const loaded = new Module(source, module); loaded.filename = source; loaded.paths = module.paths; loaded._compile(built, source);
const {probeEnvironment} = loaded.exports;
const safe = {Function: () => () => false, fetch: async () => {throw Error('network error');}};
const byId = (report, id) => report.checks.find(item => item.id === id);

test('missing APIs and a failed network request never establish host isolation', async () => {
  const report = await probeEnvironment(safe, {}, 'http://127.0.0.1:8080/');
  assert.equal(byId(report, 'node').status, 'PASS');
  assert.equal(byId(report, 'network').status, 'UNVERIFIED');
  assert.equal(report.hostEvidence.status, 'UNVERIFIED');
  assert.equal(report.status, 'UNVERIFIED');
});
test('exposed capabilities and a successful direct request fail the report', async () => {
  const report = await probeEnvironment({...safe, process: {}, fetch: async () => ({status: 404})}, {tasks: {}}, 'http://127.0.0.1:8080/');
  assert.equal(byId(report, 'node').status, 'FAIL');
  assert.equal(byId(report, 'capabilities').status, 'FAIL');
  assert.equal(byId(report, 'network').status, 'FAIL');
  assert.equal(report.status, 'FAIL');
});
test('invalid destinations are never fetched and dynamic code rejection is unverified', async () => {
  let called = false;
  const report = await probeEnvironment({...safe, Function: () => {throw Error('disabled')}, fetch: async () => {called = true}}, {}, 'http://example.com/');
  assert.equal(called, false);
  assert.equal(byId(report, 'network').status, 'UNVERIFIED');
  assert.equal(byId(report, 'dynamic-code').status, 'UNVERIFIED');
});

const evidence = () => ({schemaVersion: 1, backend: 'test', limitations: [], checks: [{id: 'containment-mode', status: 'PASS', detail: 'verified'}, ...['native', 'compiler'].flatMap(prefix => ['control', 'read', 'write', 'readonly', 'network', 'exec', 'environment'].map(id => ({id: `${prefix}-${id}`, status: 'PASS', detail: 'verified'})))]});
test('complete host evidence and passing guest checks produce PASS', async () => {
  const report = await probeEnvironment(safe, {containmentEvidence: evidence()});
  assert.equal(report.hostEvidence.status, 'PASS');
  assert.equal(report.status, 'PASS');
});
test('missing, duplicate, and invalid evidence cannot produce PASS', async () => {
  for (const mutate of [e => e.checks.pop(), e => e.checks = e.checks.filter(c => c.id !== 'native-control'), e => e.checks = e.checks.filter(c => c.id !== 'containment-mode'), e => e.checks.push(e.checks[0]), e => e.checks[0].status = 'MAYBE']) {
    const e = evidence(); mutate(e);
    const report = await probeEnvironment(safe, {containmentEvidence: e});
    assert.equal(report.status, 'UNVERIFIED');
  }
});
test('exposed global overrides passing evidence', async () => {
  const report = await probeEnvironment({...safe, Deno: {}}, {containmentEvidence: evidence()});
  assert.equal(report.status, 'FAIL');
});
test('dynamic evaluation denial is distinct from unknown errors', async () => {
  const report = await probeEnvironment({...safe, Function: () => {throw new EvalError('denied')}}, {containmentEvidence: evidence()});
  assert.equal(byId(report, 'dynamic-code').status, 'PASS');
  assert.equal(report.status, 'PASS');
});
test('report page has distinct status styles', async () => {
  const response = await loaded.exports.default.fetch(new Request('http://localhost/'), {});
  const html = await response.text();
  assert.match(html, /\.PASS\{color:/);
  assert.match(html, /\.FAIL\{color:/);
  assert.match(html, /\.UNVERIFIED\{color:/);
});
