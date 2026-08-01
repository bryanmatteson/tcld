# Debugging Tcl source and bytecode

`tcld debug` accepts Tcl source (`.tcl`, `.tk`, `.tm`, and other non-`.tbc`
files) as well as TclPro bytecode (`.tbc`). The simplest source workflow uses
the host debugger:

```bash
tcld debug app.tcl
tcld debug --backend=host app.tcl
```

`auto` is the default. For source it means `host`: tcld executes the original
file in a discovered `tclsh` with the loadable debugger nub. Breakpoints and
stopped locations keep the source file's canonical identity. Override runtime
discovery with `--tclsh` and `--dylib` when needed.

For a checkout-based installation, use `mise run tcld:install`. Cargo installs
the CLI executable but does not install loadable-extension artifacts by itself;
the task therefore builds a fresh Tcl 9 nub, writes its ABI/protocol/checksum
manifest, and installs both under the same root. This prevents a newly updated
CLI from pairing with a stale nub left in `target/`.

## Compiled source debugging

A binary built with the `hybrid` feature can compile admitted source into one
verified in-process debug image:

```bash
cargo build -p tcld --features hybrid
target/debug/tcld debug --backend=hybrid app.tcl
target/debug/tcld debug --backend=vm-strict app.tcl
```

`hybrid` permits opaque leaf calls through Tcl 9 WorldHost. `vm-strict` denies
WorldHost calls. Both preserve compiler-owned source-to-PC maps, including
procedure code-unit identity, so source breakpoints, instruction breakpoints,
stepping, stack frames, and source/disassembly navigation refer to the same
image.

Source admission is the bounded `tcld.source-compiler/v1.1-tcl86` contract.
Known forms that evaluate or load scripts outside the image, such as dynamic
`eval`, `namespace eval`, `interp eval`, and `source`, are rejected until their
bodies can be compiled and mapped. A syntax or admission failure happens before
the terminal opens and is structured for scripts and CI:

```text
source_compile_failed: /path/app.tcl:LINE:COLUMN: REASON; command="..."; use --backend=host
```

Explicit `hybrid` and `vm-strict` requests never fall back silently. Retry with
`--backend=host` when normal Tcl runtime behavior matters more than verified
instruction mapping. A binary without the feature instead reports
`hybrid_unavailable` with exit code 3 and build guidance.

## TclPro bytecode

In a hybrid build, `auto`, `hybrid`, and `vm-strict` debug raw `.tbc` directly;
they never decompile to a temporary script. Raw bytecode may expose nearby Tcl
source only as an unverified companion view. It is not promoted to canonical
source and cannot justify source breakpoints without a verified map.

```bash
tcld debug program.tbc
tcld debug --backend=vm-strict program.tbc
```

`--backend=host program.tbc` is intentionally different: it requires an exact
reconstruction, owns that temporary source for the whole session, and does not
preserve bytecode PCs. If exact reconstruction fails, tcld refuses to execute
an approximation and suggests direct hybrid debugging or `tcld explore`.

## Desktop application

The native desktop debugger accepts Tcl source and bytecode targets. Its run
configuration labels the execution backend as the Tcl host debugger and lets
you choose only the host transport: an embedded Tcl host or an external
`tclsh` nub over the wire protocol. It does not pretend to embed the terminal
debugger or duplicate the source compiler policy. Use a hybrid-capable CLI
build for explicit Hybrid/VmStrict source-image debugging.

Paths are passed as structured arguments, so targets whose filenames contain
spaces do not require shell escaping inside the desktop application. At a
shell, quote the path normally: `tcld debug "My Scripts/app.tcl"`.
