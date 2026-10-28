# Distribution

Tcld builds and validates releases on the maintainer's Mac. Linux runs in Docker
containers through **OrbStack's `orbstack` Docker context**. There are no GitHub
Actions, Git hooks, background jobs, or push/tag triggers. Run full CI only when
requested or as part of an actual release. Normal development can use focused
tests without invoking the CI command.

All commands below run from the repository root and require Python 3.12+.
Artifacts default to `~/Documents/Developer/tcld/distribution`; `--output DIR`
selects a different directory. Source, compiler outputs, logs, and final packages
belong outside the shared checkout. No Git worktrees are created.

## Products

The 1.0.0 publication promotes the previously validated 0.3.2 builds without
recompilation. Its Homebrew release version is 1.0.0; the binaries and desktop
About dialog retain build version 0.3.2. Release downloads include `version.txt`
for the publication version and `build-version.txt` for the archive/executable
version. The installer checks the original build while downloading everything
from the selected release tag. These promoted desktop packages are unsigned
development builds, without notarization or in-app updater support. This is an
explicit reuse exception for 1.0.0; newly built releases use the signing flow below.

- CLI: Tcl-independent `tcld`, terminal explorer, and Tcl 8.6/9 loadable debugger
  extensions, with manifests under `lib/tcld/nub`. Host debugging needs the
  corresponding host `tclsh`. Hybrid execution is a desktop/source-build feature.
- Desktop: `TCLD.app` in macOS DMGs, Linux AppImages and Debian packages. Includes
  the hybrid engine, patched Tcl 9.0.2 shared library, Tcl script library, Tcl
  8.6/9 debugger extensions, and redistribution notices. No Homebrew Tcl runtime
  is required on the user's computer. External debugging can still use the
  user's selected interpreter.
- Platforms: macOS Apple Silicon and Intel (minimum macOS 11), Linux x86-64
  (Debian 12/glibc 2.36 baseline). The configuration controls the release matrix.
  Intel macOS executables must run successfully under Rosetta on an ARM Mac;
  install Rosetta before preparing that target. Linux x86-64 uses OrbStack's
  emulation on an ARM Mac. Windows is not a qualified distribution target.

## Prerequisites

Install OrbStack, enable its Docker engine, and make `docker --context orbstack`
available. Native macOS builds need Xcode command-line tools, rustup, Node 22+,
and the Tauri CLI and cargo-deny versions in `scripts/distribution/config.toml`:

```sh
cargo install tauri-cli --version 2.11.4 --locked
cargo install cargo-deny --version 0.19.9 --locked
```

Publishing additionally requires an authenticated GitHub CLI (`gh`).

The runner installs the configured Rust toolchain explicitly. It builds pinned,
checksum-verified Tcl 8.6 and Tcl 9 source, applying the repository's opcode patch
to Tcl 9. It does not modify your Homebrew Tcl. The Linux image supplies the system packages, Node,
Rust, Tauri CLI, dependency-audit tooling, and a separate compilation cache.
The Debian 12 base is pinned by image digest and deliberately sets the
compatibility baseline instead of following a floating latest OS.

## Explicit local CI

```sh
python3 scripts/distribution/release.py ci
# equivalent: mise run tcld:ci
```

Runs formatting, Clippy, workspace tests, desktop hybrid/updater checks, hybrid
runtime and corpus checks, debugger integration with Tcl 8.6/9, frontend lint,
types, browser/unit tests, and dependency advisories on macOS and Linux. The
patched-runtime tests are included in the debugger test run. Miri is a separate
optional developer diagnostic, not a routine release prerequisite. An error
stops the command and prevents release packaging.

## Local package build

```sh
python3 scripts/distribution/release.py build
python3 scripts/distribution/release.py build --target x86_64-apple-darwin
python3 scripts/distribution/release.py build --target x86_64-unknown-linux-gnu
python3 scripts/distribution/release.py assemble
```

`build` performs package-specific verification, not full CI. macOS runs natively;
Linux starts only the requested OrbStack container. Local builds accept a dirty
checkout and produce unsigned development installers without updater support.
They cannot be published using the normal release command. New official builds
use the signed release flow; the explicitly authorized 1.0.0 reuse above is an
exception.

Verification unpacks CLI archives into a new directory, runs public-fixture
analysis and real host-debugger/PTY checks, and rejects build-machine Tcl links.
Desktop verification copies/extracts each package and runs the installed binary's
`--distribution-smoke-test` with Tcl/build environment variables cleared and a
bogus `TCL_LIBRARY`. It checks packaged Tcl initialization, analysis, and hybrid
execution. Signed macOS apps also pass strict signature, Gatekeeper, and stapling
checks. Each installed desktop package also opens the real webview with the public
bytecode fixture and waits for rendered instructions and a successful backend
load before exiting. Linux uses Xvfb. Interactive GUI behavior is additionally
covered by the explicit CI frontend tests.

## Signing and updates

Use your existing Developer ID Application identity in the macOS keychain. The
runner does not import certificates or manage the keychain. Configure locally:

```sh
export APPLE_SIGNING_IDENTITY='Developer ID Application: ...'
export APPLE_ID='...'
export APPLE_TEAM_ID='...'
# Set APPLE_PASSWORD privately to an app-specific password.
```

Alternatively set `APPLE_API_ISSUER`, `APPLE_API_KEY`, and `APPLE_API_KEY_PATH`
for Tauri's notarization flow. Keep all credentials outside the repository.

Generate and back up a persistent Tauri update key once:

```sh
cargo tauri signer generate -w "$HOME/.config/tcld/updater.key"
export TAURI_SIGNING_PRIVATE_KEY="$HOME/.config/tcld/updater.key"
export TCLD_UPDATER_PUBLIC_KEY="$(cat "$HOME/.config/tcld/updater.key.pub")"
# Set TAURI_SIGNING_PRIVATE_KEY_PASSWORD privately if the key is encrypted.
```

The public key is embedded in signed desktop builds. The private key signs the
update artifacts; it is distinct from Apple code signing. Linux containers
receive only the updater signing environment, not Apple credentials. **Help →
Check for Updates** checks a static `latest.json` in GitHub Releases. Installation
and restart require the user's confirmation. There are no background update
checks. Preserve this key across releases or installed apps cannot authenticate
future updates. Debian users should upgrade using their package manager;
Tauri's Linux in-app updater supports AppImage installations.

## Actual release

Update the Cargo workspace version, frontend `package.json` and lockfile root
versions, and Tauri version together. `version` checks their consistency:

```sh
python3 scripts/distribution/release.py version
python3 scripts/distribution/release.py release --output "$HOME/Documents/Developer/tcld/releases/1.0.0"
```

Commit the intended release source before `release`. It exports `HEAD` to an isolated
source directory under the output, then runs native and OrbStack CI and all
configured target builds. This binds packages to one source identity while the
shared checkout remains available for other work. Uncommitted changes and local
`.claude` state are excluded; a dirty shared checkout does not block the export.
The command never pushes, tags, publishes, or writes a tap repository.
Use a fresh output directory for each attempt at an actual release.

The result contains:

- `artifacts/`: CLI archives, DMGs, AppImages, `.deb` files, signed update bundles,
  `latest.json`, download installers, version file, and `SHA256SUMS`.
- `targets/` and `release.json`: source identity and verified artifact digests.
- `homebrew/Formula/tcld.rb` and `homebrew/Casks/tcld.rb`: generated from the actual
  package digests, ready to copy into the configured tap.

Publication is an explicit external action. Once authorized, create/push the
annotated version tag at the source commit, write `release-notes.md` in the output
directory, and run:

```sh
python3 scripts/distribution/release.py publish --output "$HOME/Documents/Developer/tcld/releases/1.0.0"
```

This verifies the tag/source match, complete target matrix, signing status, and
artifact digests before calling `gh release create --verify-tag`. GitHub hosts
assets; it does not build or run CI. Copy the generated Homebrew definitions into
`bryanmatteson/homebrew-tap` and publish that repository separately when authorized.

After the assets and tap are published, users can install with:

```sh
brew install bryanmatteson/tap/tcld
brew install --cask bryanmatteson/tap/tcld
```

Linux/macOS archive users can download `install.sh` from the release and run it
with `--version 1.0.0` and/or `--prefix DIR`. The installer verifies the archive
and nub checksums, smoke-tests the executable, preserves the complete package in
a versioned directory, and atomically switches `bin/tcld`. It refuses unrelated
existing executables and keeps old versions available. An alternate artifact
host is supported with `--base-url URL` or `TCLD_RELEASE_BASE_URL`; HTTP is allowed
only on loopback for installer testing. Python 3.9+ is required on the user's
machine. Homebrew manages updates for its own installations.
