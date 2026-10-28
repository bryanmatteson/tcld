#!/usr/bin/env python3
"""Verify the packaged macOS updater against disposable loopback HTTPS fixtures."""
import argparse
import base64
import functools
import hashlib
import http.server
import json
import os
from pathlib import Path
import shutil
import ssl
import subprocess
import tempfile
import threading


def verify(app, archive, signature, output):
    output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='updater-test-', dir=output) as temporary:
        root = Path(temporary)
        server_root = root / 'server'
        server_root.mkdir()
        client = root / 'installed' / app.name
        client.parent.mkdir()
        subprocess.run(['ditto', str(app), str(client)], check=True)
        marker = client / 'Contents/Resources/updater-before.txt'
        marker.write_text('This must disappear when the real package replaces the app.\n')
        details = subprocess.run(['codesign', '-dvv', str(app)], capture_output=True, text=True, check=True).stderr
        identity = next(line.removeprefix('Authority=') for line in details.splitlines()
                        if line.startswith('Authority=Developer ID Application:'))
        subprocess.run(['codesign', '--force', '--options', 'runtime', '--timestamp', '--sign',
                        identity, str(client)], check=True)
        key, certificate = root / 'test.key', root / 'test.crt'
        subprocess.run(['openssl', 'req', '-x509', '-newkey', 'rsa:2048', '-nodes',
                        '-keyout', str(key), '-out', str(certificate), '-days', '1',
                        '-subj', '/CN=localhost'], check=True, capture_output=True)
        key.chmod(0o600)
        class Handler(http.server.SimpleHTTPRequestHandler):
            def log_message(self, *_args):
                pass
        server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), functools.partial(Handler, directory=server_root))
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        context.load_cert_chain(certificate, key)
        server.socket = context.wrap_socket(server.socket, server_side=True)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        base = f'https://127.0.0.1:{server.server_port}'
        payload = server_root / 'update.app.tar.gz'
        shutil.copy2(archive, payload)
        version = json.loads((Path(__file__).resolve().parents[2] / 'apps/tcld-tauri/tauri.conf.json').read_text())['version']
        architecture = subprocess.check_output(['lipo', '-archs', str(app / 'Contents/MacOS/tcld-tauri')], text=True).strip()
        target = 'darwin-aarch64' if architecture == 'arm64' else 'darwin-x86_64'
        manifest = {'version': version, 'platforms': {target: {
            'url': base + '/' + payload.name, 'signature': signature.read_text().strip()}}}
        manifest_path = server_root / 'latest.json'
        manifest_path.write_text(json.dumps(manifest))
        environment = os.environ.copy()
        environment.update(HOME=str(root / 'home'), TCLD_UPDATER_PROBE_ENDPOINT=base + '/latest.json')
        Path(environment['HOME']).mkdir()
        binary = client / 'Contents/MacOS/tcld-tauri'
        original = hashlib.sha256(binary.read_bytes()).hexdigest()
        results = []

        def probe(action, success, expected):
            result = subprocess.run([str(binary), '--distribution-updater-probe'],
                                    env=environment | {'TCLD_UPDATER_PROBE_ACTION': action},
                                    capture_output=True, text=True, timeout=60)
            text = result.stdout + result.stderr
            (output / f'probe-{len(results)}-{action}.log').write_text(text)
            # macOS's event loop can return zero after AppHandle.exit(1).
            # Require the probe's explicit result as well as a clean launch.
            assert result.returncode in ((0,) if success else (0, 1)), text
            assert ('TCLD updater probe failed:' not in text) == success, text
            assert expected in text, text
            results.append({'action': action, 'success_expected': success, 'result': expected})
            return text

        try:
            probe('check', True, 'up to date')
            manifest['version'] = '9999.0.0'
            manifest_path.write_text(json.dumps(manifest))
            probe('check', True, 'update available')
            assert marker.exists() and hashlib.sha256(binary.read_bytes()).hexdigest() == original
            manifest['version'] = version
            manifest_path.write_text(json.dumps(manifest))
            payload.write_bytes(archive.read_bytes() + b'corruption')
            probe('install', False, 'signature verification failed')
            assert marker.exists() and hashlib.sha256(binary.read_bytes()).hexdigest() == original
            shutil.copy2(archive, payload)
            valid_signature = manifest['platforms'][target]['signature']
            decoded = base64.b64decode(valid_signature).decode()
            lines = decoded.splitlines()
            lines[-1] = ('A' if lines[-1][0] != 'A' else 'B') + lines[-1][1:]
            manifest['platforms'][target]['signature'] = base64.b64encode(('\n'.join(lines) + '\n').encode()).decode()
            manifest_path.write_text(json.dumps(manifest))
            probe('install', False, 'signature verification failed')
            assert marker.exists() and hashlib.sha256(binary.read_bytes()).hexdigest() == original
            manifest['platforms'][target]['signature'] = valid_signature
            manifest_path.write_text(json.dumps(manifest))
            probe('download', True, 'download signature verified')
            assert marker.exists()
            probe('install', True, 'installed')
            assert not marker.exists(), 'The old application was not replaced'
            assert hashlib.sha256(binary.read_bytes()).hexdigest() == hashlib.sha256((app / 'Contents/MacOS/tcld-tauri').read_bytes()).hexdigest()
            subprocess.run(['codesign', '--verify', '--deep', '--strict', str(client)], check=True)
            subprocess.run(['spctl', '--assess', '--type', 'execute', str(client)], check=True)
            subprocess.run(['xcrun', 'stapler', 'validate', str(client)], check=True)
            result = subprocess.run([str(binary), '--distribution-smoke-test'], env=environment,
                                    check=True, capture_output=True, text=True, timeout=30)
            assert 'TCLD distribution smoke passed' in result.stdout
            results.append({'relaunch': 'packaged runtime passed'})
            (output / 'updater-verification.json').write_text(json.dumps(results, indent=2) + '\n')
            print('Updater checks passed: version selection, no implicit installation, corrupt download and invalid signature rejection, signed download, application replacement, Gatekeeper, and relaunch.')
        finally:
            server.shutdown()
            server.server_close()
            thread.join()


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--app', type=Path, required=True)
    parser.add_argument('--archive', type=Path, required=True)
    parser.add_argument('--signature', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    verify(args.app.resolve(), args.archive.resolve(), args.signature.resolve(), args.output.expanduser().resolve())
