#!/usr/bin/env python3
"""Install all approved showcase packages and probe their hosted runtime."""
import argparse
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import time
import urllib.request

ROOT = Path(__file__).resolve().parent.parent
BINARY = ROOT / 'target/debug/paraco'
APPS = ('ai-memory-curator', 'cron-time-machine', 'github-pulse', 'json-yaml-workbench',
        'local-uptime-sentry', 'prompt-regression-lab', 'scientific-lab-notebook', 'system-vitals')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--packages', type=Path, help='Local release asset directory; omit to download publicly')
    args = parser.parse_args()
    subprocess.run(['cargo', 'build', '--locked'], cwd=ROOT, check=True)
    catalog = json.loads(subprocess.check_output([BINARY, 'marketplace', 'list'], text=True))
    approved = {app['name']: app for app in catalog['apps']}
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    with tempfile.TemporaryDirectory(prefix='paraco-showcase-marketplace-') as temporary:
        root = Path(temporary)
        entries = []
        for name in APPS:
            directory = root / name
            cmd = [BINARY, 'marketplace', 'install', name, '--version', '0.1.0', '--output', directory]
            if args.packages:
                cmd += ['--package', args.packages.resolve() / (name + '.paraco.json')]
            subprocess.run(cmd, check=True)
            manifest = json.loads((directory / 'paraco.json').read_text())
            assert manifest['name'] == name
            assert sorted(manifest['capabilities']) == sorted(approved[name]['capabilities'])
            assert json.loads((directory / 'paraco-install.json').read_text()) == approved[name]
            assert (directory / 'LICENSE').is_file()
            entry = {'path': str(directory)}
            if 'tasks' in manifest['capabilities']:
                entry.update(taskAuthoring=True, taskNetwork=[])
            entries.append(entry)
        config = root / 'server.json'
        config.write_text(json.dumps({'apps': entries}))
        with socket.socket() as listener:
            listener.bind(('127.0.0.1', 0))
            port = listener.getsockname()[1]
        env = dict(os.environ, PATH=str(ROOT / 'node_modules/.bin') + os.pathsep + os.environ.get('PATH', ''))
        with (root / 'host.log').open('w+') as log:
            process = subprocess.Popen([BINARY, '--log-dir', root / 'logs', 'serve', '--config', config, '--port', str(port)], env=env, stdout=log, stderr=log)
            try:
                for name in APPS:
                    def request(route='/', body=None):
                        origin = f'http://{name}.localhost:{port}'
                        headers = {'Host': f'{name}.localhost:{port}', 'Origin': origin}
                        if body is not None:
                            headers['Content-Type'] = 'application/json'
                        req = urllib.request.Request(f'http://127.0.0.1:{port}{route}', headers=headers,
                                                     data=None if body is None else json.dumps(body).encode())
                        return opener.open(req, timeout=10).read()
                    for _ in range(200):
                        try:
                            page = request()
                            assert b'<main id="app">' in page
                            break
                        except OSError:
                            if process.poll() is not None:
                                log.seek(0)
                                raise AssertionError(log.read())
                            time.sleep(0.1)
                    else:
                        log.seek(0)
                        raise AssertionError(f'{name} did not start: {log.read()}')
                    assert request('/app.js')
                    assert request('/ui.js')
                    assert request('/style.css')
                    if name == 'json-yaml-workbench':
                        assert request('/yaml.js')
                    elif name == 'system-vitals':
                        if os.uname().sysname == 'Linux':
                            assert json.loads(request('/api/snapshot', {}))['memory']['total'] > 0
                    elif name == 'cron-time-machine':
                        result = json.loads(request('/api/preview', {'expression': '0 9 * * 1-5', 'timezone': 'UTC'}))
                        assert len(result['times']) == 5
                    elif name in ('scientific-lab-notebook', 'ai-memory-curator', 'prompt-regression-lab'):
                        assert json.loads(request('/api/load', {}))['version'] is None
                    else:
                        action = 'repositories' if name == 'github-pulse' else 'monitors'
                        assert json.loads(request('/api/' + action, {}))['jobs'] == []
                    print(f'Verified installed package: {name}', flush=True)
            finally:
                process.terminate()
                try:
                    process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=5)
    print('All eight catalog packages installed and served successfully.')


if __name__ == '__main__':
    main()
