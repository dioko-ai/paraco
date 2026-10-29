// Isolated dashboard + worker + SDK fixture: no provider account or paid request.
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const os = require('node:os');
const net = require('node:net');
const { spawn } = require('node:child_process');
const { chromium } = require('@playwright/test');
(async () => {
  const dir = await fs.mkdtemp(path.join(os.tmpdir(), 'paraco-ai-browser-'));
  let child, browser, output = '';
  try {
    const listener = net.createServer();
    await new Promise(r => listener.listen(0, '127.0.0.1', r));
    const port = listener.address().port;
    await new Promise(r => listener.close(r));
    const origin = `http://localhost:${port}`;
    const appDir = path.join(dir, 'app');
    await fs.mkdir(appDir);
    await fs.writeFile(path.join(appDir, 'paraco.json'), JSON.stringify({ name: 'ai-demo', entrypoint: 'main.ts', capabilities: ['ai'] }));
    await fs.writeFile(path.join(appDir, 'main.ts'), `export default { async fetch(request, context) { try { return Response.json(await context.ai.complete({prompt:'hello'})); } catch (error) { return new Response(String(error),{status:403}); } } };`);
    const module = path.join(dir, 'fixture.mjs');
    await fs.writeFile(module, `export async function* query({prompt, options}) { yield {type:'result',subtype:'success',result:options.model+':'+prompt}; }`);
    await fs.writeFile(path.join(dir, 'server.json'), JSON.stringify({ apps: [{path: 'app'}] }));
    const cliDir = path.join(dir, 'bin');
    await fs.mkdir(cliDir);
    await fs.writeFile(path.join(cliDir, 'codex'), `#!${process.execPath}
if (process.argv[2] === 'login') process.exit(require('node:fs').existsSync(require('node:path').join(__dirname, 'signed-out')) ? 1 : 0);
let prompt=''; process.stdin.on('data', chunk=>prompt+=chunk); process.stdin.on('end',()=>{const args=process.argv.slice(2); const i=args.indexOf('--model'); process.stdout.write((i<0?'codex-default':args[i+1])+':'+prompt);});`, {mode:0o700});
    child = spawn(process.env.PARACO_BIN || path.join(process.cwd(), 'target/debug/paraco'), ['--data-dir', dir, '--log-dir', path.join(dir,'logs'), 'serve', '--config', path.join(dir,'server.json'), '--port', String(port)], {
      env: { ...process.env, PATH: `${cliDir}:${path.join(process.cwd(),'node_modules/.bin')}:/usr/bin:/bin`, PARACO_ALLOW_UNSANDBOXED:'1' }, stdio: ['ignore','pipe','pipe'],
    });
    child.stdout.on('data', x => output += x); child.stderr.on('data', x => output += x);
    const until = async fn => { for (let i=0; i<200; i++) { try { const value = await fn(); if (value) return value; } catch {} await new Promise(r=>setTimeout(r,100)); } throw Error(output || 'Timed out'); };
    await until(async () => (await fetch(`${origin}/api/apps`)).ok);
    browser = await chromium.launch({ headless:true });
    const page = await browser.newPage();
    const errors=[]; page.on('pageerror', e=>errors.push(e.message));
    await page.goto(origin);
    await page.locator('#ai-refresh').click();
    await page.locator('#ai-form').waitFor({state:'visible'});
    await page.locator('#ai-mode').selectOption('custom');
    await page.locator('#ai-provider').selectOption('anthropic');
    await page.locator('#ai-advanced summary').click();
    await page.locator('#ai-transport').selectOption('sdk');
    await page.locator('#ai-model').fill('global-model');
    await page.locator('#ai-node').fill(process.execPath);
    await page.locator('#ai-module').fill(module);
    const save = async () => { await page.locator('#ai-status').evaluate(n=>n.textContent=''); await page.locator('#ai-form button').click(); await page.waitForFunction(()=>document.querySelector('#ai-status').textContent.startsWith('Saved.')); };
    await save();
    // Node fetch cannot resolve *.localhost everywhere; use the gateway Host header.
    const appRequest = () => new Promise((resolve,reject) => {
      const req = require('node:http').get({hostname:'127.0.0.1',port,path:'/',headers:{Host:`ai-demo.localhost:${port}`}}, response => {
        let text=''; response.on('data', chunk=>text+=chunk); response.on('end',()=>resolve({status:response.statusCode, ok:response.statusCode===200, text:async()=>text}));
      }); req.on('error',reject); req.setTimeout(5000,()=>req.destroy(Error('App request timed out')));
    });
    await until(async () => { const r=await appRequest(); const text=await r.text(); output = `Last app response: ${r.status} ${text.slice(0,500)}\n` + output.slice(-2000); return r.ok && text.includes('global-model:hello'); });
    await page.locator('#ai-scope').selectOption('ai-demo');
    assert.equal(await page.locator('#ai-mode').inputValue(), 'inherit');
    await page.locator('#ai-mode').selectOption('custom');
    await page.locator('#ai-provider').selectOption('anthropic');
    await page.locator('#ai-advanced summary').click();
    await page.locator('#ai-transport').selectOption('sdk');
    await page.locator('#ai-model').fill('app-model');
    await page.locator('#ai-node').fill(process.execPath);
    await page.locator('#ai-module').fill(module);
    await save();
    assert.match(await (await appRequest()).text(), /app-model:hello/);
    await page.locator('#ai-mode').selectOption('disabled'); await save();
    assert.equal((await appRequest()).status, 403);
    await page.locator('#ai-mode').selectOption('inherit'); await save();
    assert.match(await (await appRequest()).text(), /global-model:hello/);
    await page.locator('#ai-scope').selectOption('');
    await page.locator('#ai-provider').selectOption('codex');
    await page.locator('#ai-model').fill('');
    assert.equal(await page.locator('#ai-api-fields').isVisible(), false);
    assert.equal(await page.locator('#ai-sdk-fields').isVisible(), false);
    assert.equal(await page.locator('#ai-advanced').isVisible(), false);
    await save();
    assert.equal(await page.locator('#ai-provider').inputValue(), 'codex');
    assert.equal(await page.locator('#ai-model').inputValue(), '');
    assert.match(await (await appRequest()).text(), /codex-default:hello/);
    await page.locator('#ai-model').fill('chosen-model'); await save();
    assert.match(await (await appRequest()).text(), /chosen-model:hello/);
    await fs.writeFile(path.join(cliDir, 'signed-out'), '');
    await page.locator('#ai-form button').click();
    await page.waitForFunction(()=>document.querySelector('#ai-status').textContent.includes('Codex needs sign-in'));
    await fs.unlink(path.join(cliDir, 'signed-out'));
    await fs.unlink(path.join(cliDir, 'codex'));
    await page.locator('#ai-form button').click();
    await page.waitForFunction(()=>document.querySelector('#ai-status').textContent.includes('Codex CLI was not found'));
    assert.match(await (await fetch(`${origin}/api/ai`)).text(), /chosen-model/);
    await page.locator('#ai-provider').selectOption('xai');
    assert.equal(await page.locator('#ai-transport').inputValue(), 'api');
    await page.locator('#ai-model').fill('grok-model');
    await page.locator('#ai-key').fill('fixture-api-secret'); await save();
    assert.equal(await page.locator('#ai-key').inputValue(), '');
    const settings = await (await fetch(`${origin}/api/ai`)).text();
    assert.ok(!settings.includes('fixture-api-secret'));
    const stored = JSON.parse(await fs.readFile(path.join(dir,'ai/settings.json'),'utf8'));
    assert.equal((await fs.stat(stored.default.credential_file)).mode & 0o077, 0);
    assert.equal((await fetch(`${origin}/api/ai`, {method:'POST', headers:{origin:`http://ai-demo.localhost:${port}`, 'content-type':'application/json'}, body:'{}'})).status,403);
    assert.deepEqual(errors,[]);
    console.log('AI dashboard, inheritance, overrides, disable, SDK worker calls, key redaction and origin checks passed');
  } finally {
    await browser?.close();
    if (child && child.exitCode === null) { const exited = new Promise(r=>child.once('exit',r)); child.kill('SIGINT'); const timer=setTimeout(()=>child.kill('SIGKILL'),5000); await exited; clearTimeout(timer); }
    await fs.rm(dir,{recursive:true,force:true});
  }
})().catch(e=>{ console.error(e); process.exitCode=1; });
