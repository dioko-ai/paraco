// Exercises public HTTP/task APIs against real supervised workerd processes.
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const os = require('node:os');
const path = require('node:path');
const net = require('node:net');
const http = require('node:http');
const {spawn, execFileSync} = require('node:child_process');
const root = path.resolve(__dirname, '..');
const binary = process.env.PARACO_BIN || path.join(root,'target/debug/paraco');
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
async function until(fn, label, ms=20000) {
  const deadline = Date.now()+ms;
  let last;
  while(Date.now()<deadline) { try { const result = await fn(); if(result) return result; } catch(e) { last=e; } await sleep(50); }
  throw new Error(`${label}: ${last?.message || 'timed out'}`);
}
async function freePort() { const s=net.createServer(); await new Promise(r=>s.listen(0,'127.0.0.1',r)); const port=s.address().port; await new Promise(r=>s.close(r)); return port; }
(async()=> {
 const tmp=await fs.mkdtemp(path.join(os.tmpdir(),'paraco-cron-'));
 const config=path.join(tmp,'server.json'), logs=path.join(tmp,'logs');
 const port=await freePort(), origin=`http://cron.localhost:${port}`;
 await fs.writeFile(config,JSON.stringify({apps:[{path:path.join(root,'examples/cron'),taskAuthoring:true,taskNetwork:[]}]}));
 let child, output='';
 function call(route, body) {
   return new Promise((resolve,reject)=> {
     const req=http.request({host:'127.0.0.1',port,path:route,method:body===undefined?'GET':'POST',headers:{Host:`cron.localhost:${port}`,Origin:origin,...(body===undefined?{}:{'Content-Type':'application/json'})}},res=> {
       let text='';res.setEncoding('utf8');res.on('data',s=>text+=s);res.on('end',()=>resolve({status:res.statusCode,ok:res.statusCode>=200&&res.statusCode<300,text}));
     });req.setTimeout(70000,()=>req.destroy(new Error('request timed out')));req.on('error',reject);req.end(body===undefined?undefined:JSON.stringify(body));
   });
 }
 async function start() {
   child=spawn(binary,['--data-dir',tmp,'--log-dir',logs,'cron','--port',String(port)],{cwd:tmp,env:{...process.env,...(process.env.PARACO_TEST_EMPTY_PATH ? {PATH:"/nonexistent"} : {}),TMPDIR:tmp},stdio:['ignore','pipe','pipe']});
   child.stdout.on('data',b=>output+=b); child.stderr.on('data',b=>output+=b);
   await until(async()=>{const r=await call('/');return r.ok&&r.text.includes('editor');},'cron readiness');
 }
 async function stop(signal='SIGINT') { const current=child; child=null;if(!current)return; current.kill(signal); await until(()=>current.exitCode!==null||current.signalCode!==null,'host shutdown'); }
 async function api(action,body={},ok=true) {
   const r=await call(`/api/${action}`,body);
   const text=r.text; let value; try { value=JSON.parse(text); } catch { throw new Error(`${action} returned ${r.status}: ${text}\n${output}`); } if(ok) assert.equal(r.ok,true,`${action}: ${JSON.stringify(value)}\n${output}`); else assert.equal(r.ok,false,action); return value;
 }
 async function draft(source,overrides={}) {return api('draft',{name:'Runtime check',source,schedule:{kind:'manual'},timeout_ms:1000,notify_success:true,notify_failure:true,...overrides});}
 async function publish(d,expected=null) {return api('publish',{revision_id:d.revision_id,expected_current_revision:expected});}
 async function outcome(job,id,status) {return until(async()=>{const runs=await api('history',{job_id:job});const r=runs.find(r=>r.id===id);if(r&&!['pending','running'].includes(r.status)){assert.equal(r.status,status,JSON.stringify(r));return r;}},status);}
 try {
   await start();
   const d=await draft('export default async (ctx: {log: (s: string) => void; notify: (s: string) => void}) => { ctx.log("task success"); ctx.notify("hello inbox"); };');
   assert.equal((await api('history',{job_id:d.job_id})).length,0,'draft cannot run');
   await api('run',{job_id:d.job_id},false);
   await publish(d);
   const {run_id}=await api('run',{job_id:d.job_id});
   await outcome(d.job_id,run_id,'success');
   assert.ok((await api('notifications')).some(n=>n.message==='hello inbox'));
   const replacement=await draft('export default async () => { throw new Error("expected failure"); };',{job_id:d.job_id});
   await api('publish',{revision_id:replacement.revision_id,expected_current_revision:null},false);
   await publish(replacement,d.revision_id);
   const error=await api('run',{job_id:d.job_id});await outcome(d.job_id,error.run_id,'error');
   const infinite=await draft('export default async () => { while(true) {} };',{job_id:d.job_id,timeout_ms:300});
   await publish(infinite,replacement.revision_id);
   const timeout=await api('run',{job_id:d.job_id});await outcome(d.job_id,timeout.run_id,'timeout');
   const long=await draft('export default async () => { console.log("before-interruption"); await new Promise(resolve => setTimeout(resolve, 60000)); };',{job_id:d.job_id,timeout_ms:60000});
   await publish(long,infinite.revision_id);
   const active=await api('run',{job_id:d.job_id});
   await api('run',{job_id:d.job_id},false);
   await api('cancel',{run_id:active.run_id});await outcome(d.job_id,active.run_id,'cancelled');
   await api('archive',{job_id:d.job_id});await api('run',{job_id:d.job_id},false);
   await stop();await start();
   assert.equal((await api('list')).find(j=>j.id===d.job_id).archived,true);
   assert.ok((await api('history',{job_id:d.job_id})).some(r=>r.id===run_id));
   await api('unarchive',{job_id:d.job_id});
   const interrupted=await api('run',{job_id:d.job_id});
   await until(async()=> (await api('logs',{run_id:interrupted.run_id})).logs.includes('before-interruption'),'live logs');
   await sleep(400);await stop('SIGKILL');await start();
   await outcome(d.job_id,interrupted.run_id,'interrupted');
   assert.ok((await api('logs',{run_id:interrupted.run_id})).logs.includes('before-interruption'),'flushed log prefix survives SIGKILL');
   await api('draft',{name:'Denied access',source:'export default async () => {};',schedule:{kind:'manual'},network:['example.com:443']},false);
   const once=await draft('export default async () => {};',{schedule:{kind:'once',at:Date.now()+500},name:'One off'});
   await publish(once);
   await until(async()=> (await api('history',{job_id:once.job_id})).some(r=>r.status==='success'),'one off');
   await api('archive',{job_id:once.job_id});await api('unarchive',{job_id:once.job_id});await sleep(600);
   assert.equal((await api('history',{job_id:once.job_id})).length,1,'unarchiving completed one-off must not replay');
   await stop();
   const stateParent=path.join(tmp,'state');
   const [stateName]=await fs.readdir(stateParent);const stateRoot=path.join(stateParent,stateName);
   const backup=path.join(tmp,'backup.sqlite3');
   execFileSync(binary,['backup','--state',stateRoot,'--output',backup]);
   await fs.rm(path.join(stateRoot,'tasks'),{recursive:true,force:true});
   await fs.copyFile(backup,path.join(stateRoot,'state.sqlite3'));
   await start();
   const restored=await api('run',{job_id:once.job_id});await outcome(once.job_id,restored.run_id,'success');
   await api('delete',{job_id:once.job_id},false);
   await api('archive',{job_id:once.job_id});
   assert.equal((await api('delete',{job_id:once.job_id})).ok,true);
   assert.ok(!(await api('list')).some(job=>job.id===once.job_id));
   await api('history',{job_id:once.job_id},false);
   console.log('cron runtime: deletion, publication, scope, success/error/timeout/cancel, archive, restart, SIGKILL, notifications, one-off and database-only backup restoration checks passed');
 } finally {await stop();await fs.rm(tmp,{recursive:true,force:true});}
})().catch(e=>{console.error(e);process.exitCode=1;});
