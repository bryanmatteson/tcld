#!/usr/bin/env python3
"""Explicit local CI, package builds, assembly, and publication. No event hooks."""
from __future__ import annotations
import argparse
import datetime as dt
import hashlib
import json
import os
from pathlib import Path
import platform
import plistlib
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import tomllib
import urllib.request

ROOT = Path(__file__).resolve().parents[2]
CONFIG = tomllib.loads((ROOT / 'scripts/distribution/config.toml').read_text())
DEFAULT_OUTPUT = Path.home() / 'Documents/Developer/tcld/distribution'
TARGETS = CONFIG['targets']


def run(args, *, cwd=None, env=None, capture=False, input=None):
    args = [str(x) for x in args]
    print('+ ' + ' '.join(args), flush=True)
    return subprocess.run(args, cwd=cwd or ROOT, env=env, check=True, text=True, input=input,
                          stdout=subprocess.PIPE if capture else None).stdout


def version():
    cargo = tomllib.loads((ROOT / 'Cargo.toml').read_text())['workspace']['package']['version']
    values = [cargo]
    for name in ['apps/tcld-gui/package.json', 'apps/tcld-gui/package-lock.json',
                 'apps/tcld-tauri/tauri.conf.json']:
        document = json.loads((ROOT / name).read_text())
        values.append(document['version'])
        if name.endswith('package-lock.json'):
            values.append(document['packages']['']['version'])
    if len(set(values)) != 1:
        raise ValueError(f'Version mismatch: {values}')
    if not re.fullmatch(r'\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?', cargo):
        raise ValueError(f'Unsupported version: {cargo}')
    return cargo


def digest(path):
    with open(path, 'rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def write_json(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2) + '\n')


def unpack(archive, destination):
    # Source and product archives may contain internal symlinks, never escapes.
    with tarfile.open(archive) as tar:
        tar.extractall(destination, filter='data')


def host_target():
    arch = 'aarch64' if platform.machine() in ('arm64', 'aarch64') else 'x86_64'
    return arch + ('-apple-darwin' if sys.platform == 'darwin' else '-unknown-linux-gnu')


def rust_env(work):
    env = os.environ.copy()
    env.update(RUSTUP_TOOLCHAIN=CONFIG['rust'], CARGO_TARGET_DIR=str(work / 'target'))
    if sys.platform == 'darwin':
        # Apple's linker can emit a misaligned LINKEDIT string table for
        # optimized extension libraries without debug information.
        env.update(CARGO_PROFILE_RELEASE_DEBUG='1', CARGO_PROFILE_RELEASE_BUILD_OVERRIDE_DEBUG='1')
    env.pop('TCL_LIBRARY', None)
    env.pop('TCLLIBPATH', None)
    env.pop('TCL9_SOURCE', None)
    return env


def tcl_runtime(work, target, env, abi='9'):
    prefix = work / ('tcl' + abi)
    stamp = prefix / 'distribution-source.json'
    commit_key = 'tcl_commit' if abi == '9' else 'tcl86_commit'
    checksum_key = 'tcl_sha256' if abi == '9' else 'tcl86_sha256'
    identity = {'commit': CONFIG[commit_key], 'patch': digest(ROOT / 'third_party/tcl/patches/0001-opcode-runtime-hooks.patch') if abi == '9' else 'stock', 'target': target, 'external_scripts': True}
    if stamp.exists() and json.loads(stamp.read_text()) == identity and (abi != '9' or (prefix / 'include/tclInt.h').is_file()):
        env['TCL9_PREFIX' if abi == '9' else 'TCL8_6_PREFIX'] = str(prefix)
        return prefix
    archive = work / f'tcl{abi}-source.tar.gz'
    work.mkdir(parents=True, exist_ok=True)
    if not archive.exists():
        urllib.request.urlretrieve(f"https://api.github.com/repos/tcltk/tcl/tarball/{identity['commit']}", archive)
    if digest(archive) != CONFIG[checksum_key]:
        raise ValueError('Tcl source checksum mismatch')
    source_root = Path(tempfile.mkdtemp(prefix=f'tcl{abi}-source-{identity["patch"][:12]}-', dir=work))
    unpack(archive, source_root)
    source = next(source_root.iterdir())
    if abi == '9':
        run(['git', 'apply', '--ignore-space-change', '--ignore-whitespace', ROOT / 'third_party/tcl/patches/0001-opcode-runtime-hooks.patch'], cwd=source)
    build = source / 'distribution-build'
    build.mkdir()
    build_env = env.copy()
    if 'apple' in target:
        arch = 'arm64' if target.startswith('aarch64') else 'x86_64'
        build_env.update(CFLAGS=f'-O2 -arch {arch} -mmacosx-version-min={CONFIG["macos_minimum"]}',
                         LDFLAGS=f'-arch {arch} -mmacosx-version-min={CONFIG["macos_minimum"]}')
    configure = [source / 'unix/configure', f'--prefix={prefix}', '--enable-threads', '--disable-framework']
    if abi == '9':
        configure.append('--disable-zipfs')
    run(configure, cwd=build, env=build_env)
    run(['make', f'-j{min(os.cpu_count() or 4, 8)}'], cwd=build, env=build_env)
    run(['make', 'install'], cwd=build, env=build_env)
    if abi == '9':
        run(['make', 'install-private-headers', f'PRIVATE_INCLUDE_INSTALL_DIR={prefix / "include"}'], cwd=build, env=build_env)
    write_json(stamp, identity)
    env['TCL9_PREFIX' if abi == '9' else 'TCL8_6_PREFIX'] = str(prefix)
    return prefix


def tools(env):
    run(['rustup', 'toolchain', 'install', CONFIG['rust'], '--no-self-update', '--profile', 'minimal', '--component', 'rustfmt,clippy'])
    cargo = Path(run(['rustup', 'which', '--toolchain', CONFIG['rust'], 'cargo'], capture=True).strip())
    # Use the selected toolchain directly. Shell aliases and machine-wide Cargo
    # wrappers must not redirect per-ABI build outputs into one target directory.
    env['PATH'] = str(cargo.parent) + os.pathsep + env.get('PATH', '')
    installed = run(['cargo', 'tauri', '--version'], env=env, capture=True).strip()
    if installed != f'tauri-cli {CONFIG["tauri_cli"]}':
        raise ValueError(f'Install cargo tauri {CONFIG["tauri_cli"]}; found {installed}')
    if int(run(['node', '--version'], capture=True).strip().lstrip('v').split('.')[0]) < 22:
        raise ValueError('Node 22 or newer is required')
    deny = run(['cargo', 'deny', '--version'], env=env, capture=True).strip()
    if deny != f'cargo-deny {CONFIG["cargo_deny"]}':
        raise ValueError(f'Install cargo-deny {CONFIG["cargo_deny"]}; found {deny}')


def nubs(work, target, env):
    ext = 'dylib' if 'apple' in target else 'so'
    paths = []
    for abi, feature in [('tcl8.6', 'tcl8_6'), ('tcl9.0', 'tcl9')]:
        directory = work / ('nub-' + abi)
        run(['cargo', 'build', '--locked', '--release', '-p', 'tcld-dbg-ext', '--no-default-features',
             '--features', feature + ',loadable-extension', '--target', target, '--target-dir', directory], env=env)
        paths.append((abi, directory / target / 'release' / ('libtcld_dbg.' + ext)))
    return paths


def package_nubs(paths, destination, target, *, sign=False):
    for abi, library in paths:
        run([ROOT / 'scripts/package-debug-nub.sh', library, abi, destination, target])
        installed = destination / 'lib/tcld/nub' / abi / library.name
        if sign and sys.platform == 'darwin':
            run(['codesign', '--force', '--options', 'runtime', '--timestamp', '--sign', os.environ['APPLE_SIGNING_IDENTITY'], installed])
            manifest = installed.with_name('nub-manifest.json')
            data = json.loads(manifest.read_text())
            data['sha256'] = digest(installed)
            write_json(manifest, data)


def archive_tree(source, destination):
    with tarfile.open(destination, 'w:gz') as archive:
        archive.add(source, arcname=source.name)


def clean_env():
    env = os.environ.copy()
    for key in list(env):
        if key.startswith(('TCL', 'DYLD_', 'LD_LIBRARY_PATH', 'CARGO_', 'RUST')):
            env.pop(key)
    env.update(PATH='/usr/bin:/bin:/usr/sbin:/sbin', NO_COLOR='1')
    return env


def verify_cli(bundle, target, env):
    verification_env = env | {'TCLD_BUNDLE_TARGET': target,
        'TCLD_TCLSH9': env['TCL9_PREFIX'] + '/bin/tclsh9.0',
        'TCLD_TCL8_BYTECODE_FIXTURE': str(ROOT / 'apps/tcld/tests/fixtures/expr.tbc')}
    run([ROOT / 'scripts/verify-debug-release-bundle.sh', bundle], env=verification_env)
    binary = bundle / 'tcld' if (bundle / 'tcld').is_file() else bundle / 'bin/tcld'
    actual = run([binary, '--version'], cwd=bundle, env=clean_env(), capture=True).strip()
    if actual != 'tcld ' + version():
        raise ValueError(f'Wrong installed version: {actual}')
    result = run([binary, 'decompile', ROOT / 'testing/fixtures/public/public_complex_fixture.tbc'],
                 cwd=bundle, env=clean_env(), capture=True)
    if 'proc ' not in result:
        raise ValueError('Installed CLI did not decompile public fixture')


def relocate_macos(binary, libraries):
    """Copy non-system dependencies recursively and rewrite every load command."""
    libraries.mkdir(parents=True, exist_ok=True)
    visited = set()
    def visit(image):
        if image in visited:
            return
        visited.add(image)
        dependencies = run(['otool', '-L', image], capture=True).splitlines()[1:]
        for line in dependencies:
            old = line.strip().split(' (', 1)[0]
            if old.startswith(('/System/', '/usr/lib/')):
                continue
            if old == str(image) or old.startswith('@'):
                continue
            dependency = Path(old)
            if not dependency.is_file():
                raise ValueError(f'Unresolved dependency {old} in {image}')
            dest = libraries / dependency.name
            if dest not in visited:
                shutil.copy2(dependency.resolve(), dest)
                dest.chmod(0o755)
                run(['install_name_tool', '-id', '@rpath/' + dest.name, dest])
                visit(dest)
            run(['install_name_tool', '-change', old, '@rpath/' + dest.name, image])
    visit(binary)
    loads = run(['otool', '-l', binary], capture=True)
    rpaths = re.findall(r'path (.+?) \(offset \d+\)', loads)
    for rpath in rpaths:
        if rpath.startswith('/') and not rpath.startswith(('/System/', '/usr/lib/')):
            run(['install_name_tool', '-delete_rpath', rpath, binary])
    if '@executable_path/../Frameworks' not in rpaths:
        run(['install_name_tool', '-add_rpath', '@executable_path/../Frameworks', binary])
    # Relocation invalidates the linker signature; Apple Silicon needs even
    # development binaries to have a valid ad-hoc signature.
    for image in [*libraries.glob('*.dylib'), binary]:
        run(['codesign', '--force', '--sign', '-', image])


def desktop_config(work, prefix, paths, target, signed):
    resources = work / 'resources'
    resources.mkdir(parents=True, exist_ok=True)
    for directory in ['tcl9.0', 'tcl9']:
        shutil.copytree(prefix / 'lib' / directory, resources / directory, dirs_exist_ok=True)
    package_nubs(paths, resources, target, sign=signed)  # Tauri seals Resources without re-signing these files
    licenses = resources / 'licenses'
    licenses.mkdir(exist_ok=True)
    for source in [ROOT / 'LICENSE', ROOT / 'THIRD_PARTY_NOTICES.md', ROOT / 'crates/tcld-vm/src/regex/license.terms']:
        shutil.copy2(source, licenses / source.name)
    # Paths outside the source tree need explicit destination mappings.
    resource_map = {str(resources / name): name + '/' for name in ['tcl9.0', 'tcl9', 'lib', 'licenses']}
    for script in (ROOT / 'apps/tcld-tauri/src/resources').glob('*.tcl'):
        resource_map[str(script)] = script.name
    config = {'build': {'beforeBuildCommand': ''},
              'bundle': {'resources': resource_map, 'icon': ['icons/icon.icns', 'icons/128x128.png'],
                         # AppImage metadata is finalized after linuxdeploy
                         # rewrites its ELF files; sign only that final payload.
                         'createUpdaterArtifacts': signed and 'apple' in target,
                         'category': 'DeveloperTool', 'shortDescription': 'Tcl bytecode analysis and debugging'}}
    if 'apple' in target:
        config['bundle']['macOS'] = {'minimumSystemVersion': CONFIG['macos_minimum'],
            'frameworks': [str(path) for path in (work / 'frameworks').glob('*.dylib')]}
    else:
        lib = prefix / 'lib/libtcl9.0.so'
        files = {'usr/lib/tcld-tauri/libtcl9.0.so': str(lib)}
        config['bundle']['linux'] = {'appimage': {'files': files}, 'deb': {'files': {'/usr/lib/tcld-tauri/libtcl9.0.so': str(lib)}}}
    if signed:
        pubkey = os.environ.get('TCLD_UPDATER_PUBLIC_KEY', '').strip()
        if not pubkey:
            raise ValueError('TCLD_UPDATER_PUBLIC_KEY must contain the public key (not a path)')
        config['plugins'] = {'updater': {'pubkey': pubkey, 'endpoints': [f'https://github.com/{CONFIG["repository"]}/releases/latest/download/latest.json']}}
    path = work / 'tauri-distribution.json'
    write_json(path, config)
    return path


def finalize_appimage(bundle_dir, env, signed):
    appimage = next((bundle_dir / 'appimage').glob('*.AppImage'))
    plugin = Path.home() / '.cache/tauri/linuxdeploy-plugin-appimage.AppImage'
    with tempfile.TemporaryDirectory(prefix='tcld-appimage-finalize-', dir=bundle_dir) as tmp:
        temporary = Path(tmp)
        run([appimage, '--appimage-extract'], cwd=temporary)
        root = temporary / 'squashfs-root'
        for manifest in (root / 'usr/lib/TCLD/lib/tcld/nub').glob('*/nub-manifest.json'):
            data = json.loads(manifest.read_text())
            data['sha256'] = digest(manifest.parent / data['library_filename'])
            write_json(manifest, data)
        result = temporary / appimage.name
        run([plugin, '--appimage-extract-and-run', '--appdir', root],
            env=env | {'OUTPUT': str(result), 'ARCH': 'x86_64', 'APPIMAGE_EXTRACT_AND_RUN': '1'})
        shutil.copy2(result, appimage)
    if signed:
        signing_env = env.copy()
        key = signing_env['TAURI_SIGNING_PRIVATE_KEY']
        if '\n' not in key and len(key) < 240 and Path(key).is_file():
            signing_env['TAURI_SIGNING_PRIVATE_KEY'] = Path(key).read_text()
        run(['cargo', 'tauri', 'signer', 'sign', appimage], env=signing_env)


def verify_desktop(executable, root, target, signed, build_env):
    if 'apple' in target:
        info = plistlib.loads((root / 'Contents/Info.plist').read_bytes())
        if root / 'Contents/MacOS' / info['CFBundleExecutable'] != executable:
            raise ValueError('Desktop bundle launcher does not match its executable')
        if info['CFBundleIdentifier'] != json.loads((ROOT / 'apps/tcld-tauri/tauri.conf.json').read_text())['identifier']:
            raise ValueError('Desktop bundle identifier does not match its configuration')
    environment = clean_env() | {'TCL_LIBRARY': '/nonexistent-build-library'}
    result = run([executable, '--distribution-smoke-test'], cwd=root, env=environment, capture=True)
    if 'TCLD distribution smoke passed' not in result or str(root) not in result:
        raise ValueError(f'Desktop did not initialize its packaged library: {result}')
    resource_root = root / 'Contents/Resources' if 'apple' in target else root / 'usr/lib/TCLD'
    for abi, shell in [('tcl8.6', build_env['TCLD_TCLSH86']), ('tcl9.0', build_env['TCL9_PREFIX'] + '/bin/tclsh9.0')]:
        directory = resource_root / 'lib/tcld/nub' / abi
        manifest = json.loads((directory / 'nub-manifest.json').read_text())
        library = directory / manifest['library_filename']
        if manifest['sha256'] != digest(library) or manifest['target_triple'] != target:
            raise ValueError(f'Packaged desktop nub manifest mismatch: {library}')
        loaded = run([shell], input=f'load {{{library}}} Tcld_dbg\nputs [package require tcld_dbg]\n', env=clean_env(), capture=True)
        if loaded.strip() != version():
            raise ValueError(f'Packaged desktop nub did not load: {library}')
    with tempfile.TemporaryDirectory(prefix='tcld-ui-home-', dir=root.parent) as home:
        ui_env = environment | {'HOME': home, 'XDG_CONFIG_HOME': home + '/config', 'XDG_CACHE_HOME': home + '/cache'}
        command = [str(executable), '--distribution-ui-smoke-test', str(ROOT / 'testing/fixtures/public/public_complex_fixture.tbc')]
        if 'linux' in target:
            ui_env.update(WEBKIT_DISABLE_COMPOSITING_MODE='1', WEBKIT_DISABLE_SANDBOX_THIS_IS_DANGEROUS='1')
            command = ['xvfb-run', '-a', *command]
        print('+ installed desktop UI smoke', flush=True)
        result = subprocess.run(command, cwd=root, env=ui_env, capture_output=True, text=True, timeout=45)
        if result.returncode != 0 or 'TCLD installed UI opened and rendered bytecode' not in result.stdout:
            raise ValueError(f'Installed UI smoke failed (exit {result.returncode}): {result.stdout} {result.stderr}')
    if 'apple' in target:
        for image in [executable, *root.glob('Contents/Frameworks/*.dylib')]:
            deps = '\n'.join(run(['otool', '-L', image], capture=True).splitlines()[1:])
            if any(x in deps for x in ['/opt/homebrew', '/Cellar/', '/Users/', '/usr/local/']):
                raise ValueError(f'Build-machine dependency in {image}: {deps}')
        if signed:
            run(['codesign', '--verify', '--deep', '--strict', root])
            run(['spctl', '--assess', '--type', 'execute', root])
            run(['xcrun', 'stapler', 'validate', root])


def cli_documentation(bundle):
    for name in ['LICENSE', 'README.md', 'COMPAT.md', 'DEBUGGING.md', 'DISTRIBUTION.md', 'THIRD_PARTY_NOTICES.md']:
        shutil.copy2(ROOT / name, bundle / name)
    examples = bundle / 'examples'
    examples.mkdir(exist_ok=True)
    for name in ['public_complex_fixture.tbc', 'public_complex_fixture.tcl']:
        shutil.copy2(ROOT / 'testing/fixtures/public' / name, examples / name)
    (bundle / 'licenses').mkdir(exist_ok=True)
    shutil.copy2(ROOT / 'crates/tcld-vm/src/regex/license.terms', bundle / 'licenses/Tcl-license.terms')


def build_target(output, target, signed=False):
    work = Path(os.environ.get('TCLD_DISTRIBUTION_WORK', output / 'work')) / target
    work.mkdir(parents=True, exist_ok=True)
    env = rust_env(work)
    tools(env)
    run(['rustup', 'target', 'add', '--toolchain', CONFIG['rust'], target])
    if 'apple' in target:
        env['MACOSX_DEPLOYMENT_TARGET'] = CONFIG['macos_minimum']
    prefix = tcl_runtime(work, target, env)
    tcl86 = tcl_runtime(work, target, env, '86')
    env['TCLD_TCLSH86'] = str(tcl86 / 'bin/tclsh8.6')
    paths = nubs(work, target, env)
    artifacts = output / 'artifacts'
    artifacts.mkdir(parents=True, exist_ok=True)
    bundle = work / f'tcld-{version()}-{target}'
    bundle.mkdir(exist_ok=True)
    run(['cargo', 'build', '--locked', '--release', '-p', 'tcld', '--target', target], env=env)
    shutil.copy2(work / 'target' / target / 'release/tcld', bundle / 'tcld')
    cli_documentation(bundle)
    package_nubs(paths, bundle, target, sign=signed)
    if signed and 'apple' in target:
        run(['codesign', '--force', '--options', 'runtime', '--timestamp', '--sign', os.environ['APPLE_SIGNING_IDENTITY'], bundle / 'tcld'])
    archive = artifacts / (bundle.name + '.tar.gz')
    archive_tree(bundle, archive)
    with tempfile.TemporaryDirectory(prefix='tcld-install-', dir=work) as tmp:
        unpack(archive, tmp)
        verify_cli(Path(tmp) / bundle.name, target, env)
    with tempfile.TemporaryDirectory(prefix='tcld-prefix-', dir=work) as tmp:
        installed = Path(tmp)
        (installed / 'bin').mkdir()
        shutil.copy2(bundle / 'tcld', installed / 'bin/tcld')
        shutil.copytree(bundle / 'lib', installed / 'lib')
        verify_cli(installed, target, env)
    run(['npm', 'ci', '--prefix', 'apps/tcld-gui'], env=env)
    run(['npm', 'run', 'build', '--prefix', 'apps/tcld-gui'], env=env)
    # Generate initial config, build, then relocate before bundling/signing.
    config = desktop_config(work, prefix, paths, target, signed)
    features = 'hybrid' + (',updater' if signed else '')
    app_dir = ROOT / 'apps/tcld-tauri'
    run(['cargo', 'tauri', 'build', '--ci', '--no-bundle', '--target', target, '--features', features, '--config', config, '--', '--locked'], cwd=app_dir, env=env)
    executable = work / 'target' / target / 'release/tcld-tauri'
    # Cargo may hard-link its final executable to a shared build-cache product.
    # Relocation and Tauri's bundle-type patching must touch only this package.
    detached = executable.with_name('tcld-tauri-distribution-copy')
    shutil.copy2(executable, detached)
    os.replace(detached, executable)
    if 'apple' in target:
        relocate_macos(executable, work / 'frameworks')
    else:
        run(['patchelf', '--set-rpath', '$ORIGIN/../lib/tcld-tauri', executable])
    config = desktop_config(work, prefix, paths, target, signed)
    command = ['cargo', 'tauri', 'bundle', '--ci', '--target', target, '--features', features,
               '--config', config, '--bundles', 'app,dmg' if 'apple' in target else 'appimage,deb']
    if not signed:
        command.append('--no-sign')
    bundle_dir = executable.parent / 'bundle'
    for pattern in ['macos/*.app.tar.gz', 'macos/*.app.tar.gz.sig', 'dmg/*.dmg', 'appimage/*.AppImage', 'appimage/*.AppImage.sig', 'deb/*.deb']:
        for previous in bundle_dir.glob(pattern):
            previous.unlink()
    run(command, cwd=app_dir, env=env)
    if 'linux' in target:
        finalize_appimage(bundle_dir, env, signed)
    verify_and_collect(output, work, target, signed, env)


def verify_and_collect(output, work, target, signed, env):
    artifacts = output / 'artifacts'
    archive = artifacts / f'tcld-{version()}-{target}.tar.gz'
    executable = work / 'target' / target / 'release/tcld-tauri'
    bundle_dir = executable.parent / 'bundle'
    produced = [archive.name]
    if 'apple' in target:
        dmg = next((bundle_dir / 'dmg').glob('*.dmg'))
        mounted = plistlib.loads(run(['hdiutil', 'attach', '-readonly', '-nobrowse', '-plist', dmg], capture=True).encode())
        volumes = [Path(entity['mount-point']) for entity in mounted['system-entities'] if 'mount-point' in entity]
        try:
            app = next(volume / 'TCLD.app' for volume in volumes if (volume / 'TCLD.app').is_dir())
            with tempfile.TemporaryDirectory(prefix='tcld-app-', dir=work) as tmp:
                moved = Path(tmp) / 'TCLD.app'
                shutil.copytree(app, moved, symlinks=True)
                verify_desktop(moved / 'Contents/MacOS/tcld-tauri', moved, target, signed, env)
        finally:
            for volume in reversed(volumes):
                run(['hdiutil', 'detach', volume])
        # The updater downloads this separate archive rather than the DMG.
        updates = list((bundle_dir / 'macos').glob('*.app.tar.gz'))
        if signed and len(updates) != 1:
            raise ValueError('Expected one macOS updater archive')
        for update in updates:
            with tempfile.TemporaryDirectory(prefix='tcld-update-', dir=work) as tmp:
                unpack(update, tmp)
                moved = Path(tmp) / 'TCLD.app'
                verify_desktop(moved / 'Contents/MacOS/tcld-tauri', moved, target, signed, env)
    else:
        # Verify the package contents after extraction, including its loader path.
        deb = next((bundle_dir / 'deb').glob('*.deb'))
        with tempfile.TemporaryDirectory(prefix='tcld-deb-', dir=work) as tmp:
            run(['dpkg-deb', '-x', deb, tmp])
            verify_desktop(Path(tmp) / 'usr/bin/tcld-tauri', Path(tmp), target, signed, env)
        appimage = next((bundle_dir / 'appimage').glob('*.AppImage'))
        with tempfile.TemporaryDirectory(prefix='tcld-appimage-', dir=work) as tmp:
            run([appimage, '--appimage-extract'], cwd=tmp)
            root = Path(tmp) / 'squashfs-root'
            verify_desktop(root / 'AppRun', root, target, signed, env)
    patterns = (['dmg/*.dmg', 'macos/*.app.tar.gz', 'macos/*.app.tar.gz.sig']
                if 'apple' in target else ['appimage/*.AppImage', 'appimage/*.AppImage.sig', 'deb/*.deb'])
    for source in sorted(path for pattern in patterns for path in bundle_dir.glob(pattern)):
        # Include only final installers/update payloads, preserving .sig pairing.
        name = f'TCLD-{target}-{source.name}'
        shutil.copy2(source, artifacts / name)
        produced.append(name)
    write_json(output / 'targets' / (target + '.json'), {
        'target': target, 'version': version(), 'signed': signed,
        'commit': os.environ.get('TCLD_RELEASE_COMMIT'),
        'artifacts': {name: digest(artifacts / name) for name in produced},
    })


def ci_native(output):
    work = output / 'ci' / host_target()
    work.mkdir(parents=True, exist_ok=True)
    env = rust_env(work)
    if sys.platform == 'linux':
        env.setdefault('TCLD_TEST_WAIT_SCALE', '3')
    tools(env)
    prefix = tcl_runtime(work, host_target(), env)
    tcl86 = tcl_runtime(work, host_target(), env, '86')
    env['TCLD_TCLSH86'] = str(tcl86 / 'bin/tclsh8.6')
    env['TCLD_TCLSH'] = str(prefix / 'bin/tclsh9.0')
    run(['npm', 'ci', '--prefix', 'apps/tcld-gui'], env=env)
    run(['cargo', 'fmt', '--all', '--', '--check'], env=env)
    features = 'tcld/hybrid,tcld-tauri/hybrid,tcld-tauri/updater'
    run(['cargo', 'clippy', '--locked', '--workspace', '--all-targets', '--features', features, '--', '-D', 'warnings'], env=env)
    env['TCLD_CORPUS_REPORT'] = str(work / 'hybrid-corpus-report.json')
    library = prefix / ('lib/libtcl9.0.dylib' if sys.platform == 'darwin' else 'lib/libtcl9.0.so')
    symbols = run(['nm', '-gU' if sys.platform == 'darwin' else '-D', library], capture=True)
    for symbol in ['tcld_opcode_supported', 'tcld_opcode_set_hook', 'tcld_opcode_arm', 'tcld_opcode_current_pc', 'tcld_opcode_current_source']:
        if symbol not in symbols:
            raise ValueError(f'Patched Tcl runtime is missing {symbol}')
    # One workspace pass includes the hybrid corpus and feature-bearing CLI
    # contracts. Standalone hybrid-gates would repeat these broad suites.
    run(['cargo', 'test', '--locked', '--workspace', '--features', features], env=env)
    run(['cargo', 'build', '--locked', '-p', 'tcld', '--features', 'hybrid'], env=env)
    binary = work / 'target/debug/tcld'
    fixtures = ROOT / 'apps/tcld/tests/fixtures'
    for arguments in [['run', fixtures / 'proc.tbc'], ['run', fixtures / 'break.tbc'], ['run', '--sandbox', fixtures / 'break.tbc']]:
        run([binary, *arguments], env=env)
    run(['cargo', 'test', '--locked', '-p', 'tcld', '--no-default-features'], env=env)
    run(['cargo', 'build', '--locked', '-p', 'tcld-dbg-ext', '--features', 'integration_tclsh'], env=env)
    run(['cargo', 'test', '--locked', '-p', 'tcld-dbg-ext', '--features', 'integration_tclsh'], env=env)
    tcl86_env = env | {'TCLD_TCLSH': env['TCLD_TCLSH86'], 'CARGO_TARGET_DIR': str(work / 'test-tcl86')}
    run(['cargo', 'build', '--locked', '-p', 'tcld-dbg-ext', '--no-default-features', '--features', 'tcl8_6,integration_tclsh', '--target-dir', work / 'test-tcl86'], env=tcl86_env)
    run(['cargo', 'test', '--locked', '-p', 'tcld-dbg-ext', '--no-default-features', '--features', 'tcl8_6,integration_tclsh', '--target-dir', work / 'test-tcl86', '--lib', '--test', 'wire_real_tclsh', '--test', 'wire_parity_matrix'], env=tcl86_env)
    gui = ROOT / 'apps/tcld-gui'
    for command in [['npm', 'run', 'lint'], ['npm', 'run', 'design:parity'], ['npx', 'tsc', '--noEmit'], ['npm', 'run', 'test:unit']]:
        run(command, cwd=gui, env=env)
    run(['cargo', 'deny', 'check', 'advisories'], env=env)
    run([sys.executable, '-m', 'unittest', 'discover', '-s', 'scripts/distribution/tests', '-v'], env=env)


def docker(output, action, target, signed=False):
    if sys.platform != 'darwin':
        raise ValueError('OrbStack orchestration runs on macOS')
    architecture = 'arm64' if target.startswith('aarch64') else 'amd64'
    image = CONFIG['linux_image'] + '-' + architecture
    base = ['docker', '--context', 'orbstack']
    run(base + ['build', '--platform', 'linux/' + architecture, '--build-arg', 'RUST_VERSION=' + CONFIG['rust'],
        '--build-arg', 'NODE_VERSION=' + CONFIG['node'], '--build-arg', 'TAURI_CLI_VERSION=' + CONFIG['tauri_cli'],
        '--build-arg', 'CARGO_DENY_VERSION=' + CONFIG['cargo_deny'],
        '-t', image, ROOT / 'scripts/distribution'])
    output.mkdir(parents=True, exist_ok=True)
    environment = os.environ.copy()
    args = base + ['run', '--rm', '--platform', 'linux/' + architecture, '--shm-size=2g',
        '--mount', f'type=bind,src={ROOT},dst=/source,readonly',
        '--mount', f'type=bind,src={output},dst=/artifacts',
        '--mount', f'type=volume,src=tcld-build-{architecture},dst=/cache',
        '--mount', 'type=volume,src=tcld-cargo-registry,dst=/root/.cargo/registry',
        '--mount', 'type=volume,src=tcld-cargo-git,dst=/root/.cargo/git',
        '--mount', 'type=volume,src=tcld-tauri-tools,dst=/root/.cache/tauri']
    for key in ['TCLD_UPDATER_PUBLIC_KEY', 'TAURI_SIGNING_PRIVATE_KEY', 'TAURI_SIGNING_PRIVATE_KEY_PASSWORD', 'TCLD_RELEASE_COMMIT']:
        if key in environment:
            if key == 'TAURI_SIGNING_PRIVATE_KEY' and '\n' not in environment[key] and len(environment[key]) < 240 and Path(environment[key]).is_file():
                environment[key] = Path(environment[key]).read_text()
            args += ['-e', key]
    args += [image, 'python3', '/source/scripts/distribution/release.py', '_container',
             '--output', '/artifacts', '--target', target, '--action', action]
    if signed:
        args.append('--signed')
    run(args, env=environment)


def container_entry(args):
    global ROOT
    source = Path(tempfile.mkdtemp(prefix='tcld-source-', dir='/work'))
    def ignored(directory, names):
        exclusions = set(shutil.ignore_patterns('.git', '.claude', 'target', 'node_modules', 'dist', '.next', '.design-parity', '__pycache__')(directory, names))
        if Path(directory) == ROOT / 'third_party/tcl':
            exclusions.update({'src', 'build', 'runtime'} & set(names))
        return exclusions
    shutil.copytree(ROOT, source, dirs_exist_ok=True, ignore=ignored)
    ROOT = source
    # Compilation cache is machine-local; artifacts always leave through /artifacts.
    cache = Path('/cache')
    (args.output / 'work').mkdir(parents=True, exist_ok=True)
    if args.action == 'ci':
        ci_native(cache)
    else:
        os.environ['TCLD_DISTRIBUTION_WORK'] = '/cache/build'
        build_target(args.output, args.target, args.signed)


def assemble(output, *, require_signed=False):
    release_version = version()
    metadata = [json.loads(path.read_text()) for path in sorted((output / 'targets').glob('*.json'))]
    if not metadata:
        raise ValueError('No verified target builds to assemble')
    files = {}
    updates = {}
    for item in metadata:
        if item['version'] != release_version or (require_signed and not item['signed']):
            raise ValueError(f'Unqualified target: {item["target"]}')
        for name, expected in item['artifacts'].items():
            path = output / 'artifacts' / name
            if digest(path) != expected:
                raise ValueError(f'Artifact changed after verification: {name}')
            files[name] = expected
            if name.endswith('.sig'):
                artifact = name[:-4]
                if artifact.endswith(('.AppImage', '.app.tar.gz')):
                    target = item['target']
                    key = ('darwin' if 'apple' in target else 'linux') + '-' + ('aarch64' if target.startswith('aarch64') else 'x86_64')
                    updates[key] = {'signature': path.read_text().strip(),
                        'url': f'https://github.com/{CONFIG["repository"]}/releases/download/v{release_version}/{artifact}'}
    if require_signed:
        expected_platforms = {('darwin' if 'apple' in item['target'] else 'linux') + '-' + ('aarch64' if item['target'].startswith('aarch64') else 'x86_64') for item in metadata}
        if set(updates) != expected_platforms:
            raise ValueError('Every release desktop target must have a signed updater artifact')
    artifacts = output / 'artifacts'
    expected_names = set(files) | {'install.sh', 'install.py', 'latest.json', 'version.txt', 'build-version.txt', 'SHA256SUMS'}
    unexpected = {path.name for path in artifacts.iterdir() if path.is_file()} - expected_names
    if unexpected:
        raise ValueError(f'Untracked release artifacts: {sorted(unexpected)}; use a fresh output directory')
    (artifacts / 'version.txt').write_text(release_version + '\n')
    (artifacts / 'build-version.txt').write_text(release_version + '\n')
    for name in ['install.sh', 'install.py']:
        shutil.copy2(ROOT / 'scripts/distribution' / name, artifacts / name)
    if not updates:
        (artifacts / 'latest.json').unlink(missing_ok=True)
    if updates:
        write_json(artifacts / 'latest.json', {'version': release_version,
            'pub_date': dt.datetime.now(dt.timezone.utc).isoformat(), 'platforms': updates})
    homebrew(output, metadata)
    for path in sorted(artifacts.iterdir()):
        if path.is_file() and path.name != 'SHA256SUMS':
            files[path.name] = digest(path)
    (artifacts / 'SHA256SUMS').write_text(''.join(f'{sha}  {name}\n' for name, sha in sorted(files.items())))
    write_json(output / 'release.json', {'version': release_version, 'targets': metadata,
                                        'artifacts': files})


def homebrew(output, metadata, *, allow_unsigned=False):
    ver = version()
    build_versions = {item['version'] for item in metadata}
    if len(build_versions) != 1:
        raise ValueError('Homebrew packages must use one build version')
    build_version = next(iter(build_versions))
    base = f'https://github.com/{CONFIG["repository"]}/releases/download/v{ver}'
    formula = ['class Tcld < Formula', '  desc "Tcl bytecode analysis and debugging"',
        f'  homepage "https://github.com/{CONFIG["repository"]}"', f'  version "{ver}"', '  license "Apache-2.0"']
    for os_name, selector in [('macos', 'apple'), ('linux', 'linux')]:
        entries = [item for item in metadata if selector in item['target']]
        if entries:
            formula.append(f'  on_{os_name} do')
            revision = max(item.get('package_revision', 0) for item in entries)
            if revision:
                formula.append(f'    revision {revision}')
            for item in entries:
                name = f'tcld-{build_version}-{item["target"]}.tar.gz'
                cpu = 'arm' if item['target'].startswith('aarch64') else 'intel'
                suffix = f'?revision={item["package_revision"]}' if item.get('package_revision') else ''
                formula.extend([f'    on_{cpu} do', f'      url "{base}/{name}{suffix}"',
                    f'      sha256 "{item["artifacts"][name]}"', '    end'])
            formula.append('  end')
    formula.extend(['  def install', '    bin.install "tcld"', '    (lib/"tcld").install "lib/tcld/nub"', '    pkgshare.install "examples"',
        '  end', '  test do', f'    assert_match "tcld {build_version}", shell_output("#{{bin}}/tcld --version")', '    assert_match "proc ", shell_output("#{bin}/tcld decompile #{pkgshare}/examples/public_complex_fixture.tbc")', '  end', 'end', ''])
    tap = output / 'homebrew'
    (tap / 'Formula').mkdir(parents=True, exist_ok=True)
    (tap / 'Formula/tcld.rb').write_text('\n'.join(formula))
    mac = [item for item in metadata if 'apple' in item['target'] and (item['signed'] or item.get('apple_signed') or allow_unsigned)]
    if not mac:
        (tap / 'Casks/tcld.rb').unlink(missing_ok=True)
    if mac:
        revision = max(item.get('package_revision', 0) for item in mac)
        cask_version = f'{ver},{revision}' if revision else ver
        lines = ['cask "tcld" do', f'  version "{cask_version}"']
        for item in mac:
            name = next(name for name in item['artifacts'] if name.endswith('.dmg'))
            cpu = 'arm' if item['target'].startswith('aarch64') else 'intel'
            lines += [f'  on_{cpu} do', f'    sha256 "{item["artifacts"][name]}"', f'    url "{base}/{name}"', '  end']
        lines += ['  name "TCLD"', '  desc "Tcl bytecode analysis workbench"',
            f'  homepage "https://github.com/{CONFIG["repository"]}"',
            '  depends_on macos: :big_sur', '  app "TCLD.app"']
        if build_version != ver or any(not item['signed'] for item in mac):
            lines += ['  caveats <<~EOS',
                f'    Release {ver} reuses build {build_version} without recompilation.',
                '    The application reports its original build version.']
            if any(not (item['signed'] or item.get('apple_signed')) for item in mac):
                lines.append('    This desktop build is unsigned and not notarized; in-app updates are unavailable.')
            else:
                if any(not (item['signed'] or item.get('notarized')) for item in mac):
                    lines.append('    This desktop build is signed but not notarized.')
                if any(not item['signed'] for item in mac):
                    lines.append('    In-app updates are unavailable; use Homebrew to update.')
            lines.append('  EOS')
        lines += ['end', '']
        (tap / 'Casks').mkdir(exist_ok=True)
        (tap / 'Casks/tcld.rb').write_text('\n'.join(lines))


def signing_environment(*, apple=True):
    keys = ['TAURI_SIGNING_PRIVATE_KEY', 'TCLD_UPDATER_PUBLIC_KEY']
    if apple:
        keys.append('APPLE_SIGNING_IDENTITY')
    for key in keys:
        if not os.environ.get(key):
            raise ValueError(f'{key} is required for a release; see DISTRIBUTION.md')
    if not apple:
        return
    apple_id = all(os.environ.get(key) for key in ['APPLE_ID', 'APPLE_PASSWORD', 'APPLE_TEAM_ID'])
    apple_api = all(os.environ.get(key) for key in ['APPLE_API_ISSUER', 'APPLE_API_KEY', 'APPLE_API_KEY_PATH'])
    if not (apple_id or apple_api):
        raise ValueError('Apple notarization credentials are required; see DISTRIBUTION.md')


def release(output, snapshot_commit=None):
    signing_environment()
    if not snapshot_commit:
        # Export committed source without requiring unrelated shared-tree work
        # to stop or become part of the release.
        commit = run(['git', 'rev-parse', 'HEAD'], capture=True).strip()
        print(f'Preparing committed source {commit}; uncommitted files are excluded.', flush=True)
        destination = output / 'source' / commit
        if destination.exists():
            raise ValueError(f'Release snapshot already exists at {destination}; choose a new --output directory')
        destination.mkdir(parents=True)
        with tempfile.TemporaryFile() as stream:
            subprocess.run(['git', 'archive', commit], cwd=ROOT, stdout=stream, check=True)
            stream.seek(0)
            with tarfile.open(fileobj=stream) as archive:
                archive.extractall(destination, filter='data')
        env = os.environ | {'TCLD_RELEASE_COMMIT': commit}
        run([sys.executable, destination / 'scripts/distribution/release.py', 'release', '--output', output,
             '--snapshot-commit', commit], cwd=destination, env=env)
        return
    ci_native(output)
    docker(output, 'ci', 'x86_64-unknown-linux-gnu')
    for target in TARGETS:
        if 'apple' in target:
            build_target(output, target, signed=True)
        else:
            docker(output, 'build', target, signed=True)
    assemble(output, require_signed=True)
    print(f'Release prepared locally at {output}; publishing is a separate explicit command.')


def publish(output):
    document = json.loads((output / 'release.json').read_text())
    if document['version'] != version() or {item['target'] for item in document['targets']} != set(TARGETS):
        raise ValueError('Release must include the configured target matrix at the current version')
    commits = {item['commit'] for item in document['targets']}
    if len(commits) != 1 or None in commits or any(not item['signed'] for item in document['targets']):
        raise ValueError('Publication requires a signed release built from one committed snapshot')
    for name, sha in document['artifacts'].items():
        if digest(output / 'artifacts' / name) != sha:
            raise ValueError(f'Changed artifact: {name}')
    tag = 'v' + version()
    actual = run(['git', 'rev-parse', tag + '^{commit}'], capture=True).strip()
    if actual not in commits:
        raise ValueError('Release tag must identify the source commit used for these artifacts')
    notes = output / 'release-notes.md'
    if not notes.is_file():
        raise ValueError(f'Write release notes to {notes} before publishing')
    remote = run(['gh', 'api', f'repos/{CONFIG["repository"]}/commits/{tag}', '--jq', '.sha'], capture=True).strip()
    if remote not in commits:
        raise ValueError('Remote release tag does not identify the package source commit')
    run(['gh', 'release', 'create', tag, *[output / 'artifacts' / name for name in sorted(document['artifacts'])],
        output / 'artifacts/SHA256SUMS', '--repo', CONFIG['repository'], '--verify-tag',
        '--title', tag, '--notes-file', notes, *(['--prerelease'] if '-' in version() else ['--latest'])])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=['version', 'ci', 'build', 'assemble', 'release', 'publish', '_container'])
    parser.add_argument('--output', type=Path, default=DEFAULT_OUTPUT)
    parser.add_argument('--target', choices=TARGETS, default=host_target())
    parser.add_argument('--signed', action='store_true', help='Use configured signing credentials for a local build')
    parser.add_argument('--snapshot-commit', help=argparse.SUPPRESS)
    parser.add_argument('--action', choices=['build', 'ci'], default='build', help=argparse.SUPPRESS)
    args = parser.parse_args()
    args.output = args.output.expanduser().resolve()
    current = version()
    if args.command == 'version':
        print(current)
    elif args.command == '_container':
        container_entry(args)
    elif args.command == 'ci':
        if sys.platform != 'darwin':
            raise ValueError('Run CI from macOS; the Linux leg uses OrbStack')
        ci_native(args.output)
        docker(args.output, 'ci', 'x86_64-unknown-linux-gnu')
    elif args.command == 'build':
        if args.signed:
            signing_environment(apple='apple' in args.target)
        if 'linux' in args.target and sys.platform == 'darwin':
            docker(args.output, 'build', args.target, args.signed)
        else:
            build_target(args.output, args.target, args.signed)
    elif args.command == 'assemble':
        assemble(args.output)
    elif args.command == 'release':
        release(args.output, args.snapshot_commit)
    elif args.command == 'publish':
        publish(args.output)


if __name__ == '__main__':
    try:
        main()
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        sys.exit(f'distribution: {error}')
