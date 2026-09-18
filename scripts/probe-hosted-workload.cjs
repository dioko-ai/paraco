#!/usr/bin/env node
/* M1 hosted workload probe. Usage:
 * PARACO_BIN=target/release/paraco node scripts/probe-hosted-workload.cjs one.json ten.json fifty.json
 * Optional PARACO_PROBE_URLS='http://...,...' supplies app endpoints; optional
 * PARACO_NOISY_URL supplies an endpoint that produces application output.
 */
const { spawn } = require('node:child_process');
const fs = require('node:fs');
const { performance } = require('node:perf_hooks');
const binary = process.env.PARACO_BIN;
const configs = process.argv.slice(2);
if (!binary || configs.length !== 3) throw new Error('usage: PARACO_BIN=... probe-hosted-workload.cjs one.json ten.json fifty.json');
const parsed = configs.map(p => ({ path:p, json:JSON.parse(fs.readFileSync(p, 'utf8')) }));
if (parsed.map(x => x.json.apps?.length).join(',') !== '1,10,50') throw new Error('configs must contain 1, 10, and 50 apps, in that order');
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
function rss(pid) { try { return +/VmRSS:\s+(\d+)/.exec(fs.readFileSync(`/proc/${pid}/status`, 'utf8'))?.[1] || null; } catch { return null; } }
async function waitExit(child) { if (child.exitCode !== null) return; await Promise.race([new Promise(r => child.once('exit', r)), sleep(3000)]); if (child.exitCode === null) child.kill('SIGKILL'); }
async function run({ path, json }, index) {
 const port = 3900 + index, out = [];
 const launched = performance.now();
 const child = spawn(binary, ['serve', '--config', path, '--port', String(port)], {stdio:['ignore','pipe','pipe']});
 child.stdout.on('data', x => out.push(String(x))); child.stderr.on('data', x => out.push(String(x)));
 try {
  for (let end = Date.now()+10000; !out.join('').includes('dashboard listening') && Date.now()<end; ) await sleep(50);
  if (!out.join('').includes('dashboard listening')) throw new Error(`server did not start: ${out.join('')}`);
  const startupMs = Math.round(performance.now() - launched);
  const rssAtReadyKiB = rss(child.pid);
  const urls = (process.env.PARACO_PROBE_URLS || `http://127.0.0.1:${port}/`).split(',');
  const started = performance.now();
  const responses = await Promise.all(Array.from({length:64}, (_, i) => fetch(urls[i % urls.length])));
  const noisy = process.env.PARACO_NOISY_URL ? await Promise.all(Array.from({length:8}, () => fetch(process.env.PARACO_NOISY_URL))) : [];
  const statusCounts = {}; for (const r of [...responses, ...noisy]) statusCounts[r.status] = (statusCounts[r.status]||0)+1;
  return {apps:json.apps.length, startupMs, requests:responses.length, noisyRequests:noisy.length, elapsedMs:Math.round(performance.now()-started), rssAtReadyKiB, rssAfterLoadKiB:rss(child.pid), statusCounts};
 } finally { child.kill('SIGINT'); await waitExit(child); }
}
(async () => {
 const results = [];
 // Run serially: if a measurement fails, its finally reaps its child before
 // the next starts, so failure cannot leave sibling probe processes behind.
 for (let index = 0; index < parsed.length; index++) results.push(await run(parsed[index], index));
 console.log(JSON.stringify({platform:process.platform, results}, null, 2));
})().catch(e => { console.error(e.stack); process.exitCode=1; });
