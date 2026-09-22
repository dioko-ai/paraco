#!/usr/bin/env node
/* M1 hosted workload probe. Usage:
 * PARACO_BIN=target/release/paraco node scripts/probe-hosted-workload.cjs one.json ten.json fifty.json
 * Optional PARACO_PROBE_URLS='http://...,...' supplies app endpoints; optional
 * PARACO_NOISY_URL supplies an endpoint that produces application output.
 */
const { spawn, execFileSync } = require('node:child_process');
const fs = require('node:fs');
const { performance } = require('node:perf_hooks');
const binary = process.env.PARACO_BIN;
const configs = process.argv.slice(2);
if (!binary || configs.length !== 3) throw new Error('usage: PARACO_BIN=... probe-hosted-workload.cjs one.json ten.json fifty.json');
const parsed = configs.map(p => ({ path:p, json:JSON.parse(fs.readFileSync(p, 'utf8')) }));
if (parsed.map(x => x.json.apps?.length).join(',') !== '1,10,50') throw new Error('configs must contain 1, 10, and 50 apps, in that order');
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
function rss(pid) { if (process.platform === 'darwin') { try { return Number(execFileSync('ps', ['-o','rss=','-p',String(pid)], {encoding:'utf8'}).trim()); } catch { return null; } } try { return +/VmRSS:\s+(\d+)/.exec(fs.readFileSync(`/proc/${pid}/status`, 'utf8'))?.[1] || null; } catch { return null; } }
function treeRss(pid) {
 try {
  const rows=execFileSync('ps',['-axo','pid=,ppid=,rss='],{encoding:'utf8'}).trim().split('\n').map(line=>line.trim().split(/\s+/).map(Number));
  const owned=new Set([pid]); let changed=true;
  while(changed) { changed=false; for(const [child,parent] of rows) if(owned.has(parent)&&!owned.has(child)){owned.add(child);changed=true;} }
  return rows.filter(row=>owned.has(row[0])).reduce((sum,row)=>sum+row[2],0);
 } catch { return null; }
}
function noisyRequest(port) {
 return new Promise((resolve,reject)=>{
  const req=require('node:http').get({hostname:'127.0.0.1',port,path:'/noisy',headers:{Host:`${process.env.PARACO_NOISY_APP}.localhost:${port}`}},res=>{res.resume();res.on('end',()=>resolve({status:res.statusCode}));});
  req.setTimeout(15000,()=>req.destroy(new Error('noisy request timed out')));req.on('error',reject);
 });
}
async function waitExit(child) {
 if(child.exitCode!==null || child.signalCode!==null) return;
 let timer;
 const graceful=await Promise.race([new Promise(resolve=>child.once('exit',()=>resolve(true))),new Promise(resolve=>{timer=setTimeout(()=>resolve(false),15000);})]);
 clearTimeout(timer);
 if(!graceful) { child.kill('SIGKILL'); throw new Error('workload runtime did not shut down within 15 seconds'); }
}

async function run({ path, json }, index) {
 const port = 3900 + index; let out = "";
 const launched = performance.now();
 const child = spawn(binary, ['serve', '--config', path, '--port', String(port)], {stdio:['ignore','pipe','pipe']});
 child.stdout.on('data', x => { out = (out+String(x)).slice(-65536); }); child.stderr.on('data', x => { out = (out+String(x)).slice(-65536); });
 try {
  for (let end = Date.now()+10000; !out.includes('dashboard listening') && Date.now()<end; ) await sleep(50);
  if (!out.includes('dashboard listening')) throw new Error(`server did not start: ${out.replace(/(management dashboard )\S+/g, '$1[redacted]')}`);
  let states;
  for (let end = Date.now()+30000; Date.now()<end;) {
    states = JSON.parse(execFileSync(binary, ['status','--port',String(port)], {encoding:'utf8'}));
    if (states.every(app => app.state === 'running')) break;
    await sleep(100);
  }
  if (!states?.every(app => app.state === 'running')) throw new Error(`apps not ready: ${JSON.stringify(states)}`);
  const startupMs = Math.round(performance.now() - launched);
  const rssAtReadyKiB = rss(child.pid), treeRssAtReadyKiB=treeRss(child.pid);
  const urls = (process.env.PARACO_PROBE_URLS || `http://127.0.0.1:${port}/`).split(',');
  const started = performance.now();
  const responses = await Promise.all(Array.from({length:64}, (_, i) => fetch(urls[i % urls.length])));
  const noisy = process.env.PARACO_NOISY_APP ? await Promise.all(Array.from({length:8}, () => noisyRequest(port))) : process.env.PARACO_NOISY_URL ? await Promise.all(Array.from({length:8}, () => fetch(process.env.PARACO_NOISY_URL))) : [];
  if(noisy.some(response=>response.status!==200)) throw new Error('noisy app request failed');
  const managementStart=performance.now();
  const management=JSON.parse(execFileSync(binary,['status','--port',String(port)],{encoding:'utf8'}));
  const managementLatencyMs=Math.round(performance.now()-managementStart);
  if(!management.every(app=>app.state==='running')) throw new Error('apps failed during noisy load');
  const statusCounts = {}; for (const r of [...responses, ...noisy]) statusCounts[r.status] = (statusCounts[r.status]||0)+1;
  return {apps:json.apps.length, startupMs, managementLatencyMs, treeRssAtReadyKiB, treeRssAfterLoadKiB:treeRss(child.pid), requests:responses.length, noisyRequests:noisy.length, elapsedMs:Math.round(performance.now()-started), rssAtReadyKiB, rssAfterLoadKiB:rss(child.pid), statusCounts};
 } finally { child.kill('SIGINT'); await waitExit(child); }
}
(async () => {
 const results = [];
 // Run serially: if a measurement fails, its finally reaps its child before
 // the next starts, so failure cannot leave sibling probe processes behind.
 for (let index = 0; index < parsed.length; index++) results.push(await run(parsed[index], index));
 console.log(JSON.stringify({platform:process.platform, results}, null, 2));
})().catch(e => { console.error(e.stack); process.exitCode=1; });
