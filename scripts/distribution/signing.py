#!/usr/bin/env python3
"""Create and load the local updater key without printing private material."""
import argparse
import json
import os
from pathlib import Path
import secrets
import shlex
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
SERVICE = 'dev.matteson.tcld.updater'
ACCOUNT = 'release-signing-key'


def credentials():
    result = subprocess.run(['security', 'find-generic-password', '-s', SERVICE,
                             '-a', ACCOUNT, '-w'], capture_output=True, text=True)
    if result.returncode:
        raise ValueError('Updater key is unavailable in Keychain; run python3 scripts/distribution/signing.py setup')
    return json.loads(result.stdout)


def configure(environment):
    """Fill only missing values; private values stay in process memory/env."""
    config = json.loads((ROOT / 'apps/tcld-tauri/tauri.conf.json').read_text())
    public = config.get('plugins', {}).get('updater', {}).get('pubkey', '')
    environment.setdefault('TCLD_UPDATER_PUBLIC_KEY', public)
    local = Path.home() / '.config/tcld/signing.json'
    if sys.platform == 'darwin' and local.exists():
        settings = json.loads(local.read_text())
        for key in ['APPLE_SIGNING_IDENTITY', 'TCLD_NOTARY_PROFILE']:
            if settings.get(key):
                environment.setdefault(key, settings[key])
    if not environment.get('TAURI_SIGNING_PRIVATE_KEY') and sys.platform == 'darwin':
        saved = credentials()
        if saved['public_key'] != environment['TCLD_UPDATER_PUBLIC_KEY']:
            raise ValueError('Keychain updater key does not match the configured public key')
        environment['TAURI_SIGNING_PRIVATE_KEY'] = saved['private_key']
        environment.setdefault('TAURI_SIGNING_PRIVATE_KEY_PASSWORD', saved['password'])


def setup():
    if sys.platform != 'darwin':
        raise ValueError('Keychain setup runs on macOS')
    directory = Path.home() / '.config/tcld'
    directory.mkdir(parents=True, exist_ok=True, mode=0o700)
    path = directory / 'updater.key'
    if path.exists():
        raise ValueError(f'Refusing to replace the existing key: {path}')
    existing = subprocess.run(['security', 'find-generic-password', '-s', SERVICE,
                               '-a', ACCOUNT], capture_output=True)
    if existing.returncode == 0:
        raise ValueError('An updater key already exists in Keychain; refusing to rotate it')
    password = secrets.token_urlsafe(48)
    previous = os.umask(0o077)
    try:
        result = subprocess.run(['cargo', 'tauri', 'signer', 'generate', '--ci',
                                 '--password', password, '--write-keys', str(path)],
                                capture_output=True, text=True)
        if result.returncode:
            raise ValueError('Tauri key generation failed; no private output was logged')
        private = path.read_text().strip()
        public = path.with_suffix('.key.pub').read_text().strip()
        value = json.dumps({'private_key': private, 'public_key': public, 'password': password})
        command = ' '.join(shlex.quote(x) for x in [
            'add-generic-password', '-s', SERVICE, '-a', ACCOUNT, '-w', value]) + '\n'
        # Interactive command input keeps private material out of process argv.
        stored = subprocess.run(['security', '-i'], input=command,
                                capture_output=True, text=True)
        if stored.returncode or credentials()['public_key'] != public:
            raise ValueError('Could not store and verify the updater credential in Keychain')
        config_path = ROOT / 'apps/tcld-tauri/tauri.conf.json'
        config = json.loads(config_path.read_text())
        config.setdefault('plugins', {})['updater'] = {
            'pubkey': public,
            'endpoints': ['https://github.com/bryanmatteson/tcld/releases/latest/download/latest.json'],
        }
        config_path.write_text(json.dumps(config, indent=2) + '\n')
        print(f'Created updater key in Keychain ({SERVICE}); public key added to tauri.conf.json.')
        print(f'Encrypted private-key backup: {path}. Its password is stored in Keychain.')
    finally:
        os.umask(previous)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=['setup', 'status', 'configure'])
    parser.add_argument('--identity')
    parser.add_argument('--notary-profile')
    args = parser.parse_args()
    try:
        if args.command == 'setup':
            setup()
        elif args.command == 'configure':
            if not args.identity or not args.notary_profile:
                raise ValueError('configure requires --identity and --notary-profile')
            path = Path.home() / '.config/tcld/signing.json'
            path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
            path.write_text(json.dumps({'APPLE_SIGNING_IDENTITY': args.identity,
                                        'TCLD_NOTARY_PROFILE': args.notary_profile}, indent=2) + '\n')
            path.chmod(0o600)
            print(f'Saved local signing configuration: {path}')
        else:
            environment = {}
            configure(environment)
            print('Keychain updater credential matches the configured public key.')
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        sys.exit(str(error))
