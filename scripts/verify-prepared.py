#!/usr/bin/env python3
"""Prepare a pinned HTTPS dependency, relocate it, and prove failed updates preserve it.
This checks cached-only execution with an empty PATH. OS network isolation is a
separate clean-container/clean-machine gate. Requires a built Paraco and Deno.
"""
import hashlib,json,os,pathlib,shutil,signal,socket,subprocess,tempfile,time,urllib.request
binary=str(pathlib.Path(os.environ.get('PARACO_BIN','target/debug/paraco')).resolve())
deno=str(pathlib.Path(shutil.which('deno')).resolve())
with tempfile.TemporaryDirectory(prefix='pc-offline-',dir='/tmp') as directory:
    root=pathlib.Path(directory); source=root/'source'; shutil.copytree('examples/offline-dependency',source)
    artifact=root/'prepared'
    subprocess.run([binary,'prepare',str(source),'--output',str(artifact),'--deno',deno],check=True)
    def digest(): return hashlib.sha256((artifact/'paraco-artifact.json').read_bytes()).hexdigest()
    before=digest()
    failed=subprocess.run([binary,'prepare',str(source),'--output',str(artifact),'--deno',deno],capture_output=True)
    assert failed.returncode!=0 and digest()==before
    (source/'main.ts').write_text("import 'unsupported:failed-update'; export default {}")
    failed=subprocess.run([binary,'prepare',str(source),'--output',str(root/'failed-update'),'--deno',deno],capture_output=True)
    assert failed.returncode!=0 and not (root/'failed-update').exists() and digest()==before
    relocated=root/'relocated'; artifact.rename(relocated); shutil.rmtree(source)
    with socket.socket() as sock: sock.bind(('127.0.0.1',0)); port=sock.getsockname()[1]
    # Run twice to ensure execution does not invalidate the artifact's digests.
    for attempt in range(2):
        with open(root/'run.log','w') as log:
            child=subprocess.Popen([binary,'--log-dir',str(root/'logs'),'run-prepared',str(relocated),'--port',str(port)],env={'PATH':'/nonexistent','TMPDIR':'/tmp'},cwd='/',stdout=log,stderr=log)
            try:
                # Deno can serve before the host consumes its readiness message.
                # Wait for host readiness before testing a normal graceful stop.
                for _ in range(200):
                    if 'listening on http://' in (root/'run.log').read_text(): break
                    if child.poll() is not None: raise RuntimeError((root/'run.log').read_text())
                    time.sleep(.1)
                else: raise RuntimeError('host readiness timed out')
                for _ in range(200):
                    try:
                        with urllib.request.urlopen(f'http://127.0.0.1:{port}',timeout=1) as response: assert response.read()==b'dependency-ok'; break
                    except OSError: time.sleep(.1)
                else: raise RuntimeError((root/'run.log').read_text())
            finally:
                if child.poll() is None: child.send_signal(signal.SIGINT)
                child.wait(timeout=10)
        assert child.returncode==0, f"launch {attempt}: exit {child.returncode}: {(root / 'run.log').read_text()}"
    print(json.dumps({'relocation':'pass','dependency':'pinned HTTPS std/path basename','PATH':'/nonexistent','cwd':'/','launches':2,'failed_preparation':'installed revision preserved','shutdown':'pass','network_isolation':'separate clean-machine gate'}))
