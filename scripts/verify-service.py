#!/usr/bin/env python3
"""Exercise a uniquely named native user service; remove its registration on exit."""
import re, json, os, pathlib, plistlib, signal, socket, subprocess, sys, tempfile, time, urllib.request
binary=str(pathlib.Path(os.environ.get('PARACO_BIN','target/debug/paraco')).resolve())
def wait(check):
    end=time.monotonic()+40
    while time.monotonic()<end:
        try:
            result=check()
            if result: return result
        except (OSError,ValueError,subprocess.CalledProcessError): pass
        time.sleep(.2)
    raise RuntimeError('native service verification timed out')
with tempfile.TemporaryDirectory(prefix='pc-service-',dir='/tmp') as directory:
    root=pathlib.Path(directory); state=root/'state with spaces'; state.mkdir(); app=root/'app'; app.mkdir()
    (app/'paraco.json').write_text('{"name":"service-app","entrypoint":"main.ts","capabilities":[]}')
    (app/'main.ts').write_text('export default {fetch(){return new Response("service-ok")}}')
    config=root/'server.json'; config.write_text(json.dumps({'apps':[{'path':str(app)}]}))
    with socket.socket() as sock: sock.bind(('127.0.0.1',0)); port=sock.getsockname()[1]
    label=f'dev.paraco.verify.{os.getpid()}'; mac=sys.platform=='darwin'
    rendered=subprocess.check_output([binary,'service','render','--platform','macos' if mac else 'linux','--entry',binary,'--config',str(config),'--state',str(state)])
    def run(*args): return subprocess.run(args,check=True,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
    if mac:
        definition=root/'service.plist'; content=plistlib.loads(rendered); content['Label']=label
        content['ProgramArguments']+=['--port',str(port)]
        content['StandardOutPath']=str(root/'stdout'); content['StandardErrorPath']=str(root/'stderr')
        definition.write_bytes(plistlib.dumps(content)); target=f'gui/{os.getuid()}/{label}'
        def setup(): run('launchctl','bootstrap',f'gui/{os.getuid()}',str(definition))
        def remove(): run('launchctl','bootout',target)
        def crash(): run('launchctl','kill','SIGKILL',target)
    else:
        definition=root/(label+'.service'); definition.write_text(rendered.decode().replace(' serve --config',' serve --port '+str(port)+' --config'))
        target=label+'.service'
        def setup(): run('systemctl','--user','link',str(definition)); run('systemctl','--user','start',target)
        def remove(): run('systemctl','--user','disable','--now',target)
        def crash(): run('systemctl','--user','kill','--kill-who=main','--signal=SIGKILL',target)
    def cli(*args): return subprocess.check_output([binary,*args,'--port',str(port)],text=True,stderr=subprocess.DEVNULL)
    active=False
    try:
        # Even partial setup can leave a linked unit behind.
        active=True; setup()
        wait(lambda: json.loads(cli('status'))[0]['state']=='running')
        first=json.loads(cli('status'))[0]
        first_url=cli('open','--print')
        cli('stop','service-app'); wait(lambda: json.loads(cli('status'))[0]['state']=='stopped')
        crash(); wait(lambda: cli('open','--print')!=first_url)
        after=json.loads(cli('status'))[0]; assert after['desired']=='stopped' and after['deployment_id']==first['deployment_id']
        cli('start','service-app'); wait(lambda: json.loads(cli('status'))[0]['state']=='running')
        request=urllib.request.Request(f'http://127.0.0.1:{port}/',headers={'Host':f'service-app.localhost:{port}'})
        with urllib.request.urlopen(request) as response: assert response.read()==b'service-ok'
        remove(); active=False
        wait(lambda: subprocess.run([binary,'status','--port',str(port)],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL).returncode!=0)
        print(json.dumps({'manager':'launchd' if mac else 'systemd','startup':'pass','unexpected_death_restart':'pass','stopped_intent_identity':'preserved','authenticated_entry':'rotated','shutdown':'pass','machine_reboot':'not performed'}))
    except Exception:
        for name in ('stdout','stderr'):
            if (root/name).exists(): print(re.sub(r'(management dashboard )\S+', r'\1[redacted]', (root/name).read_text()),file=sys.stderr)
        raise
    finally:
        if active: remove()
