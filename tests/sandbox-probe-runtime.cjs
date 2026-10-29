// End-to-end test of the host diagnostic and the bundled probe application.
const assert = require('node:assert/strict');
const {test} = require('node:test');
const {spawn, spawnSync} = require('node:child_process');
const fs = require('node:fs/promises');
const os = require('node:os');
const path = require('node:path');
const net = require('node:net');

const root = path.resolve(__dirname, '..');
const binary = process.env.PARACO_BIN || path.join(root, 'target/debug/paraco');
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
async function port() {
  const server = net.createServer();
  await new Promise((resolve, reject) => server.once('error', reject).listen(0, '127.0.0.1', resolve));
  const value = server.address().port;
  await new Promise(resolve => server.close(resolve));
  return value;
}

test('trusted containment evidence and unsafe mode reach the probe report', {timeout: 180000}, async () => {
  const temp = await fs.mkdtemp(path.join(os.tmpdir(), 'paraco-sandbox-probe-test-'));
  const env = {...process.env, TMPDIR: temp, PATH: path.join(root, 'node_modules/.bin') + path.delimiter + (process.env.PATH || '')};
  delete env.PARACO_ALLOW_UNSANDBOXED;
  delete env.PARACO_EXPERIMENTAL_MACOS_SANDBOX;
  const unsafe = {...env, PARACO_ALLOW_UNSANDBOXED: '1'};
  const children = new Set();
  function diagnostic(environment) {
    const result = spawnSync(binary, ['sandbox-check'], {cwd: temp, env: environment, encoding: 'utf8', timeout: 30000, maxBuffer: 1024 * 1024});
    if (result.error) throw result.error;
    let report;
    try { report = JSON.parse(result.stdout); } catch { throw Error(`Invalid diagnostic output: ${result.stdout.slice(-4000)} ${result.stderr.slice(-4000)}`); }
    return {code: result.status, report};
  }
  async function stop(child) {
    if (!child || child.exitCode !== null || child.signalCode !== null) return;
    const exited = new Promise(resolve => child.once('exit', resolve));
    child.kill('SIGINT');
    await Promise.race([exited, sleep(5000)]);
    if (child.exitCode === null && child.signalCode === null) { child.kill('SIGKILL'); await exited; }
    children.delete(child);
  }
  async function app(args, environment) {
    const currentPort = await port();
    const child = spawn(binary, [...args, '--port', String(currentPort)], {cwd: temp, env: environment, stdio: ['ignore', 'pipe', 'pipe']});
    children.add(child);
    let output = '';
    for (const stream of [child.stdout, child.stderr]) stream.on('data', chunk => {output = (output + chunk).slice(-12000)});
    child.on('error', error => {output += String(error)});
    const deadline = Date.now() + 40000;
    let last;
    while (Date.now() < deadline) {
      if (child.exitCode !== null || child.signalCode !== null) throw Error(`Probe exited during startup: ${output}`);
      try {
        const response = await fetch(`http://127.0.0.1:${currentPort}/api/report`, {signal: AbortSignal.timeout(2000)});
        if (response.ok) return {child, report: await response.json()};
        last = `HTTP ${response.status}`;
      } catch (error) {last = error.message;}
      await sleep(100);
    }
    throw Error(`Probe startup timed out: ${last}\n${output}`);
  }
  try {
    const secure = diagnostic(env);
    assert.equal(secure.code, 0, JSON.stringify(secure.report));
    assert.equal(secure.report.status, 'PASS', JSON.stringify(secure.report));
    const served = await app(['sandbox-probe'], env);
    assert.equal(served.report.status, 'PASS', JSON.stringify(served.report));
    assert.ok(served.report.hostEvidence.checks.length >= 12);
    assert.ok(served.report.hostEvidence.checks.every(c => c.status === 'PASS'));
    assert.ok(served.report.checks.every(c => c.status === 'PASS'));
    await stop(served.child);
    const ordinary = await app(['--data-dir', path.join(temp, 'data'), '--log-dir', path.join(temp, 'logs'), 'run', path.join(root, 'examples/sandbox-probe')], env);
    assert.equal(ordinary.report.status, 'UNVERIFIED');
    assert.equal(ordinary.report.hostEvidence.status, 'UNVERIFIED');
    await stop(ordinary.child);
    const uncontained = diagnostic(unsafe);
    assert.notEqual(uncontained.code, 0);
    assert.equal(uncontained.report.status, 'FAIL');
    for (const id of ['native-read', 'native-write', 'native-network', 'native-exec']) {
      assert.equal(uncontained.report.checks.find(c => c.id === id)?.status, 'FAIL', id);
    }
    const unsafeApp = await app(['sandbox-probe'], unsafe);
    assert.equal(unsafeApp.report.status, 'FAIL', JSON.stringify(unsafeApp.report));
    await stop(unsafeApp.child);
  } finally {
    for (const child of children) await stop(child);
    await fs.rm(temp, {recursive: true, force: true});
  }
});
