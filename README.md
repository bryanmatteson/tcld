# tcld

Tcld reads TclPro bytecode (`.tbc`), reconstructs Tcl source, and provides
terminal and desktop tools for inspection and debugging.

## Capabilities

- Decompile bytecode through a multi-pass control-flow pipeline.
- Disassemble instructions with resolved operands.
- Inspect bytecode and source in an interactive terminal explorer.
- Debug Tcl source with stepping, breakpoints, stack frames, variables, watches,
  and event history.
- Run and debug bytecode in the optional in-process TEBC VM.
- Use synchronized analysis views in the Tauri desktop application.
- Parse, highlight, and format Tcl with Tree-sitter.

All application layers consume the same domain snapshot. It includes load
provenance, metadata, instructions, literals, exception ranges, basic blocks,
trace frames, reconstructed source, source mappings, diagnostics, coverage
data, and dump text.

## Requirements

- Rust stable. The repository pins the stable channel in
  `rust-toolchain.toml`.
- Tcl 9 development headers and libraries for native debugger crates, the full
  workspace, and the optional hybrid VM.

Set `TCL9_PREFIX` or `TCL_PREFIX` when Tcl is outside the system search path.
The default CLI build does not link Tcl:

```bash
cargo build -p tcld
```

Build the in-process VM explicitly:

```bash
cargo build -p tcld --features hybrid
```

Host debugging discovers Tcl 9.0, 8.6, or 8.5 at runtime and loads the matching
debugger nub. Release archives include Tcl 8.6 and 9.0 nubs, plus Tcl 8.5 where
the target supports it. [COMPAT.md](COMPAT.md) describes the ABI and packaging
contract.

## Use

```bash
# Reconstruct Tcl source
cargo run -- decompile path/to/file.tbc

# Print bytecode instructions
cargo run -- disassemble path/to/file.tbc

# Inspect exceptions, literals, and instructions
cargo run -- dump path/to/file.tbc

# Report opcode and Tcl-version coverage
cargo run -- coverage report

# Open the terminal explorer
cargo run -- explore path/to/file.tbc

# Render one deterministic explorer frame
cargo run -- explore --dump-screen path/to/file.tbc

# Print source-map spans
cargo run -- source-map path/to/file.tbc
```

Batch decompile and disassemble output uses ANSI color when stdout is an
interactive terminal. Redirected output is plain. Use `--no-colors` or
`NO_COLOR=1` to disable color explicitly:

```bash
cargo run -- --no-colors decompile path/to/file.tbc
NO_COLOR=1 cargo run -- disassemble path/to/file.tbc
```

## Debug and run

Debug Tcl source through a host interpreter:

```bash
cargo run -- debug path/to/script.tcl
cargo run -- debug --tclsh /usr/local/bin/tclsh9.0 path/to/script.tcl
```

Run or debug bytecode in process with the hybrid build:

```bash
cargo run -p tcld --features hybrid -- run path/to/file.tbc
cargo run -p tcld --features hybrid -- run --backend=vm-strict path/to/file.tbc
cargo run -p tcld --features hybrid -- debug path/to/file.tbc
```

`vm-strict` denies WorldHost calls. The regular hybrid backend can reach Tcl
commands such as `load` and `exec`; use the strict backend for untrusted
bytecode.

Host debugging of bytecode remains available when exact bytecode program
counters are not required:

```bash
cargo run -- debug --backend=host path/to/file.tbc
```

This path reconstructs Tcl into a temporary source file before launch. It
requires an exact reconstruction. When reconstruction uses approximations,
diagnostics identify each affected operation, its bytecode range when known,
and the reason.

## Hybrid verification

```bash
# Build, test, PTY, and release gates
scripts/verify-hybrid-gates.sh

# Pinned scenarios, fixture sweep, and JSON report
scripts/verify-hybrid-corpus.sh
```

Both commands require Tcl 9 headers and libraries. A binary built without the
`hybrid` feature reports `hybrid_unavailable` with exit code 3 when a hybrid
backend is requested.

## Stability

During the 0.x series:

- `decompile` and `disassemble` output shapes and process exit codes are stable.
- Coverage TSV columns, dump output, explorer session files, debugger commands,
  and the wire protocol may change between minor releases.

The wire protocol carries its own version in the handshake.

## Architecture

```
tcld/
├── crates/
│   ├── tcld-core/       # Bytecode parser and instruction tables
│   ├── tcld-ir/         # Intermediate representation and control-flow graph
│   ├── tcld-syntax/     # Tcl parsing, extraction, and highlighting
│   ├── tcld-decompile/  # Reconstruction pipeline and source mappings
│   ├── tcld-explore/    # Explorer domain model
│   ├── tcld-tui/        # Generic terminal framework
│   └── tcld-vm/         # Hybrid TEBC runtime
├── apps/
│   ├── tcld/            # Command-line interface
│   ├── tcld-tauri/      # Desktop backend
│   └── tcld-gui/        # Desktop frontend
└── testing/
    ├── fixtures/        # Public compatibility fixtures
    └── fuzz/            # Fuzz targets and seed corpora
```

`tcld-decompile` owns the canonical analysis snapshot. `tcld-explore` derives
artifact capabilities and navigation facts from that snapshot.
`apps/tcld/src/explore` adapts those facts to the generic `tcld-tui`
`DataSource` boundary. Runtime debugging remains a separate `tcld debug`
workflow.

TurtleTap owns terminal attachment, raw mode, resize events, and ticks.
Termosaic owns semantic theme resolution and color-depth adaptation. Tcld owns
its viewport, input model, layouts, palettes, and `ui.*` token vocabulary.

## Development

```bash
# Rust workspace
cargo fmt --all --check
cargo test --workspace

# Explorer render contracts
cargo test -p tcld --test explore_dump_screen_contract

# Terminal lifecycle checks
scripts/explore-pty-smoke.sh
scripts/debug-pty-smoke.sh

# Desktop frontend
cd apps/tcld-gui
npm ci
npm test
npm run build
```

The golden decompiler suite contains 37 bytecode fixtures covering control
flow, exception handling, and procedure reconstruction.

This public branch is a curated reconstruction of private development history.
Author dates retain source provenance; committer dates record the
reconstruction.

## License

Copyright 2026 Bryan Matteson.

Licensed under the [Apache License, Version 2.0](LICENSE). Redistributed TclPro
fixtures retain their original notices; see
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
