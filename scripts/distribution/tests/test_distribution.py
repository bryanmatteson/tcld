import functools
import http.server
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import tarfile
import tempfile
import threading
import unittest
from unittest.mock import patch

DIRECTORY = Path(__file__).resolve().parents[1]


def module(name):
    spec = importlib.util.spec_from_file_location(name, DIRECTORY / (name + '.py'))
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


release = module('release')
installer = module('install')
TARGET = 'aarch64-apple-darwin'


class InstallerTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.prefix = Path(self.temp.name) / 'prefix with spaces'

    def package(self, version='1.0.0', extra=None, target=TARGET):
        name = f'tcld-{version}-{target}'
        files = {'tcld': (f'#!/bin/sh\nprintf "tcld {version}\\n"\n'.encode(), 0o755)}
        for abi in ['tcl8.6', 'tcl9.0']:
            library = b'loadable-extension-' + abi.encode()
            directory = f'lib/tcld/nub/{abi}/'
            library_name = 'libtcld_dbg.' + ('dylib' if 'apple' in target else 'so')
            files[directory + library_name] = (library, 0o755)
            files[directory + 'nub-manifest.json'] = (json.dumps({
                'schema_version': 1, 'wire_protocol_major': 2, 'target_triple': target,
                'tcl_abi': abi, 'library_filename': library_name,
                'sha256': installer.sha256(library),
            }).encode(), 0o644)
        stream = io.BytesIO()
        with tarfile.open(fileobj=stream, mode='w:gz') as archive:
            for path, (data, mode) in files.items():
                info = tarfile.TarInfo(name + '/' + path)
                info.size, info.mode = len(data), mode
                archive.addfile(info, io.BytesIO(data))
            if extra:
                archive.addfile(extra)
        data = stream.getvalue()
        return {'version.txt': version.encode(), 'build-version.txt': version.encode(), name + '.tar.gz': data,
                'SHA256SUMS': f'{installer.sha256(data)}  {name}.tar.gz\n'.encode()}

    def install(self, files, requested=None):
        with patch.object(installer, 'fetch', side_effect=lambda url: files[url.rsplit('/', 1)[1]]):
            return installer.install('https://example.test/download', requested, self.prefix, TARGET)

    def test_upgrade_keeps_both_abis_and_previous_version(self):
        binary = self.install(self.package())
        previous = binary.resolve()
        self.install(self.package('1.0.1'))
        self.assertNotEqual(previous, binary.resolve())
        self.assertTrue(previous.is_file())
        for abi in ['tcl8.6', 'tcl9.0']:
            self.assertTrue((binary.resolve().parent / 'lib/tcld/nub' / abi / 'nub-manifest.json').is_file())
        result = subprocess.check_output([binary, '--version'], text=True)
        self.assertEqual(result.strip(), 'tcld 1.0.1')

    def test_corrupt_upgrade_preserves_current_executable(self):
        binary = self.install(self.package())
        previous = binary.resolve()
        bad = self.package('1.0.1')
        bad['tcld-1.0.1-' + TARGET + '.tar.gz'] += b'corruption'
        with self.assertRaisesRegex(ValueError, 'checksum'):
            self.install(bad)
        self.assertEqual(binary.resolve(), previous)

    def test_promoted_release_downloads_original_build_from_new_release_tag(self):
        files = self.package('0.3.2')
        files['version.txt'] = b'1.0.0'
        requested_urls = []
        def fetch(url):
            requested_urls.append(url)
            return files[url.rsplit('/', 1)[1]]
        for requested in [None, '1.0.0']:
            with patch.object(installer, 'fetch', side_effect=fetch):
                binary = installer.install(installer.DEFAULT_BASE, requested, self.prefix, TARGET)
            self.assertEqual(subprocess.check_output([binary, '--version'], text=True).strip(), 'tcld 0.3.2')
        archive_urls = [url for url in requested_urls if url.endswith('.tar.gz')]
        self.assertEqual(archive_urls, [
            f'https://github.com/bryanmatteson/tcld/releases/download/v1.0.0/tcld-0.3.2-{TARGET}.tar.gz'
        ] * 2)

    def test_refuses_to_replace_user_executable(self):
        binary = self.prefix / 'bin/tcld'
        binary.parent.mkdir(parents=True)
        binary.write_text('user executable')
        with self.assertRaisesRegex(ValueError, 'existing file'):
            self.install(self.package())
        self.assertEqual(binary.read_text(), 'user executable')

    def test_rejects_archive_traversal_and_symlinks(self):
        for path, kind in [('../escaped', tarfile.REGTYPE),
                           (f'tcld-1.0.0-{TARGET}/escape', tarfile.SYMTYPE)]:
            with self.subTest(path=path):
                info = tarfile.TarInfo(path)
                info.type, info.linkname = kind, '/tmp/escape'
                with self.assertRaises(ValueError):
                    self.install(self.package(extra=info))
                self.assertFalse((self.prefix / 'bin/tcld').exists())

    def test_modified_old_install_is_not_reactivated(self):
        files = self.package()
        binary = self.install(files)
        (binary.resolve().parent / 'lib/tcld/nub/tcl9.0/libtcld_dbg.dylib').write_bytes(b'changed')
        with self.assertRaisesRegex(ValueError, 'modified'):
            self.install(files)

    def test_rejects_non_https_download(self):
        with self.assertRaisesRegex(ValueError, 'HTTPS'):
            installer.fetch('http://example.test/archive')

    def test_shell_installer_downloads_and_installs_from_loopback_host(self):
        served = Path(self.temp.name) / 'served'
        served.mkdir()
        for name, data in self.package(target=release.host_target()).items():
            (served / name).write_bytes(data)
        (served / 'install.py').write_bytes((DIRECTORY / 'install.py').read_bytes())
        class Handler(http.server.SimpleHTTPRequestHandler):
            def log_message(self, *_args):
                pass
        server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), functools.partial(Handler, directory=served))
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            subprocess.run(['sh', str(DIRECTORY / 'install.sh'), '--base-url',
                f'http://127.0.0.1:{server.server_port}', '--prefix', str(self.prefix)],
                check=True, capture_output=True, text=True)
            self.assertTrue((self.prefix / 'bin/tcld').is_symlink())
        finally:
            server.shutdown()
            server.server_close()
            thread.join()


class AssemblyTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.output = Path(self.temp.name)
        self.artifacts = self.output / 'artifacts'
        self.artifacts.mkdir()

    def target(self, target, signed=True):
        ver = release.version()
        archive = f'tcld-{ver}-{target}.tar.gz'
        files = {archive: b'cli archive'}
        if 'apple' in target:
            files[f'TCLD-{target}-TCLD.dmg'] = b'dmg'
            updater = f'TCLD-{target}-TCLD.app.tar.gz'
        else:
            updater = f'TCLD-{target}-TCLD.AppImage'
        files[updater] = b'updater payload'
        if signed:
            files[updater + '.sig'] = b'base64-signature'
        for name, data in files.items():
            (self.artifacts / name).write_bytes(data)
        metadata = {'version': ver, 'target': target, 'signed': signed, 'commit': 'a' * 40,
                    'artifacts': {name: release.digest(self.artifacts / name) for name in files}}
        release.write_json(self.output / 'targets' / (target + '.json'), metadata)
        return metadata

    def test_assembly_generates_arch_specific_updates_and_complete_homebrew_install(self):
        self.target(TARGET)
        self.target('x86_64-apple-darwin')
        self.target('x86_64-unknown-linux-gnu')
        release.assemble(self.output, require_signed=True)
        manifest = json.loads((self.artifacts / 'latest.json').read_text())
        self.assertEqual(set(manifest['platforms']), {'darwin-aarch64', 'darwin-x86_64', 'linux-x86_64'})
        for entry in manifest['platforms'].values():
            self.assertTrue((self.artifacts / entry['url'].rsplit('/', 1)[1]).exists())
        formula = (self.output / 'homebrew/Formula/tcld.rb').read_text()
        self.assertIn('(lib/"tcld").install "lib/tcld/nub"', formula)
        self.assertIn('on_arm do', formula)
        self.assertIn('on_intel do', formula)
        self.assertTrue((self.output / 'homebrew/Casks/tcld.rb').is_file())
        checksums = (self.artifacts / 'SHA256SUMS').read_text()
        self.assertIn('install.py', checksums)
        self.assertIn('latest.json', checksums)

    def test_tampering_after_build_blocks_assembly(self):
        metadata = self.target(TARGET)
        name = next(iter(metadata['artifacts']))
        (self.artifacts / name).write_bytes(b'tampered')
        with self.assertRaisesRegex(ValueError, 'changed'):
            release.assemble(self.output)

    def test_homebrew_promotes_build_without_changing_archive_names(self):
        metadata = []
        for target in release.TARGETS:
            item = self.target(target, signed=False)
            item['version'] = '0.3.2'
            item['artifacts'] = {
                name.replace('tcld-1.0.0-', 'tcld-0.3.2-'): sha
                for name, sha in item['artifacts'].items()
            }
            metadata.append(item)
        release.homebrew(self.output, metadata, allow_unsigned=True)
        formula = (self.output / 'homebrew/Formula/tcld.rb').read_text()
        self.assertIn('version "1.0.0"', formula)
        self.assertIn('/v1.0.0/tcld-0.3.2-aarch64-apple-darwin.tar.gz', formula)
        self.assertIn('assert_match "tcld 0.3.2"', formula)
        self.assertTrue((self.output / 'homebrew/Casks/tcld.rb').is_file())
        self.assertIn('unsigned and not notarized', (self.output / 'homebrew/Casks/tcld.rb').read_text())

    def test_unsigned_build_cannot_assemble_as_release(self):
        self.target(TARGET, signed=False)
        with self.assertRaisesRegex(ValueError, 'Unqualified'):
            release.assemble(self.output, require_signed=True)

    def test_publication_checks_tag_before_external_action(self):
        for target in release.TARGETS:
            self.target(target)
        release.assemble(self.output, require_signed=True)
        (self.output / 'release-notes.md').write_text('Notes')
        with patch.object(release, 'run', return_value='b' * 40) as run:
            with self.assertRaisesRegex(ValueError, 'tag must identify'):
                release.publish(self.output)
        self.assertEqual(run.call_count, 1)
        self.assertEqual(run.call_args.args[0][:2], ['git', 'rev-parse'])

    def test_release_exports_committed_source_without_blocking_dirty_shared_tree(self):
        source = self.output / 'checkout'
        source.mkdir()
        def git(*arguments):
            return subprocess.check_output(['git', *arguments], cwd=source, text=True).strip()
        git('init', '-q')
        (source / 'release.txt').write_text('committed release')
        git('add', 'release.txt')
        git('-c', 'user.name=Distribution Test', '-c', 'user.email=test@example.invalid', 'commit', '-qm', 'Release source')
        commit = git('rev-parse', 'HEAD')
        (source / 'release.txt').write_text('ongoing unrelated work')
        (source / '.claude').mkdir()
        (source / '.claude/local').write_text('private local state')
        commands = []
        def run(arguments, **kwargs):
            commands.append(arguments)
            if arguments[0] == 'git':
                return git(*arguments[1:])
        with patch.object(release, 'ROOT', source), patch.object(release, 'signing_environment'), \
             patch.object(release, 'run', side_effect=run):
            release.release(self.output / 'prepared')
        snapshot = self.output / 'prepared/source' / commit
        self.assertEqual((snapshot / 'release.txt').read_text(), 'committed release')
        self.assertFalse((snapshot / '.claude').exists())
        self.assertEqual((source / 'release.txt').read_text(), 'ongoing unrelated work')
        self.assertEqual(commands[-1][-2:], ['--snapshot-commit', commit])

    def test_orbstack_uses_explicit_context_and_does_not_forward_apple_credentials(self):
        with patch.object(release.sys, 'platform', 'darwin'), \
             patch.dict(os.environ, {'APPLE_PASSWORD': 'private', 'TCLD_UPDATER_PUBLIC_KEY': 'public'}), \
             patch.object(release, 'run') as run:
            release.docker(self.output, 'build', 'x86_64-unknown-linux-gnu')
        commands = [call.args[0] for call in run.call_args_list]
        self.assertEqual(commands[0][:3], ['docker', '--context', 'orbstack'])
        self.assertIn('--rm', commands[1])
        self.assertIn('TCLD_UPDATER_PUBLIC_KEY', commands[1])
        self.assertNotIn('APPLE_PASSWORD', commands[1])
        self.assertNotIn('private', ' '.join(map(str, commands[1])))

    def test_mismatched_version_stops_before_build(self):
        source = self.output / 'source'
        for name in ['Cargo.toml', 'apps/tcld-gui/package.json', 'apps/tcld-gui/package-lock.json', 'apps/tcld-tauri/tauri.conf.json']:
            destination = source / name
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes((release.ROOT / name).read_bytes())
        path = source / 'apps/tcld-tauri/tauri.conf.json'
        data = json.loads(path.read_text())
        data['version'] = '99.0.0'
        path.write_text(json.dumps(data))
        with patch.object(release, 'ROOT', source), self.assertRaisesRegex(ValueError, 'Version mismatch'):
            release.version()


if __name__ == '__main__':
    unittest.main()
