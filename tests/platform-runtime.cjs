// Exercises the artifact notebook through the supervised local CLI runtime.
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const os = require('node:os');
const path = require('node:path');
const net = require('node:net');
const { spawn, execFileSync } = require('node:child_process');
const { test } = require('node:test');

const root = path.resolve(__dirname, '..');
const binary = process.env.PARACO_BIN || path.join(root, 'target/debug/paraco');
const app = path.join(root, 'examples/artifacts');
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));

async function freePort() {
  const server = net.createServer();
  await new Promise((resolve, reject) => server.listen(0, '127.0.0.1', error => error ? reject(error) : resolve()));
  const port = server.address().port;
  await new Promise(resolve => server.close(resolve));
  return port;
}

test('artifact notebook publishes, resolves, traces lineage, and retains artifacts across restart', { timeout: 90000 }, async () => {
  const temp = await fs.mkdtemp(path.join(os.tmpdir(), 'paraco-platform-runtime-'));
  const logs = path.join(temp, 'logs');
  const port = await freePort();
  const origin = `http://127.0.0.1:${port}`;
  let child;
  let output = '';

  async function call(route, body) {
    const response = await fetch(`${origin}${route}`, {
      method: body === undefined ? 'GET' : 'POST',
      headers: body === undefined ? {} : { Origin: origin, 'Content-Type': 'application/json' },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    const text = await response.text();
    let value;
    try { value = JSON.parse(text); } catch { value = text; }
    return { response, value };
  }

  async function start() {
    child = spawn(binary, ['--data-dir', temp, '--log-dir', logs, 'run', app, '--port', String(port)], {
      cwd: temp,
      env: {
        ...process.env,
        TMPDIR: temp,
        PARACO_ESBUILD: path.join(root, 'node_modules/.bin/esbuild'),
        PARACO_WORKERD: path.join(root, 'node_modules/.bin/workerd'),
        PARACO_WORKERD_VERSION: execFileSync(path.join(root, 'node_modules/.bin/workerd'), ['--version'], { encoding: 'utf8' }).trim().replace(/^workerd\s+/, ''),
      },
      stdio: ['ignore', 'pipe', 'pipe'],
    });
    child.stdout.on('data', chunk => output += chunk);
    child.stderr.on('data', chunk => output += chunk);
    const deadline = Date.now() + 25000;
    let last;
    while (Date.now() < deadline) {
      if (child.exitCode !== null) throw new Error(`Paraco exited during startup: ${output}`);
      try {
        const { response } = await call('/');
        if (response.ok) return;
      } catch (error) { last = error; }
      await sleep(50);
    }
    throw new Error(`Timed out waiting for Paraco app: ${last?.message || ''}\n${output}`);
  }

  async function stop() {
    const current = child;
    child = undefined;
    if (!current) return;
    if (current.exitCode !== null || current.signalCode !== null) return;
    const exited = new Promise(resolve => current.once('exit', resolve));
    current.kill('SIGINT');
    await Promise.race([exited, sleep(5000)]);
    if (current.exitCode === null && current.signalCode === null) {
      current.kill('SIGKILL');
      await exited;
    }
  }

  async function post(route, body) {
    const { response, value } = await call(route, body);
    assert.equal(response.ok, true, `${route} returned ${response.status}: ${JSON.stringify(value)}\n${output}`);
    return value;
  }

  try {
    await start();
    const report = await post('/api/reports?dataset=private', { content: 'First retained report.' });
    assert.equal(report.artifact.output, 'report');
    assert.equal(report.artifact.mediaType, 'text/plain');
    assert.equal(report.artifact.content, 'First retained report.');
    const installationId = report.artifact.installationId;

    const evaluated = await post('/api/evaluate?dataset=private', {});
    assert.equal(evaluated.report.id, report.artifact.id);
    assert.equal(evaluated.artifact.output, 'evaluation');
    assert.equal(evaluated.artifact.mediaType, 'application/json');
    assert.deepEqual(evaluated.artifact.inputs, [report.artifact.id]);
    assert.equal(evaluated.artifact.content.executionId, evaluated.artifact.executionId);

    const lineage = await post('/api/lineage?dataset=private', { id: evaluated.artifact.id });
    assert.equal(lineage.artifact.id, evaluated.artifact.id);
    assert.deepEqual(lineage.inputs.map(item => item.id), [report.artifact.id]);

    const rejected = await fetch(`${origin}/api/reports?dataset=private`, {
      method: 'POST', headers: { Origin: 'http://attacker.invalid', 'Content-Type': 'application/json' },
      body: JSON.stringify({ content: 'blocked' }),
    });
    assert.equal(rejected.status, 403, 'cross-origin publish must be rejected');

    await stop();
    await start();
    const { response, value } = await call('/api/artifacts?dataset=private');
    assert.equal(response.ok, true, JSON.stringify(value));
    assert.deepEqual(value.artifacts.map(item => item.id), [report.artifact.id, evaluated.artifact.id]);
    assert.ok(value.artifacts.every(item => item.installationId === installationId), 'installation identity survives restart');
  } finally {
    await stop();
    await fs.rm(temp, { recursive: true, force: true });
  }
});
