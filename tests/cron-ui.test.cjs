const {test}=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs');
const vm=require('node:vm');
const code=fs.readFileSync(new URL('../examples/cron/app.js',`file://${__filename}`),'utf8');
function browser() {
  class Element {
    constructor(){this.value='';this.checked=false;this.children=[];this.listeners={};this.hidden=false;}
    addEventListener(event,fn){this.listeners[event]=fn;}
    append(...children){this.children.push(...children);}
    replaceChildren(...children){this.children=children;}
    setAttribute(){}
    get childElementCount(){return this.children.length;}
  }
  const nodes=new Map(); const get=id=>{if(!nodes.has(id))nodes.set(id,new Element());return nodes.get(id);};
  Object.entries({source:'export default async function(ctx) {}',name:'Test',kind:'manual',timezone:'UTC',timeout:'60'}).forEach(([id,v])=>get(id).value=v);
  get('includeSource').checked=true;
  const requests=[]; let resolveAi;
  const context=vm.createContext({document:{getElementById:get,createElement:()=>new Element()},AbortController,Intl,Date,confirm:()=>true,fetch:async(url,options)=>{
    requests.push({action:url.slice(5),body:JSON.parse(options.body)});
    if(url==='/api/ai')return new Promise(resolve=>{resolveAi=text=>resolve({ok:true,json:async()=>({text})});});
    let value=[];
    if(url==='/api/status')value={machine_off:'Off means off.',global_concurrency:4,last_tick:null};
    if(url==='/api/draft')value={revision_id:'r1',job_id:'j1',digest:'digest',body:JSON.parse(options.body)};
    if(url==='/api/publish')value={ok:true};
    return {ok:true,json:async()=>value};
  }});
  vm.runInContext(code,context);
  const fire=(id,event='click')=>get(id).listeners[event]?.({preventDefault(){}});
  return {nodes,get,context,requests,fire,resolveAi:text=>resolveAi(text)};
}
const tick=()=>new Promise(resolve=>setImmediate(resolve));
test('publication requires reviewed saved revision and edits invalidate review',async()=>{
  const b=browser();await tick();b.fire('editor','submit');await tick();
  assert.equal(b.get('review').hidden,false);assert.equal(b.get('publish').disabled,true);
  b.get('approved').checked=true;b.fire('approved','change');b.fire('publish');await tick();
  assert.deepEqual(b.requests.find(r=>r.action==='publish').body,{revision_id:'r1',expected_current_revision:null});
  b.fire('editor','submit');await tick();b.get('source').value+='\n// edit';b.fire('editor','input');
  assert.equal(b.get('review').hidden,true);assert.equal(b.get('publish').disabled,true);
});
test('late AI responses and cancellation cannot overwrite edits or publish',async()=>{
  const b=browser();await tick();b.get('instruction').value='Write a job';b.fire('generate');
  b.get('source').value='manual edits';b.fire('editor','input');b.resolveAi('generated');await tick();
  assert.equal(b.get('source').value,'manual edits');assert.equal(b.get('suggestionPanel').hidden,true);
  b.fire('generate');b.fire('abortAi');b.resolveAi('late cancelled response');await tick();
  assert.equal(b.get('source').value,'manual edits');assert.equal(b.requests.some(r=>r.action==='draft'||r.action==='publish'),false);
});
test('AI context excludes logs unless explicitly selected and bounds prompts',async()=>{
  const b=browser();await tick();b.get('instruction').value='x'.repeat(16001);b.fire('generate');await tick();
  assert.equal(b.requests.some(r=>r.action==='ai'),false);
  b.get('instruction').value='Help';b.fire('generate');
  assert.match(b.requests.find(r=>r.action==='ai').body.prompt,/HISTORY AND LOGS\n\(not included\)/);
  b.fire('abortAi');b.resolveAi('late');await tick();
});
test('source review marks removed and added lines',()=>{
  const b=browser();assert.equal(vm.runInContext("sourceDiff('a\\nb','a\\nc')",b.context),'  a\n- b\n+ c');
});
test('HTTP adapter rejects foreign origins and oversized inputs without invoking capabilities',t=>{
  const {spawnSync}=require('node:child_process');
  const deno=process.env.DENO_BIN||'deno';
  if(spawnSync(deno,['--version']).status!==0) { t.skip('Deno is required for HTTP adapter integration'); return; }
  const moduleUrl=new URL('../examples/cron/main.ts',`file://${__filename}`).href;
  const result=spawnSync(deno,['eval',`
    import app from ${JSON.stringify(moduleUrl)};
    let calls=0;
    const ctx={tasks:{command:async()=>{calls++;return [];}},ai:{complete:async()=>{calls++;return {text:'draft'};}}};
    const req=(path,body,origin='http://cron.localhost')=>new Request('http://cron.localhost/api/'+path,{method:'POST',headers:{origin,'content-type':'application/json'},body:JSON.stringify(body)});
    if((await app.fetch(req('list',{},'https://evil.example'),ctx)).status!==403)throw Error('foreign origin');
    if((await app.fetch(req('draft',{source:'x'.repeat(49000)}),ctx)).status!==400)throw Error('body bound');
    if((await app.fetch(req('ai',{prompt:'x'.repeat(16001)}),ctx)).status!==400)throw Error('prompt bound');
    if(calls!==0)throw Error('capability invoked on rejected input');
    if((await app.fetch(req('list',{}),ctx)).status!==200||calls!==1)throw Error('authorized action');
    if((await app.fetch(req('unknown',{}),ctx)).status!==404||calls!==1)throw Error('action whitelist');
    if((await app.fetch(new Request('http://cron.localhost/../../secrets'),ctx)).status!==404)throw Error('asset whitelist');
    for (const [error,status] of [
      ['publication conflict: current revision changed; review again',409],
      ['task requests network access outside deployment grants',403],
      ['unknown revision',404],
      ['task validation failed: syntax error at file:///home/private/tasks/task.ts:2:3',400],
      ['dependency validation failed; task must be a self-contained TypeScript module',400],
      ['Task dependencies are not supported yet; use a self-contained TypeScript module',400],
    ]) {
      const errorCtx={tasks:{command:async()=>{throw Error(error);}},ai:ctx.ai};
      const response=await app.fetch(req('publish',{}),errorCtx);const value=await response.json();
      if(response.status!==status)throw Error('error status mapping');
      if(value.error.includes('/home/private'))throw Error('private path leaked');
      if(error.startsWith('task validation')&&!value.error.includes('syntax error'))throw Error('validation detail lost');
      if(error.startsWith('dependency validation')&&!value.error.includes('dependency validation failed'))throw Error('dependency validation detail lost');
      if(error.startsWith('Task dependencies')&&!value.error.includes('self-contained'))throw Error('dependency contract guidance lost');
    }
    const originalTimeout=globalThis.setTimeout;
    globalThis.setTimeout=(fn,ms,...args)=>originalTimeout(fn,ms===5000?10:ms,...args);
    const stalled=new Request('http://cron.localhost/api/draft',{method:'POST',headers:{origin:'http://cron.localhost','content-type':'application/json'},body:new ReadableStream({start(){}})});
    if((await app.fetch(stalled,ctx)).status!==408)throw Error('body deadline');
    globalThis.setTimeout=originalTimeout;

  `],{encoding:'utf8'});
  assert.equal(result.status,0,result.stderr);
});
test('real Cron browser lifecycle uses explicit review and immutable revision publication', {skip: !process.env.RUN_CRON_BROWSER, timeout: 120000}, async()=>{
  const {mkdtemp,mkdir,writeFile,cp,rm}=require('node:fs/promises');
  const {tmpdir}=require('node:os');const {join}=require('node:path');const {spawn}=require('node:child_process');const net=require('node:net');
  const {chromium}=require('@playwright/test');
  const dir=await mkdtemp(join(tmpdir(),'paraco-cron-browser-'));
  const server=net.createServer();await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));const port=server.address().port;await new Promise(resolve=>server.close(resolve));
  await cp(join(process.cwd(),'examples/cron'),join(dir,'cron'),{recursive:true});await mkdir(join(dir,'tmp'));
  await writeFile(join(dir,'server.json'),JSON.stringify({apps:[{path:'cron',taskAuthoring:true,taskNetwork:[]}]}));
  const child=spawn(process.env.PARACO_BIN||join(process.cwd(),'target/debug/paraco'),['--log-dir',join(dir,'logs'),'serve','--config',join(dir,'server.json'),'--port',String(port)],{cwd:process.cwd(),stdio:['ignore','pipe','pipe'],env:{...process.env,TMPDIR:join(dir,'tmp')}});
  let output='';child.stdout.on('data',data=>output+=data);child.stderr.on('data',data=>output+=data);
  let browser;
  const wait=async(fn,label)=>{const end=Date.now()+30000;while(Date.now()<end){if(child.exitCode!==null)throw Error('runtime exited: '+output);if(await fn())return;await new Promise(resolve=>setTimeout(resolve,50));}throw Error('Timed out: '+label+'\n'+output);};
  try {
    await wait(()=>output.includes('management dashboard'),'server readiness');
    browser=await chromium.launch({headless:true,args:['--no-sandbox']});const page=await browser.newPage();
    page.on('dialog',dialog=>dialog.accept());
    await page.goto(`http://cron.localhost:${port}/`);await page.getByRole('heading',{name:'Cron',exact:true}).waitFor();
    await wait(async()=>!(await page.locator('#runtime').textContent()).includes('Last scheduler check: undefined')&&(await page.locator('#runtime').textContent()).length>0,'tasks ready');
    await page.locator('#name').fill('Browser lifecycle');await page.locator('#source').fill('export default async function(ctx: {log:(s:string)=>void;notify:(s:string)=>void}) { ctx.log("browser-run-ok"); ctx.notify("browser notification"); }');
    await page.getByRole('button',{name:'Save draft & review',exact:true}).click();await page.locator('#review').waitFor({state:'visible'});
    assert.equal(await page.locator('#publish').isDisabled(),true);
    await page.locator('#approved').check();await page.locator('#source').fill('export default async function(ctx: {log:(s:string)=>void;notify:(s:string)=>void}) { ctx.log("browser-run-ok"); ctx.notify("browser notification"); }\n// reviewed version');
    assert.equal(await page.locator('#review').isHidden(),true);
    await page.getByRole('button',{name:'Save draft & review',exact:true}).click();await page.locator('#review').waitFor({state:'visible'});await page.locator('#approved').check();await page.locator('#publish').click();
    await page.getByRole('status').filter({hasText:'Reviewed revision published.'}).waitFor();
    await page.locator('#run').click();await wait(async()=>{await page.locator('#refresh').click();const history=await page.locator('#history').textContent();return history.includes('browser-run-ok') && history.includes('success');},'successful job outcome and run log');
    assert.match(await page.locator('#history').textContent(),/success/);
    await page.locator('#archive').click();await page.getByRole('status').filter({hasText:'Job archived.'}).waitFor();assert.equal(await page.locator('#run').isDisabled(),true);
    await page.locator('#archive').click();await page.getByRole('status').filter({hasText:'Job unarchived.'}).waitFor();
    await page.locator('#revisions details').first().locator('summary').click();await page.getByRole('button',{name:'Restore as editable draft'}).first().click();
    assert.equal(await page.locator('#review').isHidden(),true);
    assert.match(await page.getByRole('status').textContent(),/Restored to the editor/);
    await page.locator('#kind').selectOption('cron');await page.locator('#timezone').fill('America/Denver');await page.locator('#expression').fill('0 9 * * *');await page.locator('#preview').click();
    await wait(async()=>/T\d\d:/.test(await page.locator('#occurrences').textContent()),'timezone schedule preview');
    await page.locator('#source').fill('export default async function(ctx) { syntax error here }');await page.getByRole('button',{name:'Save draft & review',exact:true}).click();
    await page.locator('#review').waitFor({state:'visible'});await page.locator('#approved').check();
    const failedPublication = page.waitForResponse(response => response.url().endsWith('/api/publish') && response.request().method()==='POST');
    await page.locator('#publish').click();
    const failure = await failedPublication; assert.equal(failure.status(),400);
    const validation = await failure.json(); assert.match(validation.error,/validation failed/,JSON.stringify(validation));
    await wait(async()=>/validation failed/.test(await page.locator('#message').textContent()),'visible syntax validation error');
    assert.equal(await page.locator('#state').textContent(),'Saved draft awaiting review');
  } finally {
    if(browser)await browser.close();
    const exited=new Promise(resolve=>child.once('exit',resolve));child.kill('SIGINT');await Promise.race([exited,new Promise(resolve=>setTimeout(resolve,5000))]);if(child.exitCode===null){child.kill('SIGKILL');await exited;}
    await rm(dir,{recursive:true,force:true});
  }
});
