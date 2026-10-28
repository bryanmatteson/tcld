#!/usr/bin/env python3
"""Install a complete Tcld CLI archive, preserving ABI libraries on upgrades."""
import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import platform
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import uuid
import urllib.parse
import urllib.request

DEFAULT_BASE = 'https://github.com/bryanmatteson/tcld/releases/latest/download'


def fetch(url):
    parsed = urllib.parse.urlparse(url)
    if parsed.scheme != 'https' and not (parsed.scheme == 'http' and parsed.hostname in ('127.0.0.1', 'localhost', '::1')):
        raise ValueError('Downloads require HTTPS (HTTP is allowed only for loopback testing)')
    with urllib.request.urlopen(url, timeout=120) as response:
        final = urllib.parse.urlparse(response.url)
        if final.scheme != 'https' and final.hostname not in ('127.0.0.1', 'localhost', '::1'):
            raise ValueError('Download redirected to an insecure URL')
        return response.read()


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def install(base, requested, prefix, target=None):
    base = base.rstrip('/')
    if requested:
        release_version = requested.removeprefix('v')
        if base == DEFAULT_BASE:
            base = f'https://github.com/bryanmatteson/tcld/releases/download/v{release_version}'
    else:
        release_version = fetch(base + '/version.txt').decode().strip()
        if base == DEFAULT_BASE:
            base = f'https://github.com/bryanmatteson/tcld/releases/download/v{release_version}'
    if not re.fullmatch(r'\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?', release_version):
        raise ValueError('Invalid release version')
    # A release can promote an already built archive without recompilation.
    # Keep its archive name, embedded version and checksums intact while using
    # the release tag to locate every download.
    version = fetch(base + '/build-version.txt').decode().strip()
    if not re.fullmatch(r'\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?', version):
        raise ValueError('Invalid build version')
    if target is None:
        arch = {'arm64': 'aarch64', 'aarch64': 'aarch64', 'x86_64': 'x86_64', 'amd64': 'x86_64'}.get(platform.machine().lower())
        system = {'darwin': 'apple-darwin', 'linux': 'unknown-linux-gnu'}.get(sys.platform)
        if not arch or not system:
            raise ValueError('Unsupported OS or architecture; see the release downloads')
        target = arch + '-' + system
    name = f'tcld-{version}-{target}'
    filename = name + '.tar.gz'
    checksums = {}
    for line in fetch(base + '/SHA256SUMS').decode().splitlines():
        checksum, artifact = line.split(None, 1)
        artifact = artifact.strip().lstrip('*')
        if artifact in checksums or not re.fullmatch('[0-9a-f]{64}', checksum):
            raise ValueError('Invalid checksum list')
        checksums[artifact] = checksum
    archive = fetch(base + '/' + filename)
    if checksums.get(filename) != sha256(archive):
        raise ValueError('Archive checksum mismatch')
    prefix = prefix.expanduser().absolute()
    releases = prefix / 'lib/tcld/releases'
    releases.mkdir(parents=True, exist_ok=True)
    binary = prefix / 'bin/tcld'
    binary.parent.mkdir(parents=True, exist_ok=True)
    if binary.is_symlink():
        if not binary.resolve().is_relative_to(releases.resolve()):
            raise ValueError(f'Refusing to replace an unrelated symlink: {binary}')
    elif binary.exists():
        raise ValueError(f'Refusing to replace an existing file: {binary}; use another --prefix')
    with tempfile.TemporaryDirectory(prefix='.install-', dir=releases) as temp:
        staging = Path(temp)
        tar_path = staging / 'download.tar.gz'
        tar_path.write_bytes(archive)
        with tarfile.open(tar_path) as tar:
            seen = set()
            for member in tar.getmembers():
                path = PurePosixPath(member.name)
                if path.is_absolute() or '..' in path.parts or not path.parts or path.parts[0] != name:
                    raise ValueError('Unsafe archive path')
                if member.name in seen or not (member.isfile() or member.isdir()):
                    raise ValueError('Archive must contain unique regular files and directories')
                seen.add(member.name)
                destination = staging.joinpath(*path.parts)
                if member.isdir():
                    destination.mkdir(parents=True, exist_ok=True)
                else:
                    destination.parent.mkdir(parents=True, exist_ok=True)
                    with tar.extractfile(member) as src, destination.open('wb') as dst:
                        shutil.copyfileobj(src, dst)
                    destination.chmod(0o755 if member.mode & 0o111 else 0o644)
        root = staging / name
        for abi in ['tcl8.6', 'tcl9.0']:
            directory = root / 'lib/tcld/nub' / abi
            manifest = json.loads((directory / 'nub-manifest.json').read_text())
            library = manifest['library_filename']
            expected_library = 'libtcld_dbg.' + ('dylib' if 'apple-darwin' in target else 'so')
            if library != expected_library:
                raise ValueError('Unsafe debugger library filename')
            if manifest['tcl_abi'] != abi or manifest['target_triple'] != target or manifest['wire_protocol_major'] != 2 or manifest['schema_version'] != 1:
                raise ValueError('Debugger manifest mismatch')
            if sha256((directory / library).read_bytes()) != manifest['sha256']:
                raise ValueError('Debugger library checksum mismatch')
        result = subprocess.run([str(root / 'tcld'), '--version'], check=True, capture_output=True, text=True).stdout.strip()
        if result != 'tcld ' + version:
            raise ValueError(f'Installed executable version mismatch: {result}')
        destination = releases / (name + '-' + sha256(archive)[:12])
        if destination.exists():
            if destination.is_symlink():
                raise ValueError(f'Refusing a symlink installation directory: {destination}')
            # An existing content-addressed install is never trusted implicitly.
            for path in root.rglob('*'):
                if path.is_file():
                    installed = destination / path.relative_to(root)
                    if not installed.is_file() or installed.is_symlink() or sha256(installed.read_bytes()) != sha256(path.read_bytes()):
                        raise ValueError(f'Existing installation was modified: {installed}')
        else:
            os.rename(root, destination)
        temporary_link = binary.parent / ('.tcld-' + uuid.uuid4().hex)
        try:
            temporary_link.symlink_to(os.path.relpath(destination / 'tcld', binary.parent))
            os.replace(temporary_link, binary)
        finally:
            temporary_link.unlink(missing_ok=True)
    label = release_version if release_version == version else f'{release_version} (build {version})'
    print(f'Installed Tcld {label}: {binary}\nAdd {binary.parent} to PATH if needed.')
    return binary


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--version')
    parser.add_argument('--prefix', type=Path, default=Path.home() / '.local')
    parser.add_argument('--base-url', default=os.environ.get('TCLD_RELEASE_BASE_URL', DEFAULT_BASE))
    args = parser.parse_args()
    install(args.base_url, args.version, args.prefix)


if __name__ == '__main__':
    try:
        main()
    except (ValueError, OSError, KeyError, subprocess.CalledProcessError) as error:
        sys.exit(f'tcld install: {error}')
