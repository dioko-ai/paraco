const {test}=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs');
const vm=require('node:vm');
const code=fs.readFileSync(new URL('../examples/cron/app.js',`file://${__filename}`),'utf8');
function browser(handler) {
  class Element {
    constructor(){this.value='';this.checked=false;this.children=[];this.listeners={};this.hidden=false;this.dataset={};}
    addEventListener(event,fn){this.listeners[event]=fn;}
    append(...children){this.children.push(...children);}
    replaceChildren(...children){this.children=children;}
    setAttribute(){}
    contains(){return false;}
    get childElementCount(){return this.children.length;}
  }
  const nodes=new Map(); const get=id=>{if(!nodes.has(id))nodes.set(id,new Element());return nodes.get(id);};
  Object.entries({source:'export default async function(ctx) {}',name:'Test',kind:'manual',timezone:'UTC',timeout:'60',frequency:'daily',time:'09:00'}).forEach(([id,v])=>get(id).value=v);
  get('includeSource').checked=true;
  const requests=[]; let resolveAi;
  const context=vm.createContext({document:{getElementById:get,createElement:()=>new Element()},AbortController,Intl,Date,window:{addEventListener(){}},setInterval(){},confirm:()=>true,fetch:async(url,options)=>{
    requests.push({action:url.slice(5),body:JSON.parse(options.body)});
    if (handler) { const result=handler(url.slice(5), JSON.parse(options.body)); if (result !== undefined) return await result; }
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
const response=value=>({ok:true,json:async()=>value});
const deferred=()=>{let resolve;const promise=new Promise(r=>resolve=r);return {promise,resolve};};
test('save keeps new job identity when edits arrive and blocks double submit',async()=>{
  const pending=deferred();const b=browser(action=>action==='draft'?pending.promise:undefined);await tick();
  b.fire('editor','submit');b.fire('editor','submit');await tick();
  assert.equal(b.requests.filter(r=>r.action==='draft').length,1);
  b.get('source').value='newer';b.fire('editor','input');
  pending.resolve(response({job_id:'j1',revision_id:'r1',digest:'x',body:{source:'older'}}));await tick();await tick();
  assert.equal(vm.runInContext('state.selected',b.context),'j1');
  assert.equal(b.get('source').value,'newer');assert.equal(b.get('review').hidden,true);
});
test('selection locks placeholder fields until the actual revision loads',async()=>{
  const pending=deferred();const b=browser(action=>action==='revisions'?pending.promise:action==='history'?response([{id:'old'}]):undefined);await tick();
  vm.runInContext("state.jobs=[{id:'j1',name:'Job',current_revision:null}];state.runs=[{id:'other-job'}]",b.context);
  vm.runInContext("selectJob('j1')",b.context);await tick();
  assert.equal(vm.runInContext('state.runs.length',b.context),0);
  assert.equal(b.get('source').disabled,true);
  assert.equal(b.get('kind').disabled,true);
  pending.resolve(response([{id:'r',body:{name:'Old',source:'old',schedule:{kind:'manual'},timeout_ms:60000,network:[]}}]));await tick();await tick();
  assert.equal(b.get('source').value,'old');
  assert.equal(b.get('source').disabled,false);
});
test('paging sends offsets and periodic refresh preserves open rows',async()=>{
  const revisions=Array.from({length:10},(_,i)=>({id:'r'+i,created:0,body:{}}));
  const b=browser(action=>action==='revisions'?response(revisions):action==='history'?response([]):undefined);await tick();
  vm.runInContext("state.selected='j1'",b.context);await vm.runInContext("loadDetails('j1')",b.context);
  b.get('revisions').children[0].open=true;
  await vm.runInContext('refresh()',b.context);assert.equal(b.get('revisions').children[0].open,true);
  b.fire('moreRevisions');await tick();
  assert.equal(b.requests.filter(r=>r.action==='revisions').at(-1).body.offset,10);
  b.fire('prevRevisions');await tick();assert.equal(b.requests.filter(r=>r.action==='revisions').at(-1).body.offset,0);
});
test('failed publishing remains retryable',async()=>{
  let attempts=0;const b=browser(action=>action==='publish'&&++attempts===1?{ok:false,status:400,json:async()=>({error:'try again'})}:undefined);await tick();
  b.fire('editor','submit');await tick();b.get('approved').checked=true;b.fire('approved','change');b.fire('publish');await tick();
  assert.equal(b.get('publish').disabled,false);b.fire('publish');await tick();assert.equal(attempts,2);
});
test('older refresh responses cannot overwrite a newer refresh',async()=>{
  const delayed=deferred();let lists=0;const b=browser(action=>action==='list'?(++lists===1?delayed.promise:response([{id:'new',name:'New'}])):undefined);
  await vm.runInContext('refresh()',b.context);delayed.resolve(response([{id:'old',name:'Old'}]));await tick();
  assert.equal(vm.runInContext('state.jobs[0].id',b.context),'new');
});
test('publishing a saved revision does not mark it dirty',async()=>{
  const b=browser();await tick();b.fire('editor','submit');await tick();b.get('approved').checked=true;b.fire('approved','change');b.fire('publish');await tick();
  assert.equal(vm.runInContext('state.dirty',b.context),false);
});
test('selection blocks saving and generation until the requested job loads',async()=>{
  const pending=deferred();const b=browser(action=>action==='revisions'?pending.promise:undefined);await tick();
  vm.runInContext("state.jobs=[{id:'j1',name:'Job'}]",b.context);vm.runInContext("selectJob('j1')",b.context);await tick();
  assert.equal(b.get('save').disabled,true);assert.equal(b.get('generate').disabled,true);b.fire('editor','submit');b.fire('generate');await tick();
  assert.equal(b.requests.some(r=>r.action==='draft'||r.action==='ai'),false);
  pending.resolve(response([]));await tick();await tick();assert.equal(b.get('save').disabled,false);
});
test('newest saved draft loads even when a published revision exists',async()=>{
  const draft={name:'Job',source:'draft code',schedule:{kind:'manual'},timeout_ms:60000,network:[],notify_success:false,notify_failure:true};
  const b=browser(action=>action==='revisions'?response([{id:'draft',published:null,body:draft},{id:'pub',published:10,body:{...draft,source:'published'}}]):undefined);await tick();
  vm.runInContext("state.jobs=[{id:'j1',name:'Job',current_revision:'pub'}]",b.context);await vm.runInContext("selectJob('j1')",b.context);
  assert.equal(b.get('source').value,'draft code');assert.equal(b.get('state').textContent,'Saved draft loaded');
});
test('one-off timestamps preserve milliseconds on an unchanged round trip',()=>{
  const b=browser();const at=Date.parse('2026-11-01T08:30:15.123Z');
  b.context.testAt=at;vm.runInContext("setDraft({name:'x',source:'x',schedule:{kind:'once',at:testAt},timeout_ms:60000,network:[]})",b.context);
  assert.equal(vm.runInContext('schedule().at',b.context),at);assert.match(b.get('at').value,/15\.123/);
});
test('delete reports incomplete artifact cleanup',async()=>{
  const b=browser(action=>action==='delete'?response({ok:true,artifacts_cleaned:false}):undefined);await tick();
  vm.runInContext("state.jobs=[{id:'j1',name:'Job',archived:true}];state.selected='j1'",b.context);b.fire('delete');await tick();await tick();
  assert.match(b.get('message').textContent,/artifacts could not be removed/);
});
test('source review marks removed and added lines',()=>{
  const b=browser();assert.equal(vm.runInContext("sourceDiff('a\\nb','a\\nc')",b.context),'  a\n- b\n+ c');
});
test('HTTP adapter rejects foreign origins and oversized inputs without invoking capabilities',async()=>{
  const esbuild=require('esbuild');
  const entry=new URL('../examples/cron/main.ts',`file://${__filename}`).pathname;
  const bundle=esbuild.buildSync({entryPoints:[entry],bundle:true,platform:'node',format:'cjs',write:false});
  const loaded={exports:{}};
  new Function('module','exports',bundle.outputFiles[0].text)(loaded,loaded.exports);
  const app=loaded.exports.default;
    let calls=0;
    const ctx={tasks:{command:async()=>{calls++;return [];}},ai:{complete:async()=>{calls++;return {text:'draft'};}}};
    const req=(path,body,origin='http://cron.localhost')=>new Request('http://cron.localhost/api/'+path,{method:'POST',headers:{origin,'content-type':'application/json'},body:JSON.stringify(body)});
    if((await app.fetch(req('list',{},'https://evil.example'),ctx)).status!==403)throw Error('foreign origin');
    if((await app.fetch(req('draft',{source:'x'.repeat(6 * 256 * 1024 + 64 * 1024)}),ctx)).status!==400)throw Error('body bound');
    if((await app.fetch(req('ai',{prompt:'x'.repeat(16001)}),ctx)).status!==400)throw Error('prompt bound');
    if(calls!==0)throw Error('capability invoked on rejected input');
    assert.equal((await app.fetch(req('draft',{source:'\u0001'.repeat(256 * 1024)}),ctx)).status,200);
    calls=0;
    if((await app.fetch(req('list',{}),ctx)).status!==200||calls!==1)throw Error('authorized action');
    if((await app.fetch(req('unknown',{}),ctx)).status!==404||calls!==1)throw Error('action whitelist');
    if((await app.fetch(new Request('http://cron.localhost/../../secrets'),ctx)).status!==404)throw Error('asset whitelist');
    for (const [error,status] of [
      ['publication conflict: current revision changed; review again',409],
      ['job has an active run',409],
      ['archive job before deleting it',409],
      ['task requests network access outside deployment grants',403],
      ['unknown revision',404],
      ['task validation failed: syntax error at file:///home/private/tasks/task.ts:2:3',400],
      ['dependency validation failed; task must be a self-contained TypeScript module',400],
      ['Task dependencies are not supported yet; use a self-contained TypeScript module',400],
    ]) {
      const errorCtx={tasks:{command:async()=>{throw Error(error);}},ai:ctx.ai};
      const response=await app.fetch(req('publish',{}),errorCtx);const value=await response.json();
      if(response.status!==status)throw Error('error status mapping');
      if((error==='job has an active run'||error==='archive job before deleting it')&&value.error!==error)throw Error('delete guidance lost');
      if(value.error.includes('/home/private'))throw Error('private path leaked');
      if(error.startsWith('task validation')&&!value.error.includes('syntax error'))throw Error('validation detail lost');
      if(error.startsWith('dependency validation')&&!value.error.includes('dependency validation failed'))throw Error('dependency validation detail lost');
      if(error.startsWith('Task dependencies')&&!value.error.includes('self-contained'))throw Error('dependency contract guidance lost');
    }
    const originalTimeout=globalThis.setTimeout;
    globalThis.setTimeout=(fn,ms,...args)=>originalTimeout(fn,ms===5000?10:ms,...args);
    const stalled=new Request('http://cron.localhost/api/draft',{method:'POST',headers:{origin:'http://cron.localhost','content-type':'application/json'},body:new ReadableStream({start(){}}),duplex:'half'});
    if((await app.fetch(stalled,ctx)).status!==408)throw Error('body deadline');
    globalThis.setTimeout=originalTimeout;
});
test('real Cron browser lifecycle uses explicit review and immutable revision publication', {skip: !process.env.RUN_CRON_BROWSER, timeout: 120000}, async()=>{
  const {mkdtemp,mkdir,writeFile,cp,rm}=require('node:fs/promises');
  const {tmpdir}=require('node:os');const {join}=require('node:path');const {spawn}=require('node:child_process');const net=require('node:net');
  const {chromium}=require('@playwright/test');
  const dir=await mkdtemp(join(tmpdir(),'paraco-cron-browser-'));
  const server=net.createServer();await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));const port=server.address().port;await new Promise(resolve=>server.close(resolve));
  await cp(join(process.cwd(),'examples/cron'),join(dir,'cron'),{recursive:true});await mkdir(join(dir,'tmp'));
  await writeFile(join(dir,'server.json'),JSON.stringify({apps:[{path:'cron',taskAuthoring:true,taskNetwork:[]}]}));
  const child=spawn(process.env.PARACO_BIN||join(process.cwd(),'target/debug/paraco'),['--log-dir',join(dir,'logs'),'--data-dir',dir,'serve','--config',join(dir,'server.json'),'--port',String(port)],{cwd:process.cwd(),stdio:['ignore','pipe','pipe'],env:{...process.env,TMPDIR:join(dir,'tmp')}});
  let output='';child.stdout.on('data',data=>output+=data);child.stderr.on('data',data=>output+=data);
  let browser;
  const wait=async(fn,label)=>{const end=Date.now()+30000;while(Date.now()<end){if(child.exitCode!==null)throw Error('runtime exited: '+output);if(await fn())return;await new Promise(resolve=>setTimeout(resolve,50));}throw Error('Timed out: '+label+'\n'+output);};
  try {
    await wait(()=>output.includes('dashboard listening'),'server readiness');
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
    const [runResponse]=await Promise.all([page.waitForResponse(response=>response.url().endsWith('/api/run')&&response.request().method()==='POST'),page.locator('#run').click()]);
    assert.equal(runResponse.ok(),true);const jobId=runResponse.request().postDataJSON().job_id;const {run_id:runId}=await runResponse.json();
    let lastRuns;
    try {
      await wait(async()=>{
        // Serialize refreshes through this job's history response and rendered state.
        // Clicking repeatedly before fetch completes can supersede every detail load.
        const [response]=await Promise.all([page.waitForResponse(response=>response.url().endsWith('/api/history')&&response.request().method()==='POST'&&response.request().postDataJSON().job_id===jobId,{timeout:5000}),page.locator('#refresh').click()]);
        assert.equal(response.ok(),true);lastRuns=await response.json();const run=lastRuns.find(run=>run.id===runId);
        if(run)await page.waitForFunction(({id,status})=>[...document.querySelectorAll('#history details')].some(row=>row.dataset.id===id&&row.querySelector('summary').textContent.includes(status)),{id:runId,status:run.status},{timeout:5000});
        const history=await page.locator('#history').textContent();return history.includes('browser-run-ok')&&history.includes('success');
      },'successful job outcome and run log');
    } catch(error) {
      error.message+='\nHistory API: '+JSON.stringify(lastRuns)+'\nRendered history: '+await page.locator('#history').textContent()+'\nStatus: '+await page.getByRole('status').textContent();throw error;
    }
    assert.match(await page.locator('#history').textContent(),/success/);
    await page.locator('#archive').click();await page.getByRole('status').filter({hasText:'Job archived.'}).waitFor();assert.equal(await page.locator('#run').isDisabled(),true);
    await page.locator('#archive').click();await page.getByRole('status').filter({hasText:'Job unarchived.'}).waitFor();
    await page.locator('#revisions details').first().locator('summary').click();await page.getByRole('button',{name:'Restore as editable draft'}).first().click();
    assert.equal(await page.locator('#review').isHidden(),true);
    assert.match(await page.getByRole('status').textContent(),/Restored to the editor/);
    await page.locator('#kind').selectOption('cron');await page.locator('#frequency').selectOption('advanced');await page.locator('#timezone').fill('America/Denver');await page.locator('#expression').fill('0 9 * * *');await page.locator('#preview').click();
    await wait(async()=>/America\/Denver/.test(await page.locator('#occurrences').textContent()),'timezone schedule preview');
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
test('Chromium exercises guided schedules, races, paging and refresh', {skip: !process.env.RUN_CRON_MOCK_BROWSER, timeout: 60000}, async()=>{
  const http=require('node:http');const {chromium}=require('@playwright/test');
  const html=fs.readFileSync(new URL('../examples/cron/index.html',`file://${__filename}`));
  const app=fs.readFileSync(new URL('../examples/cron/app.js',`file://${__filename}`));
  const requests=[];let draftGate,revisionGate;let current=null;let notifications=[{id:'n',message:'Notice',created:1,read:false}];
  const jobs=[{id:'j1',name:'Existing',current_revision:'published',archived:false}];
  const body={name:'Existing',source:'export default async function() {}',schedule:{kind:'manual'},timeout_ms:60000,network:[],notify_success:false,notify_failure:true};
  const revisions=Array.from({length:11},(_,i)=>({id:i===0?'published':'r'+i,body,created:i,published:i===0}));
  const runs=Array.from({length:501},(_,i)=>({id:'run'+i,revision_id:'published',started:i,status:'success',logs:'ok'}));
  const server=http.createServer(async(req,res)=>{
    if(req.method==='GET'){res.setHeader('Content-Type',req.url==='/app.js'?'text/javascript':'text/html');return res.end(req.url==='/app.js'?app:html);}
    let raw='';for await(const chunk of req)raw+=chunk;const input=JSON.parse(raw),action=req.url.slice(5);requests.push({action,input});
    if(action==='draft'&&draftGate)await draftGate.promise;
    if(action==='revisions'&&revisionGate)await revisionGate.promise;
    let value=[];
    if(action==='list')value=jobs;
    if(action==='notifications')value=notifications;
    if(action==='status')value={machine_off:'Ready',last_tick:null,global_concurrency:4};
    if(action==='revisions')value=revisions.slice(input.offset||0,(input.offset||0)+10);
    if(action==='history')value=runs.slice(input.offset||0,(input.offset||0)+500);
    if(action==='revision')value=revisions[0];
    if(action==='preview')value={times:[]};
    if(action==='draft'){current=input;value={job_id:'new-job',revision_id:'new-rev',digest:'abc',body:input};if(!jobs.some(j=>j.id==='new-job'))jobs.push({id:'new-job',name:input.name,current_revision:null,archived:false});}
    if(action==='publish')value={ok:true};
    res.setHeader('Content-Type','application/json');res.end(JSON.stringify(value));
  });
  await new Promise(r=>server.listen(0,'127.0.0.1',r));let browser;
  try {
    browser=await chromium.launch({headless:true,args:['--no-sandbox']});const page=await browser.newPage({timezoneId:'America/Denver'});
    await page.addInitScript(()=>{window.setInterval=fn=>{window.cronPoll=fn;return 1;};});
    page.on('dialog',dialog=>dialog.dismiss());
    await page.goto(`http://127.0.0.1:${server.address().port}/`);await page.getByRole('button',{name:'Existing'}).waitFor();
    assert.equal(await page.evaluate(()=>{const cards=['history','notifications','instruction','revisions'].map(id=>document.getElementById(id).closest('.card'));return new Set(cards).size===4 && cards.every(card=>card.parentElement===cards[0].parentElement);}),true,'Runs, local inbox, AI, and revision history must be sibling cards');
    assert.equal(await page.locator('#timezone').inputValue(),'America/Denver');
    await page.locator('#kind').selectOption('cron');await page.locator('#time').fill('14:35');await page.locator('#preview').click();
    await page.waitForFunction(()=>document.querySelector('#occurrences').textContent.includes('Manual schedule'));
    assert.equal(requests.filter(r=>r.action==='preview').at(-1).input.schedule.expression,'35 14 * * *');
    await page.locator('#frequency').selectOption('weekly');await page.locator('#weekday').selectOption('2');await page.locator('#preview').click();await page.waitForTimeout(50);
    assert.equal(requests.filter(r=>r.action==='preview').at(-1).input.schedule.expression,'35 14 * * 2');
    await page.locator('#kind').selectOption('once');await page.locator('#at').fill('2026-12-01T09:00');await page.locator('#preview').click();await page.waitForTimeout(50);
    assert.equal(requests.filter(r=>r.action==='preview').at(-1).input.schedule.at,Date.parse('2026-12-01T09:00:00-07:00'));
    draftGate=deferred();await page.locator('#save').click();await page.locator('#source').fill('newer source');draftGate.resolve();draftGate=null;
    await page.getByRole('status').filter({hasText:'newer edits'}).waitFor();assert.equal(await page.locator('#source').inputValue(),'newer source');
    await page.locator('#save').click();await page.locator('#review').waitFor();assert.equal(requests.filter(r=>r.action==='draft').at(-1).input.job_id,'new-job');
    await page.locator('#approved').check();await page.locator('#publish').click();await page.getByRole('status').filter({hasText:'Reviewed revision published'}).waitFor();
    await page.locator('#source').fill('dirty edit');await page.getByRole('button',{name:'Existing'}).click();assert.equal(await page.locator('#source').inputValue(),'dirty edit');
    page.removeAllListeners('dialog');page.on('dialog',dialog=>dialog.accept());revisionGate=deferred();await page.getByRole('button',{name:'Existing'}).click();assert.equal(await page.locator('#source').isDisabled(),true);assert.equal(await page.locator('#kind').isDisabled(),true);revisionGate.resolve();revisionGate=null;await page.waitForFunction(()=>!document.querySelector('#source').disabled);assert.equal(await page.locator('#source').inputValue(),body.source);
    await page.locator('#moreRevisions').click();await page.waitForTimeout(100);assert.equal(requests.filter(r=>r.action==='revisions').at(-1).input.offset,10);
    await page.locator('#moreHistory').click();await page.waitForTimeout(100);assert.equal(requests.filter(r=>r.action==='history').at(-1).input.offset,500);
    await page.locator('#prevHistory').click();await page.waitForTimeout(100);
    await page.locator('#history summary').first().focus();const beforePoll=requests.filter(r=>r.action==='list').length;await page.evaluate(()=>window.cronPoll());await page.waitForTimeout(100);
    assert.ok(requests.filter(r=>r.action==='list').length>beforePoll);
    assert.equal(await page.evaluate(()=>document.activeElement.matches('#history summary')),true);
    await page.getByRole('button',{name:'Mark read'}).focus();notifications=[...notifications,{id:'n2',message:'New notice',created:2,read:false}];await page.evaluate(()=>window.cronPoll());await page.waitForTimeout(100);
    assert.equal(await page.evaluate(()=>document.activeElement.textContent),'Mark read');
    await page.locator('#name').focus();await page.evaluate(()=>window.cronPoll());await page.getByText('New notice').waitFor();
  } finally {if(browser)await browser.close();await new Promise(r=>server.close(r));}
});

test('delayed delete preserves a newer draft', async()=>{
  const pending=deferred(); const b=browser(action=>action==='delete'?pending.promise:undefined); await tick();
  vm.runInContext("state.jobs=[{id:'old',name:'Old',archived:true}];state.selected='old'",b.context);
  b.fire('delete'); b.fire('new'); b.get('source').value='new edits'; b.fire('editor','input');
  pending.resolve(response({})); await tick(); await tick();
  assert.equal(b.get('source').value,'new edits');
  assert.equal(vm.runInContext('state.dirty',b.context),true);
});
test('restore respects unsaved-edit confirmation',async()=>{
  const b=browser(action=>action==='revisions'?response([{id:'r',body:{source:'old'}}]):undefined); await tick();
  vm.runInContext("state.selected='j';state.dirty=true;confirm=()=>false",b.context);
  await vm.runInContext("loadDetails('j')",b.context);
  b.get('source').value='unsaved';
  b.get('revisions').children[0].children.at(-1).listeners.click(); await tick();
  assert.equal(b.get('source').value,'unsaved');
});

test('published summary remains separate from unsaved editor changes', async () => {
  const b = browser(); await tick();
  vm.runInContext(`state.jobs=[{id:'job', name:'Published', current_revision:'r1', published_schedule:{kind:'cron',expression:'0 9 * * *',timezone:'UTC'}, next_at:1000, last_result:'succeeded', unpublished_changes:1, blocked:'Blocked: permission missing'}]; state.selected='job'; renderJobs();`, b.context);
  assert.match(b.get('publishedSummary').textContent, /0 9 \* \* \* \(UTC\)/);
  assert.match(b.get('publishedSummary').textContent, /Last result: succeeded/);
  assert.match(b.get('publishedSummary').textContent, /Unpublished changes: Yes/);
  assert.match(b.get('publishedSummary').textContent, /Blocked: permission missing/);
  b.get('kind').value='manual'; b.fire('editor','input');
  assert.match(b.get('publishedSummary').textContent, /0 9 \* \* \*/);
  assert.equal(b.get('run').disabled, true);
});

test('publish waits for the save refresh and preserves the later validation error', async()=>{
  const pending=deferred();let delay=false;
  const b=browser(action=>action==='list'&&delay?pending.promise:action==='publish'?{ok:false,status:400,json:async()=>({error:'task validation failed: syntax'})}:undefined);
  await tick();delay=true;b.fire('editor','submit');await tick();
  assert.equal(b.get('review').hidden,false);assert.equal(vm.runInContext('state.saving',b.context),true);
  b.get('approved').checked=true;b.fire('approved','change');
  assert.equal(b.get('publish').disabled,true);b.fire('publish');await tick();
  assert.equal(b.requests.some(r=>r.action==='publish'),false);
  pending.resolve(response([]));await tick();await tick();
  assert.equal(vm.runInContext('state.saving',b.context),false);assert.equal(b.get('publish').disabled,false);
  b.fire('publish');await tick();await tick();
  assert.match(b.get('message').textContent,/validation failed/);
  assert.equal(b.get('state').textContent,'Saved draft awaiting review');
  assert.equal(b.get('publish').disabled,false);
});

test('saved review explains activation and permissions before approval', async () => {
  const b=browser(); await tick(); b.fire('editor','submit'); await tick();
  assert.match(b.get('reviewSummary').textContent,/Runs only when you choose Run published revision/);
  assert.match(b.get('reviewSummary').textContent,/Network: none/);
  assert.match(b.get('reviewSummary').textContent,/local inbox notifications/);
  assert.equal(b.get('publish').disabled,true);
  assert.equal(b.get('editorTitle').textContent,'Test');
});
test('unpublished jobs are explicitly inactive and irrelevant actions stay hidden', async () => {
  const b=browser(); await tick();
  assert.equal(b.get('publishedPanel').hidden,true);
  vm.runInContext("state.jobs=[{id:'draft',name:'Draft',current_revision:null}];state.selected='draft';renderJobs()",b.context);
  assert.match(b.get('publishedSummary').textContent,/Not active/);
  assert.equal(b.get('delete').hidden,true);
  assert.equal(b.get('run').disabled,true);
});
test('one-off preview uses the browser timezone and manual preview stays hidden', async () => {
  const at=Date.parse('2026-11-01T08:30:00Z');
  const b=browser(action=>action==='preview'?response({times:[at]}):undefined); await tick();
  assert.equal(b.get('preview').hidden,true);
  b.get('kind').value='once'; b.get('at').value='2026-11-01T01:30'; b.fire('kind','change'); b.fire('preview'); await tick();
  assert.equal(b.get('preview').hidden,false);
  assert.match(b.get('occurrences').textContent,new RegExp(Intl.DateTimeFormat().resolvedOptions().timeZone));
});


test('activation review handles archived and already admitted one-offs exactly', () => {
  const b=browser();
  const at=Date.now()-1000;
  b.context.onceAt=at;
  assert.match(vm.runInContext("activationSummary({schedule:{kind:'once',at:onceAt}},{archived:true}).activation",b.context),/Publishing keeps it archived/);
  assert.match(vm.runInContext("activationSummary({schedule:{kind:'once',at:onceAt}},{once_claimed:true,published_schedule:{kind:'once',at:onceAt}}).next",b.context),/none.*already admitted/);
  assert.match(vm.runInContext("activationSummary({schedule:{kind:'once',at:onceAt}},{once_claimed:true,published_schedule:{kind:'once',at:onceAt-1}}).activation",b.context),/eligible immediately/);
});
test('archive, one-off admission and published revision changes invalidate open review', async () => {
  for (const changed of [{archived:true},{once_claimed:true},{current_revision:'other'}]) {
    let jobs=[{id:'j1',name:'Test',current_revision:null}];
    const b=browser(action=>action==='list'?response(jobs):undefined); await tick();
    vm.runInContext("state.selected='j1'",b.context); b.fire('editor','submit'); await tick(); await tick();
    assert.equal(b.get('review').hidden,false);
    jobs=[{...jobs[0],...changed}]; await vm.runInContext('refresh()',b.context);
    assert.equal(b.get('review').hidden,true);
    assert.equal(b.get('publish').disabled,true);
    assert.match(b.get('message').textContent,/activation state changed/);
  }
});


test('new draft history gives next steps and archive refresh preserves editing state', async () => {
  let jobs=[];
  const b=browser(action=>action==='list'?response(jobs):undefined); await tick();
  assert.match(b.get('history').children[0].textContent,/Publish a reviewed revision/);
  assert.match(b.get('revisions').children[0].textContent,/Save a draft/);
  vm.runInContext("state.selected='j1'",b.context); b.fire('editor','input');
  jobs=[{id:'j1',name:'Test',archived:true,current_revision:'r'}]; await vm.runInContext('refresh()',b.context);
  assert.equal(b.get('state').textContent,'Archived · Draft edits');
  jobs=[{...jobs[0],archived:false}]; await vm.runInContext('refresh()',b.context);
  assert.equal(b.get('state').textContent,'Draft edits');
  b.fire('new');
  assert.match(b.get('history').children[0].textContent,/Publish a reviewed revision/);
  assert.match(b.get('revisions').children[0].textContent,/Save a draft/);
});
