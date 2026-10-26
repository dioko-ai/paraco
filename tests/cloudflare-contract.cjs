// Runs the Cloudflare entry point locally; this is not a live deployment test.
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const os = require('node:os');
const net = require('node:net');
const {spawn, execFileSync} = require('node:child_process');
(async () => {
  const root = path.resolve(__dirname, '..');
  const dir = await fs.mkdtemp(path.join(os.tmpdir(), 'paraco-cloudflare-'));
  let child;
  try {
    execFileSync(path.join(root,'node_modules/.bin/esbuild'), [path.join(root,'examples/cloudflare/worker.ts'),'--bundle','--format=esm','--platform=browser',`--outfile=${dir}/worker.js`], {stdio:'pipe'});
    const listener=net.createServer(); await new Promise(resolve=>listener.listen(0,'127.0.0.1',resolve));
    const port=listener.address().port; await new Promise(resolve=>listener.close(resolve));
    await fs.writeFile(path.join(dir,'worker.capnp'), `using W = import "/workerd/workerd.capnp";
const config :W.Config = (services=[(name="app",worker=(compatibilityDate="2026-08-03",compatibilityFlags=["disallow_importable_env"],modules=[(name="worker.js",esModule=embed "worker.js")],globalOutbound="blocked")),(name="blocked",network=(allow=[]))],sockets=[(name="http",address="127.0.0.1:${port}",http=(),service="app")]);`);
    child=spawn(path.join(root,'node_modules/.bin/workerd'),['serve',path.join(dir,'worker.capnp')],{stdio:['ignore','pipe','pipe']});
    let output='';child.stdout.on('data',b=>output+=b);child.stderr.on('data',b=>output+=b);
    let response;
    for(let i=0;i<100;i++) {
      assert.equal(child.exitCode,null,output);
      try {response=await fetch(`http://127.0.0.1:${port}/`);break;} catch {await new Promise(resolve=>setTimeout(resolve,50));}
    }
    assert.ok(response,output);assert.equal(response.status,200);assert.equal(await response.text(),'Hello from Paraco');
    console.log('Cloudflare entry point: same core handler passes local workerd compatibility check; live deployment remains separate.');
  } finally {
    if(child && child.exitCode===null){child.kill('SIGKILL');await new Promise(resolve=>child.once('exit',resolve));}
    await fs.rm(dir,{recursive:true,force:true});
  }
})().catch(error=>{console.error(error);process.exitCode=1;});
