#!/usr/bin/env python3
"""Native storage/restart/open and 1/10/50-app verification. Requires a built binary."""
import re, json, os, pathlib, signal, socket, subprocess, sys, tempfile, time, urllib.request
binary = str(pathlib.Path(os.environ.get('PARACO_BIN', 'target/debug/paraco')).resolve())
def wait(check, seconds=40):
    end = time.monotonic()+seconds
    while time.monotonic()<end:
        try:
            value = check()
            if value: return value
        except (OSError, ValueError, subprocess.CalledProcessError): pass
        time.sleep(.1)
    raise RuntimeError('verification timed out')
def port():
    with socket.socket() as sock:
        sock.bind(('127.0.0.1',0)); return sock.getsockname()[1]
def get(p, name, method='GET', body=None):
    request = urllib.request.Request(f'http://127.0.0.1:{p}/', data=body, method=method, headers={'Host':f'{name}.localhost:{p}', 'Content-Type':'application/json'})
    with urllib.request.urlopen(request,timeout=3) as response: return json.load(response)
with tempfile.TemporaryDirectory(prefix='pc-', dir='/tmp') as directory:
    root = pathlib.Path(directory); env = dict(os.environ, TMPDIR=directory, PARACO_LOG_DIR=str(root/'logs'))
    configs=[]
    for index in range(50):
        app=root/f'app{index}'; app.mkdir()
        (app/'paraco.json').write_text(json.dumps({'name':f'app{index}','entrypoint':'main.ts','capabilities':['storage']}))
        (app/'main.ts').write_text('''export default {async fetch(r,c) { if(new URL(r.url).pathname==='/noisy') { for(let i=0;i<2048;i++) console.log('noise'.repeat(205)); } if(r.method==='PUT') { const v=await r.json(); await c.data.set('draft',v); await c.config.set('timezone','UTC'); } return Response.json({draft:await c.data.get('draft'),timezone:await c.config.get('timezone')}); }};''')
    for count in (1,10,50):
        config=root/f'{count}.json'; config.write_text(json.dumps({'apps':[{'path':f'app{i}'} for i in range(count)]})); configs.append(str(config))
    if not os.environ.get('PARACO_SKIP_WORKLOAD'):
        subprocess.run(['node','scripts/probe-hosted-workload.cjs',*configs],env=dict(env,PARACO_BIN=binary,PARACO_NOISY_APP="app0"),check=True)
    p=port(); output=open(root/'output','w'); child=None
    def command(*args): return subprocess.check_output([binary,*args,'--port',str(p)],env=env,text=True,stderr=subprocess.DEVNULL)
    def start():
        global child
        child=subprocess.Popen([binary,'serve','--config',configs[1],'--port',str(p)],env=env,stdout=output,stderr=output)
        wait(lambda: len(json.loads(command('status')))==10)
    def stop(sig=signal.SIGINT):
        child.send_signal(sig); child.wait(timeout=15)
    try:
        start(); wait(lambda: all(a['state']=='running' for a in json.loads(command('status'))))
        first=json.loads(command('status'))
        get(p,'app0','PUT',b'{"source":"return 42"}')
        assert get(p,'app1')['draft'] is None
        url=command('open','--print').strip(); assert url.startswith('http://')
        command('stop','app0'); wait(lambda: json.loads(command('status','app0'))[0]['state']=='stopped')
        stop(signal.SIGKILL); start()
        restored=json.loads(command('status','app0'))[0]
        assert restored['desired']=='stopped' and restored['deployment_id']==first[0]['deployment_id']
        assert command('open','--print').strip()!=url
        command('start','app0'); wait(lambda: json.loads(command('status','app0'))[0]['state']=='running')
        assert get(p,'app0')=={'draft':{'source':'return 42'},'timezone':'UTC'}
        stop()
        print(json.dumps({'storage':'persisted after SIGKILL and restart','isolation':'app1 cannot read app0 draft','desired_state':'stopped intent and identity restored','open':'private URL rotates after restart'}))
    except Exception:
        output.flush()
        print(re.sub(r'(management dashboard )\S+', r'\1[redacted]', (root/'output').read_text()),file=sys.stderr)
        try: print(command('status'),file=sys.stderr)
        except Exception: pass
        raise
    finally:
        if child and child.poll() is None: stop()
        output.close()
