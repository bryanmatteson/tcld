# Tcl Compatibility

For the user-facing source and bytecode debugger workflows, backend choices,
and failure behavior, see [DEBUGGING.md](DEBUGGING.md).

## Supported runtimes

The debugger nub supports Tcl 8.5, 8.6, and 9.0 runtime ABIs.

| Surface | Tcl 9.0 | Tcl 8.6 | Tcl 8.5 |
|---|---:|---:|---:|
| Workspace unit tests | verified | verified | adapter and discovery |
| Command stepping | verified | verified | verified |
| Instruction stepping | patched runtime | patched runtime | unavailable |
| Tier 1 wrappers | verified | verified with test constraint | limited |
| Tier 2 surface | verified | verified | command-level |
| Tier 3 features | verified | verified | limited |
| TclPro parity | verified | verified | limited |
| Wire loopback | verified | verified | limited |
| External `tclsh` wire tests | verified | verified | when available |

The Tcl 8.6 external-process suite covers Tier 0–3 operations, including
conditional breakpoints, variables, watch expressions, breakpoint persistence,
ignore counts, and pause/resume events.

Tcl 8.5 supports command-level debugging. It does not provide instruction
stepping or opcode profiling. Shell discovery prefers 8.6 for Tcl 8 bytecode
and uses 8.5 when 8.6 is not present.

## Building a debugger nub

`mise run tcld:install` is the supported checkout installation path. It
installs both the CLI and a manifested Tcl 9 nub. A raw `cargo install --path
apps/tcld` installs only the executable because Cargo does not install cdylib
artifacts.

Cargo features `tcl8_5`, `tcl8_6`, and `tcl9` select the ABI and are mutually
exclusive. `tcl9` is the default. Prefix variables select non-system
installations.

```bash
# Tcl 8.5
cargo build -p tcld-dbg-ext --no-default-features \
  --features tcl8_5,loadable-extension

# Tcl 8.6
TCL8_6_PREFIX=/opt/homebrew/opt/tcl-tk@8 \
  cargo build -p tcld-dbg-ext --no-default-features --features tcl8_6

# Tcl 9.0
cargo build -p tcld-dbg-ext
```

On macOS, an 8.5 loadable nub needs only the SDK headers.
`tcl-sys` discovers `Tcl.framework/Versions/8.5/Headers`, allowing the result to
load in `/usr/bin/tclsh` 8.5.9 without a separate Tcl 8.5 installation.

## Tcl 8.6 test constraint

Tcl 8.6 can finalize its per-thread notifier after repeated
`Tcl_DeleteInterp` cycles in a single long-running test process. Later
`update` or `vwait` calls then report `Tcl_WaitForEvent: Notifier not
initialized`.

This affects the Tier 1 in-process test pool. It does not affect the normal
debugger topology, where one `tclsh` process owns one long-lived interpreter.
The test harness retains an anchor interpreter to reduce notifier churn. The
external-process suite provides the runtime verification for Tcl 8.6.

## Tcl 8.4

Tcl 8.4 and earlier are not supported. The debugger relies on APIs introduced
in Tcl 8.5:

- `info frame` for source locations and breakpoint targets;
- `apply` for anonymous command bodies;
- `dict` for bootstrap state and event payloads.

The 32-bit object-size API also differs from the supported ABI lines.

## Patchlevel selection

Runtime discovery probes `info patchlevel` and selects by major and minor
version:

| Patchlevel | ABI |
|---|---|
| `8.5.x` | Tcl 8.5 |
| `8.6.x` | Tcl 8.6 |
| `9.x.y` | Tcl 9 |

The full patchlevel is reported in diagnostics but does not change nub
selection. There is no minimum patchlevel within a supported ABI.

## Hybrid TEBC VM

The hybrid VM is an optional Tcl 9-linked feature:

```bash
cargo build -p tcld --features hybrid
scripts/verify-hybrid-gates.sh
```

Set `TCL9_PREFIX` or `TCL_PREFIX` when Tcl is outside the default search path.
The hybrid VM embeds Tcl 9 for WorldHost calls. It does not load multiple Tcl
ABIs into one process.

| Command | Behavior |
|---|---|
| `tcld run path.tbc` | Runs bytecode in the in-process VM |
| `tcld run --backend=vm-strict path.tbc` | Runs the VM with WorldHost denied |
| `tcld run --sandbox path.tbc` | Alias for `vm-strict` |
| `tcld run --backend=host path.tcl` | Runs source in a host interpreter |
| `tcld debug path.tbc` | Uses the hybrid debugger when linked |
| `tcld debug --backend=host path.tbc` | Reconstructs source and uses the wire nub |
| Binary without `hybrid` | Returns `hybrid_unavailable` with exit code 3 for a requested VM backend |

Tcl source is also a first-class debug target. `auto` and `host` execute the
original source in the host debugger. In a hybrid build, explicit `hybrid` and
`vm-strict` compile admitted source into a verified in-process debug image.

The downloadable CLI archive is the host-debugger distribution: it has no
compile-time Tcl dependency and includes the versioned loadable nubs. The
hybrid VM is a separate source-build profile because it links Tcl 9 in process.
The release-bundle verifier rejects a compile-time Tcl dependency, so an
archive never claims to contain Hybrid/VmStrict; requesting either reports the
capability failure above with rebuild guidance.

The hybrid debugger supports nested step, over, and out; code-qualified program
counter breakpoints; selected-frame variables; opcode coverage; synchronized
disassembly and source; and HostFrame-backed evaluation and watches.

Both versioned opcode tables have explicit dispositions:

| Table | VM | HostValue | HostFrame | Trap |
|---|---:|---:|---:|---:|
| Tcl 8.6 | 139 | 34 | 2 | 15 |
| Tcl 9.0 | 139 | 33 | 2 | 17 |

Traps cover Tcl 9 constant opcodes, object-system operations,
yield/coroutine behavior, and selected introspection. They return
`UnimplementedOpcode` with exit code 2.

The corpus report separates static opcode disposition from completed runtime
coverage. It runs 27 named scenarios, four procedure bodies from the public
complex fixture, and all 37 application fixtures. Remaining semantic boundaries
are pinned to specific errors.

The regular hybrid backend can call Tcl commands such as `load` and `exec`.
Use `--sandbox` or `vm-strict` for untrusted bytecode.

Legacy Tcl 8 bytecode sometimes expects unqualified access to common globals.
The host bootstrap supplies that namespace behavior under Tcl 9. Each VM frame
also owns an immutable namespace for procedure, WorldHost, and variable
resolution. HostFrame evaluation mirrors scalars, arrays, links, unsets, new
locals, and Tcl completion codes.

Verify the hybrid path with:

```bash
scripts/verify-hybrid-gates.sh

# Individual gates
cargo build -p tcld-vm --features host,debug
cargo build -p tcld --features hybrid
cargo test -p tcld-vm --features host,debug
cargo test -p tcld --features hybrid \
  --test hybrid_run_contract --test hybrid_debug_contract
scripts/verify-hybrid-corpus.sh
```

## Tcl API adaptation

The Rust boundary provides version-neutral helpers for C APIs that changed
between Tcl 8 and 9:

| Helper | Tcl 9 | Tcl 8 |
|---|---|---|
| `tcl_sys::Tcl_GetChild` | direct symbol | links to `Tcl_GetSlave` |
| `tcl_sys::has_string_rep` | `Tcl_HasStringRep` | returns `true` |
| `tcl_sys::fetch_internal_rep` | `Tcl_FetchInternalRep` | returns `NULL` |
| `tcl_sys::Tcl_Size` | `isize` | `c_int` |

Code that depends on internal-representation inspection must compile only with
`feature = "tcl9"`. Tcl 8 helpers intentionally provide no deeper inspection.

## Verification

```bash
# Tcl 9.0
cargo build --workspace
cargo test --workspace
cargo build -p tcld-dbg-ext --features integration_tclsh
cargo test -p tcld-dbg-ext --features integration_tclsh \
  --test wire_real_tclsh --test wire_parity_matrix

# Tcl 8.6
TCL8_6_PREFIX=/opt/homebrew/opt/tcl-tk@8 \
  cargo build -p tcld-dbg-ext --no-default-features --features tcl8_6
TCL8_6_PREFIX=/opt/homebrew/opt/tcl-tk@8 \
TCLD_TCLSH=/opt/homebrew/opt/tcl-tk@8/bin/tclsh8.6 \
  cargo test -p tcld-dbg-ext --no-default-features \
  --features tcl8_6,integration_tclsh \
  --test wire_real_tclsh --test wire_parity_matrix

# Tcl 8.5 loadable nub
cargo build -p tcld-dbg-ext --no-default-features \
  --features tcl8_5,loadable-extension --target-dir target/debug-nub-tcl85
printf 'load {%s} Tcld_dbg\nputs [package require tcld_dbg]\n' \
  target/debug-nub-tcl85/debug/libtcld_dbg.dylib | /usr/bin/tclsh8.5
```

CI requires Tcl 8.6 and 9.0. macOS release jobs also require Tcl 8.5 when they
package the system-shell nub.

## Release archives

Release archives contain a Tcl-independent CLI and loadable nubs under:

```
lib/tcld/nub/tcl8.5
lib/tcld/nub/tcl8.6
lib/tcld/nub/tcl9.0
```

The Tcl 8.5 directory is optional. Each directory contains the library and a
manifest with the Tcl ABI, wire-protocol major, target triple, filename, and
SHA-256. Nubs resolve Tcl symbols from the selected interpreter and carry no
build-machine Tcl dependency or runtime path.

At startup, `tcld debug` probes candidate shells, selects the matching nub,
validates its manifest, and checks both ABIs during the wire handshake.
Mismatches are rejected before the target is sourced. Explicit `--tclsh`,
`--dylib`, `TCLD_TCLSH`, and `TCLD_DBG_DYLIB` overrides remain subject to the
same ABI check.

Without a target constraint, discovery prefers Tcl 9, then 8.6, then 8.5.
Tcl 8 bytecode prefers 8.6 and falls back to 8.5. Tcl 9 bytecode selects Tcl 9.
An explicit cross-ABI shell override is accepted with a warning.

Package and verify nubs in isolated target directories:

```bash
scripts/package-debug-nubs.sh \
  <tcl86-lib> <tcl9-lib> <bundle-root> <target-triple> [<tcl85-lib>]
scripts/verify-debug-release-bundle.sh <bundle-root>
```

The bundle verifier checks manifests, Tcl linkage, runtime paths, loadability,
and PTY startup in each configured interpreter.

## Stability

During the 0.x series, decompile and disassemble output shapes and process exit
codes are stable. Coverage TSV columns, dump output, explorer session files,
debugger commands, and the independently versioned wire protocol may change
between minor releases.
