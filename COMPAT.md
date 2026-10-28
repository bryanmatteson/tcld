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
The source compiler reads words as Tcl 9's parser does: braces and quotes
group only at the start of a word, so `a{b}c`, `a"b` and `${name}x` are
ordinary unquoted words; braced variable names may hold balanced braces; and
array indices, command substitutions, and comments end where Tcl ends them.
Compiled expressions take the same operands Tcl's do: array elements such as
`$a($k)`, braced names such as `${x y}`, quoted strings with substitutions,
and braced literals. As in Tcl, operands keep their strings inside an
expression (`$x eq "0x10"`), and only the expression's value becomes a number.

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
| Tcl 8.6 | 173 | 1 | 2 | 14 |
| Tcl 9.0 | 172 | 1 | 2 | 16 |

The VM computes arithmetic, math functions, strings, dictionaries, clock
reads, and Tcl 9 `lreplace`/`linsert` natively, matching Tcl 9 results and
errors, so `vm-strict` runs them too. String case and character classes use
Tcl 9.0.2's Unicode tables. Regular expressions use a native Rust port of
Tcl 9.0.2's Henry Spencer engine, including its Unicode classes, captures,
backreferences, lookahead constraints, and compile errors. Only `namespace
upvar` still needs the host to compute a value.

The source compiler emits these instructions where Tcl's own compiler does:
`string` subcommands such as `length`, `index`, `range`, `map`, `trim`, `is`,
and `match`; `dict get`, `exists`, `set`, `unset`, `incr`, `append`,
`lappend`, `create`, `for`, `map`, `update`, and `with`; and `clock`
readings.

`list`, `llength`, `lindex`, `lrange`, `concat`, `join`, `split`, and `error`
are VM commands in every form, including expanded `{*}` arguments and
computed `lrange` indices. The compiler emits Tcl's list instructions for the
forms Tcl compiles and invokes the VM command for the rest.

Every `string` subcommand, and every `dict` subcommand that does not evaluate
a script, is also a VM command, with Tcl's results and its usage, option, and
index errors: for example `string repeat`, `reverse`, `wordstart`, `wordend`,
`insert`, and `is` with `-strict` and `-failindex`, and `dict keys`, `values`,
`size`, `merge`, `remove`, `replace`, `getdef`, and `filter` by key or value.
Forms the compiler does not lower, including computed subcommands, run as VM
commands in `vm-strict` and in compiled source; precompiled bytecode in hybrid
mode still calls Tcl's commands for them.

`vm-strict` also compiles source created at runtime: unbraced `expr` with
substitutions, `eval`, `uplevel`, procedures with computed names or bodies,
`namespace eval`, `inscope`, and `code`, control commands with computed bodies
or conditions, `switch` arms, `lmap` bodies, `apply` lambdas, and computed
command names. Generated code is admitted under
`vm-strict` as well, so it cannot reach the host either. In `vm-strict` the VM
keeps global and namespace variables and namespaces itself; hybrid mode keeps
them in embedded Tcl. A command that is neither a Tcl command nor defined in
the image resolves when it runs, because the program may create it then; an
undefined one fails with Tcl's `invalid command name` error. Commands Tcl
implements with its event loop, file system, or callbacks into script still
require hybrid mode: `source`, `after`, `update`, `vwait`, `trace`,
`subst`, `time`, `tailcall`, the `dict` forms that
evaluate a computed script (`for`, `map`, `update`, and `with` with a computed
body, `update` of a qualified variable, and `filter` with a script), and
`dict info`, which reports Tcl's own hash table.

In `vm-strict`, `puts` to `stdout` and `stderr` runs in the VM, with Tcl's
arguments and errors. It writes the process's own streams, or the
debugger's output events when the debugger captures them, as hybrid mode's
host capture does; any other channel requires hybrid mode.

The VM answers `info exists`, `commands`, `procs`, `body`, `args`, and `level`
from its own procedures, levels, and variables, and runs `lsearch` and
`lsort` itself in both modes, including `lsort -command`.
`regexp`, `regsub`, and `lsearch -regexp` also run natively in both modes,
including expanded or computed arguments, match variables, `-all`,
`-indices`, `-inline`, `-start`, `-about`, and replacement backreferences.
Their results and error codes are checked against embedded Tcl, including
the applicable command cases from Tcl's `reg.test` corpus.

In `vm-strict`, `info commands` lists the program's
procedures and the Tcl commands the VM provides, `rename` renames or deletes
the program's procedures. `lsort -command` invokes mapped comparator commands
in both modes, preserving Tcl's comparison order, stable ties, `-unique`,
`-index`, `-stride`, and `-indices`. Comparator results follow Tcl's integer
conversion rules; errors and other completion codes propagate with the
comparison context. `regsub -command` invokes replacement commands in mapped VM
images in both modes, in the caller's variable scope and namespace. Captures
are literal arguments, including every subexpression when there are more than
nine, and successful replacement values are appended literally.
Errors and other completion codes propagate, and an output variable is assigned
only after substitution succeeds. As in Tcl, these synchronous callbacks cannot
yield across the substitution computation or a sort comparison.

`apply` executes the lambda's exact body as mapped VM source in both modes,
with its own local variables, literal defaults, variadic arguments, and the
lambda's global or explicit namespace. It supports command prefixes used by
`lsort -command` and `regsub -command`, including computed lambdas and captured
arguments. Lambda bodies retain breakpoint and local-edit support. Argument,
namespace, and completion results are checked against embedded Tcl; hybrid
mode also retains coroutine suspension, traces, and tailcalls through lambdas.

`switch` selects arms natively in both modes, including computed arm lists,
abbreviated options, exact/glob/regexp matching, `-nocase`, fallthrough,
and regexp capture variables. Only the selected body is admitted and compiled;
it executes as mapped source in the caller's scope and namespace, preserving
breakpoints, locals, return/break/continue, and the selected pattern's error
context. Branches inside lambda, sort, and substitution callbacks remain mapped.
Results, completion options, and error codes are compared with embedded Tcl in
both modes; hybrid mode also supports traces and coroutine suspension in arms.
Mapped bodies retain their code identity as callback arguments change, so a
breakpoint in a lambda or branch continues to stop on later comparisons and
substitutions. Debugger local edits remain available in hybrid mode.

`lmap` iterates and collects results natively in both modes, including computed
variable lists, input lists, and bodies. It snapshots input values, pads short
lists with empty values, binds variables in the caller's scope, and preserves
Tcl's assignment order, break/continue behavior, and nonlocal completions.
Eligible literal loops inside procedures use native foreach/lmapCollect
instructions and retain their original source maps. Runtime bodies are admitted
only when an iteration executes. Both forms retain repeated breakpoints and
caller locals, including inside sort and substitution callbacks. Hybrid mode
also preserves variable traces, coroutine suspension, and tailcalls. Results,
completion options, and error codes are checked against embedded Tcl in both
modes.

`try` validates clauses, selects handlers, and runs cleanup natively in both
modes. Bodies, `on`/`trap` handlers, and `finally` scripts enter exact mapped
source only when executed. Handler bindings use the caller's scope; shared
handler bodies, custom completion codes, return levels, and nested `-during`
options follow Tcl. Those scripts retain breakpoints and locals, including in
sort and substitution callbacks. Hybrid mode also supports traces, coroutines,
cancellation, and tailcalls through try handlers and cleanup.

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

Explicit local CI requires Tcl 8.6 and 9.0. Release packages include their
loadable nubs. Tcl 8.5 remains an optional source-build extension. See
[Distribution](DISTRIBUTION.md) for the local OrbStack runner.

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
