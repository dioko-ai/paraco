// Exercises the opt-in local system capability through the supervised CLI runtime.
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const os = require('node:os');
const path = require('node:path');
const net = require('node:net');
const { spawn, execFileSync } = require('node:child_process');
const { test } = require('node:test');

const root = path.resolve(__dirname, '..');
const binary = process.env.PARACO_BIN || path.join(root, 'target/debug/paraco');
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
const linux = process.platform === 'linux';

async function freePort() {
  const server = net.createServer();
  await new Promise((resolve, reject) => server.listen(0, '127.0.0.1', error => error ? reject(error) : resolve()));
  const port = server.address().port;
  await new Promise(resolve => server.close(resolve));
  return port;
}

const entrypoint = `export default {
  async fetch(_request: Request, ctx: any): Promise<Response> {
    const value = { system: !!ctx.system, snapshot: ctx.system ? await ctx.system.snapshot() : null };
    return new Response(JSON.stringify(value), { headers: { "Content-Type": "application/json" } });
  }
};
`;

async function writeApp(appDir, name, capabilities) {
  await fs.mkdir(appDir, { recursive: true });
  await fs.writeFile(path.join(appDir, 'paraco.json'), JSON.stringify({
    schemaVersion: 1, name, entrypoint: 'main.ts', capabilities,
  }, null, 2));
  await fs.writeFile(path.join(appDir, 'main.ts'), entrypoint);
}

test('system capability is opt-in, reports host metrics, and is protected by the installation baseline', {
  timeout: 120000,
  skip: linux ? false : `System telemetry is currently Linux-only (this host is ${process.platform}).`,
}, async () => {
  const temp = await fs.mkdtemp(path.join(os.tmpdir(), 'paraco-system-runtime-'));
  const logs = path.join(temp, 'logs');
  const appDir = path.join(temp, 'system-app');
  const noSystemDir = path.join(temp, 'baseline-app');
  const probe = net.createServer();
  let probePort;
  let child;
  let output = '';
  const environment = {
    ...process.env,
    TMPDIR: temp,
    PARACO_ESBUILD: path.join(root, 'node_modules/.bin/esbuild'),
    PARACO_WORKERD: path.join(root, 'node_modules/.bin/workerd'),
    PARACO_WORKERD_VERSION: execFileSync(path.join(root, 'node_modules/.bin/workerd'), ['--version'], { encoding: 'utf8' }).trim().replace(/^workerd\s+/, ''),
  };

  async function start(app, port) {
    child = spawn(binary, ['--log-dir', logs, 'run', app, '--port', String(port)], {
      cwd: temp, env: environment, stdio: ['ignore', 'pipe', 'pipe'],
    });
    const current = child;
    current.stdout.on('data', chunk => { output += chunk; });
    current.stderr.on('data', chunk => { output += chunk; });
    const origin = `http://127.0.0.1:${port}`;
    const deadline = Date.now() + 25000;
    let last;
    while (Date.now() < deadline) {
      if (current.exitCode !== null) throw new Error(`Paraco exited during startup: ${output}`);
      try {
        const response = await fetch(origin);
        if (response.ok) return { origin, value: await response.json() };
        last = new Error(`HTTP ${response.status}`);
      } catch (error) { last = error; }
      await sleep(50);
    }
    throw new Error(`Timed out waiting for Paraco app: ${last?.message || ''}\n${output}`);
  }

  async function stop() {
    const current = child;
    child = undefined;
    if (!current || current.exitCode !== null || current.signalCode !== null) return;
    const exited = new Promise(resolve => current.once('exit', resolve));
    current.kill('SIGINT');
    await Promise.race([exited, sleep(5000)]);
    if (current.exitCode === null && current.signalCode === null) {
      current.kill('SIGKILL');
      await exited;
    }
  }

  async function assertDenied(app, port) {
    const result = await new Promise((resolve, reject) => {
      const denied = spawn(binary, ['--log-dir', logs, 'run', app, '--port', String(port)], {
        cwd: temp, env: environment, stdio: ['ignore', 'pipe', 'pipe'],
      });
      let text = '';
      denied.stdout.on('data', chunk => { text += chunk; });
      denied.stderr.on('data', chunk => { text += chunk; });
      const timeout = setTimeout(() => { denied.kill('SIGKILL'); reject(new Error('Expected capability rejection; app kept running: '+text)); }, 10000);
      denied.once('error', error => {clearTimeout(timeout);reject(error);});
      denied.once('close', (code, signal) => {clearTimeout(timeout);resolve({ code, signal, text });});
    });
    assert.equal(result.code, 1, `updated installation should be denied (exit ${result.code}, signal ${result.signal}): ${result.text}`);
    assert.match(result.text, /capabilit/i, `expected a capability approval error: ${result.text}`);
  }

  try {
    await fs.mkdir(logs, { recursive: true, mode: 0o700 });
    await new Promise((resolve, reject) => probe.listen(0, '127.0.0.1', error => error ? reject(error) : resolve()));
    probePort = probe.address().port;

    await writeApp(appDir, 'system-probe', ['system']);
    const firstPort = await freePort();
    const first = await start(appDir, firstPort);
    assert.equal(first.value.system, true);
    const snapshot = first.value.snapshot;
    assert.ok(snapshot && typeof snapshot === 'object', 'system snapshot is present');
    assert.ok(Number.isFinite(snapshot.sampledAt) && snapshot.sampledAt > 0, 'sample time is epoch milliseconds');
    assert.equal(snapshot.platform, 'linux');
    assert.equal(typeof snapshot.hostname, 'string');
    assert.ok(snapshot.hostname.length > 0);
    assert.ok(Number.isInteger(snapshot.cores) && snapshot.cores >= 1);
    assert.ok(Number.isFinite(snapshot.cpu?.total) && snapshot.cpu.total > 0);
    assert.ok(Number.isFinite(snapshot.cpu?.idle) && snapshot.cpu.idle >= 0 && snapshot.cpu.idle <= snapshot.cpu.total);
    for (const key of ['total', 'available', 'swapTotal', 'swapFree']) {
      assert.ok(Number.isFinite(snapshot.memory?.[key]) && snapshot.memory[key] >= 0, `memory.${key} is a non-negative byte count`);
    }
    assert.ok(snapshot.memory.total > 0);
    assert.ok(Array.isArray(snapshot.load) && snapshot.load.length === 3 && snapshot.load.every(value => Number.isFinite(value) && value >= 0));
    assert.ok(Number.isFinite(snapshot.uptime) && snapshot.uptime > 0);
    assert.ok(Array.isArray(snapshot.ports));
    assert.ok(Array.isArray(snapshot.warnings));
    assert.ok(snapshot.ports.some(port => port.port === probePort && port.protocol === 'tcp' && port.state === 'listening'),
      `snapshot includes bound test HTTP port ${probePort}: ${JSON.stringify(snapshot.ports)}`);

    await stop();
    await writeApp(appDir, 'system-probe', []);
    const second = await start(appDir, await freePort());
    assert.equal(second.value.system, false, 'removing the manifest request hides the API on restart');
    assert.equal(second.value.snapshot, null);

    await stop();
    await writeApp(noSystemDir, 'system-baseline', []);
    const baseline = await start(noSystemDir, await freePort());
    assert.equal(baseline.value.system, false, 'new installation without system capability has no API');
    await stop();
    await writeApp(noSystemDir, 'system-baseline', ['system']);
    await assertDenied(noSystemDir, await freePort());
  } finally {
    await stop();
    if (probe.listening) await new Promise(resolve => probe.close(resolve));
    await fs.rm(temp, { recursive: true, force: true });
  }
});
