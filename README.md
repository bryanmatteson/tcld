<p align="center"><img src="docs/assets/icon.svg" width="112" alt="Tcld icon"></p>
<h1 align="center">Tcld</h1>
<p align="center"><strong>Recover the source. Follow the bytecode. Understand the program.</strong></p>

[![Release](https://img.shields.io/github/v/release/bryanmatteson/tcld)](https://github.com/bryanmatteson/tcld/releases/latest)
[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

Tcld is a decompiler, explorer, and debugger for Tcl software. Open TclPro
`.tbc` files, recover readable Tcl, inspect instructions and control flow, and
follow execution in a native desktop workbench, a terminal explorer, or the CLI.

TclPro made it possible to distribute compiled Tcl without the original source.
Tcld helps you understand those programs again, with linked views of the
bytecode, reconstructed commands, symbols, and execution state.

<p align="center"><img src="docs/assets/workbench.gif" width="1100" alt="Tcld desktop workbench showing linked disassembly, reconstructed source, symbols, and an instruction inspector"></p>

A native session with the bundled public TclPro fixture. Selecting an
instruction follows its reconstructed source and updates the inspector.
[Media and capture scripts](docs/assets/README.md).

[Install](#install) · [Quick start](#quick-start) · [Desktop](#desktop-workbench) ·
[Debugging](#debugging-and-execution) · [Build](#build-from-source) ·
[Development](#development) · [Documentation](#documentation)

## Install

### Homebrew

Install the CLI, the desktop app, or both:

```sh
brew install bryanmatteson/tap/tcld
brew install --cask bryanmatteson/tap/tcld
```

The CLI formula supports macOS Apple Silicon, macOS Intel, and Linux x86-64.
The desktop cask installs `TCLD.app` on macOS. Desktop packages include a private,
patched Tcl 9 runtime; you do not need to install Tcl separately to use them.
The CLI package includes Tcl 8.6 and Tcl 9 debugger libraries; host debugging
also requires a matching `tclsh` on your machine.

Upgrade a Homebrew installation with:

```sh
brew update
brew upgrade --formula bryanmatteson/tap/tcld
brew upgrade --cask bryanmatteson/tap/tcld
```

### Direct downloads

[GitHub Releases](https://github.com/bryanmatteson/tcld/releases/latest) provides
these packages:

| Platform | CLI | Desktop | Minimum system |
| --- | --- | --- | --- |
| macOS Apple Silicon | `.tar.gz` | `.dmg` | macOS 11 |
| macOS Intel | `.tar.gz` | `.dmg` | macOS 11 |
| Linux x86-64 | `.tar.gz` | `.AppImage`, `.deb` | Debian 12 / glibc 2.36 baseline |

The macOS CLI and desktop packages are signed with Developer ID and notarized
by Apple. The apps and DMGs have stapled notarization tickets. Windows packages
are not currently distributed.

For the CLI, the download installer verifies the archive and debugger-library
checksums, keeps the complete package together, and installs under `~/.local`:

```sh
curl -fsSLO https://github.com/bryanmatteson/tcld/releases/latest/download/install.sh
sh install.sh
```

Python 3.9 or later is required. Add `~/.local/bin` to your `PATH` if needed.
Use `sh install.sh --version 1.0.1` to select a release or
`sh install.sh --prefix /path/to/prefix` to choose an installation directory.
If you unpack a CLI archive manually, keep `tcld` beside its `lib/tcld/nub`
directory so the debugger can find its libraries.

**Release and build versions:** 1.0.1 packages report `1.0.1` in the CLI and
desktop About dialog. The earlier 1.0.0 release reused build 0.3.2; those older
executables and archive names retain `0.3.2`. See the
[1.0.0 release notes](https://github.com/bryanmatteson/tcld/releases/tag/v1.0.0)
for that publication's reuse details.

### Updates

The published 1.0.0 apps do not contain an in-app updater. Upgrade those
installations through Homebrew or a new download.

Desktop releases starting with 1.0.1 enable **Help → Check for Updates…**.
Checks happen when requested; installing an update and restarting require
confirmation. Update payloads use a persistent Tauri signing key, separate from
Apple code signing. Existing 1.0.0 users need to install 1.0.1 once through
Homebrew or a direct download to enable future in-app updates.
Linux in-app installation is limited to AppImages; Debian
packages continue to use the package manager. See [Distribution](DISTRIBUTION.md)
for release preparation and signing.

## Quick start

From a checkout, try the public fixture:

```sh
tcld decompile testing/fixtures/public/public_complex_fixture.tbc
```

For your own files:

```sh
tcld decompile path/to/file.tbc     # reconstruct Tcl source
tcld disassemble path/to/file.tbc   # list instructions and operands
tcld dump path/to/file.tbc          # inspect literals, exceptions, and instructions
tcld source-map path/to/file.tbc    # inspect command source mappings
tcld explore path/to/file.tbc       # open the terminal explorer
tcld coverage report               # report opcode and Tcl-version coverage
```

Output uses ANSI color when stdout is a terminal and plain text when redirected.
Use `--no-colors` or `NO_COLOR=1` to disable color explicitly. Run
`tcld --help` or `tcld <command> --help` for command options.

## What Tcld can do

- **Recover and inspect.** Parse bytecode from Tcl 8.0–8.6 and 9.0, resolve operands and
  literals, reconstruct procedures and control flow, and inspect recovery
  diagnostics. Parser support, decompiler recovery, and runtime execution have
  different compatibility boundaries; see [Compatibility](COMPAT.md).
- **Navigate a program.** Follow instructions, recovered source, original source
  mappings, symbols, literals, exceptions, graphs, and inspector details through
  linked selections. Filter listings, fold command groups, follow jump targets,
  keep bookmarks, and use navigation history.
- **Arrange the workbench.** Open several files, resize panels and tab stacks,
  switch analysis and debugging layouts, and save layouts. Menus, keyboard
  shortcuts, and a command palette expose the same actions.
- **Inspect reconstruction.** Compare available original source with recovered
  commands and their instruction spans. Examine basic blocks, edges, loops,
  stack effects, exception ranges, and pipeline traces.
- **Debug source and bytecode.** Set breakpoints, step into/over/out, inspect
  frames and variables, evaluate expressions, and keep watches. Named run
  configurations retain the backend, arguments, working directory, and environment.
- **Automate analysis.** Use the CLI to export decompiled source, disassembly,
  source maps, dumps, and coverage reports. Optional hybrid builds also run
  bytecode in process.

Recovery is structural: procedures, loops, `switch`, exception handling, and
namespaces are reconstructed where the bytecode provides enough information.
Diagnostics record approximations and unsupported operations. Recovered source
is an analysis result; consult the diagnostics when exact behavior matters.

## Desktop workbench

Disassembly, Source View, and Inspector form the main investigation loop.
Grouped instruction rails connect recovered commands to bytecode program
counters. Selecting a symbol or instruction brings its source and metadata into
view; jump arrows and control-flow graphs expose the relationships between blocks.

![Tcld desktop workbench with linked source, disassembly, and instruction details](docs/assets/workbench.jpg)

Symbols organizes procedures, namespaces, and variables. Compare relates
available source to recovered commands. Specialized layouts bring graphs,
exceptions, diagnostics, traces, or debugger controls forward.

The app opens `.tbc` artifacts, Tcl bytecode wrappers, and Tcl source accepted by
the shared source compiler. Opening source compiles it for analysis without
executing it; unsupported forms produce a compiler diagnostic. The bundled
examples pair original source with compiled fixtures. Desktop distributions
include the hybrid execution engine and debugger resources.

### Terminal explorer

```sh
tcld explore path/to/file.tbc
```

The terminal explorer links instructions, literals, exceptions, reconstructed
source, and source maps. Its inspector follows the cursor as you move between
views.

![Tcld terminal explorer](docs/demo.gif)

Recorded with VHS; see the [capture script](docs/demo.tape).

## Debugging and execution

### Host Tcl debugging

The distributed CLI can debug source through an external Tcl interpreter:

```sh
tcld debug path/to/script.tcl
tcld debug --tclsh /path/to/tclsh9.0 path/to/script.tcl
```

Tcld selects a matching debugger library for the discovered interpreter. Release
packages include Tcl 8.6 and Tcl 9 libraries. Tcl 8.5 command-level debugging is
available with a separately built matching library; it is not included in the
current release packages. Instruction stepping through the host debugger
requires a runtime with the Tcld opcode patch.

To debug bytecode through host Tcl, request reconstruction explicitly:

```sh
tcld debug --backend=host path/to/file.tbc
```

This path requires exact reconstruction and debugs the resulting source. Direct
bytecode debugging uses the hybrid engine instead.

### Hybrid execution

The desktop packages include the hybrid engine. For a CLI with the same runtime
capability, build with the `hybrid` feature and Tcl 9 development libraries:

```sh
cargo build --release -p tcld --features hybrid
./target/release/tcld run path/to/file.tbc
./target/release/tcld run --backend=vm-strict path/to/file.tbc
./target/release/tcld debug path/to/file.tbc
./target/release/tcld debug --backend=hybrid path/to/script.tcl
```

The hybrid backend can call into embedded Tcl 9. `vm-strict` denies those host
calls and reports unsupported operations instead of falling back. Source
compilation has its own admission checks. See [Debugging](DEBUGGING.md) for
backend selection, source mapping, stepping, dynamic code, coroutine support,
and diagnostics.

The downloadable CLI is the Tcl-independent analysis and host-debugger build.
It does not contain the hybrid engine or desktop UI. A requested VM backend
returns `hybrid_unavailable` with exit code 3; `tcld ui` returns `ui_unavailable`
with exit code 3 when the UI feature is absent. Install the desktop package or
use the source-build profiles below for those capabilities.

## Build from source

The repository pins Rust **1.97.1** in `rust-toolchain.toml`. Rustup selects it
when you work in the checkout. Desktop frontend builds use Node.js 22 or later
and npm; the distribution runner pins its own tooling in
[scripts/distribution/config.toml](scripts/distribution/config.toml).

### CLI

The default CLI has no compile-time Tcl dependency:

```sh
cargo build --locked --release -p tcld
./target/release/tcld decompile testing/fixtures/public/public_complex_fixture.tbc
```

A raw Cargo build installs neither the executable nor its debugger libraries.
For a checkout installation with the Tcl 9 loadable debugger library, use
`mise run tcld:install` or `scripts/install-tcld.sh`. That path needs Tcl 9
headers and installs under `~/.cargo` by default. A plain `cargo install` installs
only the executable; use a release package for the complete prebuilt CLI.

### Desktop and embedded UI

Install your platform's Tauri development prerequisites and Tcl 9 headers and
libraries. Set `TCL9_PREFIX` when Tcl is outside the normal search paths. The
[patched runtime](third_party/tcl/README.md) enables host instruction stepping;
the distribution runner builds and packages it automatically.

Build a standalone desktop executable with an embedded frontend:

```sh
npm ci --prefix apps/tcld-gui
npm run build --prefix apps/tcld-gui
cargo build --locked --release -p tcld-tauri --features custom-protocol,hybrid,updater
./target/release/tcld-tauri path/to/file.tbc
```

To host the workbench inside the CLI process instead:

```sh
cargo build --locked --release -p tcld --features native-ui,hybrid
./target/release/tcld ui path/to/file.tbc
```

The file argument is optional. With [mise](https://mise.jdx.dev),
`mise run tcld:build-ui` builds the embedded UI in debug mode, and
`mise run tcld:ui [FILE]` builds and opens it. These convenience tasks enable
`native-ui`; add `hybrid` explicitly when you need in-process bytecode execution.
Developer binaries use your configured Tcl installation. Use the
[distribution runner](DISTRIBUTION.md) for relocatable packages with a private
runtime, debugger resources, signing, and notarization.

## Architecture and compatibility

The CLI, terminal explorer, desktop, and debugger consume a shared analysis
snapshot: load provenance, instructions, literals, exception ranges, basic
blocks, reconstructed source, mappings, diagnostics, and coverage. The views
refer to the same program and bytecode identities.

```text
tcld/
├── apps/
│   ├── tcld/             # CLI, terminal explorer, and debugger launcher
│   ├── tcld-tauri/       # Native desktop backend
│   └── tcld-gui/         # Desktop frontend
├── crates/
│   ├── tcld-core/        # Bytecode parser and opcode tables
│   ├── tcld-ir/          # Intermediate representation and control flow
│   ├── tcld-syntax/      # Tcl parsing and highlighting
│   ├── tcld-compile/     # Source compilation and admission
│   ├── tcld-decompile/   # Reconstruction and canonical analysis snapshot
│   ├── tcld-explore/     # Explorer model
│   ├── tcld-tui/         # Terminal framework
│   ├── tcld-vm/          # In-process VM and Tcl host integration
│   ├── tcld-debug-*/     # Debugger state and wire protocol
│   ├── tcld-dbg-ext/     # Loadable host debugger library
│   └── tcld-interp/      # Embedded Tcl interpreter
├── scripts/distribution/ # Explicit local CI and packaging
├── third_party/tcl/      # Tcl runtime patch
└── testing/              # Fixtures and fuzz targets
```

Compatibility is tracked by opcode and Tcl version. `tcld coverage report`
distinguishes golden-tested reconstruction, structural recovery, and rendering.
Execution support is tracked separately. Tcl ABI, debugger protocol version,
target architecture, and library checksums are checked before loading a nub.

Golden fixtures, runtime-oracle comparisons, debugger integration checks, PTY
smoke tests, and fuzz targets cover different parts of the system. Their
presence is not a claim that every Tcl program can be reconstructed or executed.
For detailed runtime and ABI support, see [COMPAT.md](COMPAT.md).

## Development

Run focused checks for the area you change. These commands are explicit and do
not install background jobs or Git hooks:

```sh
# Rust formatting and workspace tests
cargo fmt --all -- --check
cargo test --locked --workspace

# Terminal rendering and lifecycle
cargo test --locked -p tcld --test explore_dump_screen_contract
scripts/explore-pty-smoke.sh
scripts/debug-pty-smoke.sh

# Hybrid runtime checks; require Tcl 9 development libraries
scripts/verify-hybrid-gates.sh
scripts/verify-hybrid-corpus.sh

# Desktop frontend
npm ci --prefix apps/tcld-gui
npm run lint --prefix apps/tcld-gui
npm test --prefix apps/tcld-gui
npm run build --prefix apps/tcld-gui

# Distribution tooling
python3 -m unittest discover -s scripts/distribution/tests -v
```

Full CI runs **on the maintainer's machine**, with Linux checks in OrbStack,
only when requested or before a release. GitHub hosts source and release assets;
there are no GitHub Actions workflows. To invoke the local CI runner explicitly:

```sh
python3 scripts/distribution/release.py ci
```

Packaging and release instructions, signing-key setup, artifact locations, and
publication commands are in [DISTRIBUTION.md](DISTRIBUTION.md).

## Documentation

| Guide | Contents |
| --- | --- |
| [Debugging](DEBUGGING.md) | Source and bytecode workflows, backends, stepping, and diagnostics |
| [Compatibility](COMPAT.md) | Tcl versions, runtime modes, debugger ABIs, and protocol contracts |
| [Distribution](DISTRIBUTION.md) | Local CI, packaging, signing, notarization, and publication |
| [Tcl runtime patch](third_party/tcl/README.md) | Opcode stepping hooks and the patched runtime build |
| [Media](docs/assets/README.md) | Native workbench screenshots and capture recipes |
| [Third-party notices](THIRD_PARTY_NOTICES.md) | Redistributed fixture and dependency notices |

## License

Copyright 2026 Bryan Matteson. Licensed under the
[Apache License, Version 2.0](LICENSE). Redistributed TclPro fixtures and Tcl
components retain their own notices; see [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
