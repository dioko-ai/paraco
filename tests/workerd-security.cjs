// End-to-end checks for workerd task network grants and app isolation.
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const os = require('node:os');
const path = require('node:path');
const net = require('node:net');
const http = require('node:http');
const {spawn, execFileSync} = require('node:child_process');

const root = path.resolve(__dirname, '..');
const paraco = path.join(root, 'target/debug/paraco');
const workerd = process.env.WORKERD_BIN || path.join(root, 'node_modules/.bin/workerd');
const esbuildDir = path.join(root, 'node_modules/.bin');
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));

async function until(fn, label, ms = 20000) {
  const deadline = Date.now() + ms;
  let last;
  while (Date.now() < deadline) {
    try { const value = await fn(); if (value) return value; } catch (error) { last = error; }
    await sleep(50);
  }
  throw new Error(`${label}: ${last?.message || 'timed out'}`);
}

async function listen(handler) {
  const server = http.createServer(handler);
  await new Promise((resolve, reject) => server.listen(0, '127.0.0.1', resolve).once('error', reject));
  return {server, port: server.address().port, close: () => new Promise(resolve => server.close(resolve))};
}

async function main() {
  const tmp = await fs.mkdtemp(path.join(os.tmpdir(), 'paraco-workerd-security-'));
  const fixtureHits = {allowed: 0, redirect: 0, outside: 0, appProbe: 0};
  const outside = await listen((req, res) => {
    fixtureHits.outside++;
    res.writeHead(200, {'content-type': 'text/plain'});
    res.end('outside-secret');
  });
  const fixture = await listen((req, res) => {
    if (req.url === '/redirect') {
      fixtureHits.redirect++;
      res.writeHead(302, {location: `http://127.0.0.1:${outside.port}/outside`});
      res.end();
      return;
    }
    fixtureHits.allowed++;
    res.writeHead(200, {'content-type': 'text/plain'});
    res.end('approved-fixture-body');
  });
  const probe = await listen((req, res) => {
    fixtureHits.appProbe++;
    res.writeHead(200, {'content-type': 'text/plain'});
    res.end('unauthorized-app-response');
  });
  const cronPort = await new Promise((resolve, reject) => {
    const server = net.createServer();
    server.once('error', reject);
    server.listen(0, '127.0.0.1', () => { const port = server.address().port; server.close(() => resolve(port)); });
  });
  const env = {
    ...process.env,
    PATH: `${esbuildDir}${path.delimiter}${process.env.PATH || ''}`,
    TMPDIR: tmp,
    PARACO_HOST_CANARY: 'host-secret-workerd-canary',
    PARACO_WORKERD: workerd,
    PARACO_WORKERD_VERSION: execFileSync(workerd, ['--version'], {encoding: 'utf8'}).trim().replace(/^workerd\s+/, ''),
  };
  let cron;
  let app;
  let output = '';
  let runtimeGlobals;
  const origin = `http://cron.localhost:${cronPort}`;
  const call = (route, body) => new Promise((resolve, reject) => {
    const req = http.request({host: '127.0.0.1', port: cronPort, path: route,
      method: body === undefined ? 'GET' : 'POST',
      headers: {Host: `cron.localhost:${cronPort}`, Origin: origin,
        ...(body === undefined ? {} : {'Content-Type': 'application/json'})}}, res => {
      let text = '';
      res.setEncoding('utf8'); res.on('data', chunk => text += chunk);
      res.on('end', () => resolve({status: res.statusCode, ok: res.statusCode >= 200 && res.statusCode < 300, text}));
    });
    req.setTimeout(70000, () => req.destroy(new Error('request timed out')));
    req.on('error', reject);
    req.end(body === undefined ? undefined : JSON.stringify(body));
  });
  const api = async (action, body = {}, ok = true) => {
    const response = await call(`/api/${action}`, body);
    let value;
    try { value = JSON.parse(response.text); }
    catch { throw new Error(`${action} returned ${response.status}: ${response.text}\n${output}`); }
    if (ok) assert.equal(response.ok, true, `${action}: ${JSON.stringify(value)}\n${output}`);
    else assert.equal(response.ok, false, `${action} unexpectedly succeeded: ${JSON.stringify(value)}`);
    return value;
  };
  const draft = (source, network, ok = true) => api('draft', {name: 'workerd network check', source,
    schedule: {kind: 'manual'}, timeout_ms: 10000, network, notify_success: false, notify_failure: false}, ok);
  const publish = d => api('publish', {revision_id: d.revision_id, expected_current_revision: null});
  const runAndWait = async d => {
    const started = await api('run', {job_id: d.job_id});
    return until(async () => {
      const history = await api('history', {job_id: d.job_id});
      const result = history.find(item => item.id === started.run_id);
      return result && !['pending', 'running'].includes(result.status) ? result : null;
    }, `run ${d.job_id}`);
  };
  const runLogs = async run => (await api('logs', {run_id: run.id})).logs;
  const stop = async child => {
    if (!child || child.exitCode !== null || child.signalCode !== null) return;
    child.kill('SIGINT');
    await until(() => child.exitCode !== null || child.signalCode !== null, 'process shutdown', 12000);
  };
  const sourceApp = path.join(tmp, 'probe-app');

  try {
    assert.ok(await fs.stat(paraco).catch(() => null), 'build Paraco before running this integration test');
    cron = spawn(paraco, ['--data-dir', tmp, '--log-dir', path.join(tmp, 'logs'), 'cron', '--port', String(cronPort),
      '--allow-task-network', `127.0.0.1:${fixture.port}`],
    {cwd: tmp, env, stdio: ['ignore', 'pipe', 'pipe']});
    cron.stdout.on('data', chunk => output += chunk);
    cron.stderr.on('data', chunk => output += chunk);
    await until(async () => { const response = await call('/'); return response.ok && response.text.includes('editor'); }, 'Cron readiness');

    const allowed = await draft(`export default async ({log}) => {
      const response = await fetch('http://127.0.0.1:${fixture.port}/allowed');
      log('approved response: ' + await response.text());
    };`, [`127.0.0.1:${fixture.port}`]);
    await publish(allowed);
    const allowedRun = await runAndWait(allowed);
    assert.equal(allowedRun.status, 'success', JSON.stringify(allowedRun));
    assert.match(await runLogs(allowedRun), /approved response: approved-fixture-body/);
    assert.equal(fixtureHits.allowed, 1);

    const denied = await draft(`export default async () => {
      await fetch('http://127.0.0.1:${outside.port}/outside');
    };`, []);
    await publish(denied);
    const deniedRun = await runAndWait(denied);
    assert.equal(deniedRun.status, 'error', JSON.stringify(deniedRun));
    assert.match(await runLogs(deniedRun), /network destination not granted/);
    assert.equal(fixtureHits.outside, 0, 'ungranted loopback endpoint was contacted');

    await draft('export default async () => {};', [`127.0.0.1:${outside.port}`], false);

    const redirect = await draft(`export default async () => {
      await fetch('http://127.0.0.1:${fixture.port}/redirect', {redirect: 'follow'});
    };`, [`127.0.0.1:${fixture.port}`]);
    await publish(redirect);
    const redirectRun = await runAndWait(redirect);
    assert.equal(redirectRun.status, 'error', JSON.stringify(redirectRun));
    assert.match(await runLogs(redirectRun), /network destination not granted/);
    assert.equal(fixtureHits.redirect, 1);
    assert.equal(fixtureHits.outside, 0, 'redirect escaped the declared host grant');

    await fs.mkdir(sourceApp);
    await fs.writeFile(path.join(sourceApp, 'paraco.json'), JSON.stringify({name: 'runtime-probe', entrypoint: 'main.ts', capabilities: []}));
    await fs.writeFile(path.join(sourceApp, 'main.ts'), `export default {async fetch(_request, ctx) {
      const globals = {deno: typeof Deno, process: typeof process,
        processEnv: typeof process === 'undefined' ? 'undefined' : typeof process.env,
        hostCanary: typeof process === 'undefined' ? null : (process.env.PARACO_HOST_CANARY ?? null),
        processPid: typeof process === 'undefined' ? 'undefined' : typeof process.pid,
        processCwd: typeof process === 'undefined' ? 'undefined' : typeof process.cwd,
        processExit: typeof process === 'undefined' ? 'undefined' : typeof process.exit,
        processBinding: typeof process === 'undefined' ? 'undefined' : typeof process.binding,
        ai: typeof ctx.ai};
      let workersEnv = {status: 'blocked'};
      try {
        const specifier = 'cloudflare:workers';
        workersEnv = await Promise.race([
          import(specifier).then(module => ({status: 'loaded', env: typeof module.env,
            settings: typeof module.env?.SETTINGS, host: typeof module.env?.HOST,
            keys: Object.keys(module.env || {})}), () => ({status: 'blocked'})),
          new Promise(resolve => setTimeout(() => resolve({status: 'blocked'}), 250)),
        ]);
      } catch {}
      let result;
      try {
        result = await Promise.race([
          fetch('http://127.0.0.1:${probe.port}/probe').then(response => response.text(), error => 'blocked:' + error.name),
          new Promise(resolve => setTimeout(() => resolve('blocked:timeout'), 3000)),
        ]);
      }
      catch (error) { result = 'blocked:' + error.name; }
      return Response.json({globals, workersEnv, result});
    }};`);
    const appPort = await new Promise((resolve, reject) => {
      const server = net.createServer(); server.once('error', reject);
      server.listen(0, '127.0.0.1', () => { const port = server.address().port; server.close(() => resolve(port)); });
    });
    app = spawn(paraco, ['--data-dir', tmp, '--log-dir', path.join(tmp, 'app-logs'), 'run', sourceApp, '--port', String(appPort)],
      {cwd: tmp, env, stdio: ['ignore', 'pipe', 'pipe']});
    let appOutput = '';
    app.stdout.on('data', chunk => appOutput += chunk);
    app.stderr.on('data', chunk => appOutput += chunk);
    const appRequest = async () => new Promise((resolve, reject) => {
      const request = http.get({host: '127.0.0.1', port: appPort, path: '/', headers: {Host: 'localhost'}}, response => {
        let text = ''; response.setEncoding('utf8'); response.on('data', chunk => text += chunk);
        response.on('end', () => resolve({status: response.statusCode, text}));
      });
      request.setTimeout(5000, () => request.destroy(new Error('app request timed out')));
      request.on('error', reject);
    });
    await until(() => {
      if (app.exitCode !== null) throw new Error(`app exited: ${appOutput}`);
      return appOutput.includes('listening on http://');
    }, `runtime app listener\n${appOutput}`);
    const response = await appRequest();
    assert.equal(response.status, 200, `${response.text}\n${appOutput}`);
    const value = JSON.parse(response.text);
    runtimeGlobals = value.globals;
    assert.equal(runtimeGlobals.deno, 'undefined');
    assert.equal(runtimeGlobals.ai, 'undefined');
    assert.equal(runtimeGlobals.hostCanary, null, 'workerd inherited a host environment canary');
    assert.equal(runtimeGlobals.process, 'undefined');
    assert.equal(runtimeGlobals.processEnv, 'undefined');
    assert.equal(runtimeGlobals.processPid, 'undefined');
    assert.equal(runtimeGlobals.processCwd, 'undefined');
    assert.equal(runtimeGlobals.processExit, 'undefined');
    assert.equal(runtimeGlobals.processBinding, 'undefined');
    assert.equal(value.workersEnv.settings, 'undefined', 'cloudflare:workers exposed SETTINGS');
    assert.equal(value.workersEnv.host, 'undefined', 'cloudflare:workers exposed the host service');
    if (value.workersEnv.status === 'loaded') assert.deepEqual(value.workersEnv.keys, []);
    assert.match(value.result, /^blocked:/);
    assert.equal(fixtureHits.appProbe, 0, 'ungranted global fetch contacted the fixture');
    await stop(app); app = null;

    const parent = path.dirname(sourceApp);
    await fs.writeFile(path.join(parent, 'private.ts'), "export const secret = 'PRIVATE_IMPORT_SENTINEL';");
    assert.ok((await fs.stat(path.join(parent, 'private.ts'))).isFile());
    await fs.writeFile(path.join(sourceApp, 'main.ts'), "import {secret} from '../private.ts'; export default {fetch(){return new Response(secret)}};");
    const importPort = await new Promise((resolve, reject) => {
      const server = net.createServer(); server.once('error', reject);
      server.listen(0, '127.0.0.1', () => { const port = server.address().port; server.close(() => resolve(port)); });
    });
    const importChild = spawn(paraco, ['--data-dir', tmp, '--log-dir', path.join(tmp, 'import-logs'), 'run', sourceApp, '--port', String(importPort)],
      {cwd: tmp, env, stdio: ['ignore', 'pipe', 'pipe']});
    let importOutput = '';
    importChild.stdout.on('data', chunk => importOutput += chunk);
    importChild.stderr.on('data', chunk => importOutput += chunk);
    await until(() => importChild.exitCode !== null || importChild.signalCode !== null, 'outside import rejection');
    assert.match(importOutput, /application compilation failed:.*Could not resolve/s);
    assert.doesNotMatch(importOutput, /PRIVATE_IMPORT_SENTINEL/);
    assert.equal(runtimeGlobals.process, 'undefined');
    assert.equal(runtimeGlobals.processPid, 'undefined');
    assert.equal(runtimeGlobals.processCwd, 'undefined');
    console.log(`workerd security probe: ${JSON.stringify({globals: runtimeGlobals, workersEnv: value.workersEnv, fetch: value.result})}`);
    console.log('workerd security: task grants, redirect denial, runtime globals, global fetch, and package import containment passed');
  } finally {
    await stop(app);
    await stop(cron);
    await Promise.all([fixture.close(), outside.close(), probe.close()]);
    await fs.rm(tmp, {recursive: true, force: true});
  }
}

main().catch(error => { console.error(error); process.exitCode = 1; });
