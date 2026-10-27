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

Source admission is the bounded `tcld.source-compiler/v1.17-try-tcl86` contract.
Static braced `if`, `while`, `for`, `foreach` and `catch` bodies compile into mapped
instructions, including nested commands, break/continue, lazy expressions and
builtin math functions. All six GUI examples compile from source, including edits.

In Hybrid and `vm-strict` modes, `eval`, computed command names and runtime-provided bodies of these
control forms are compiled after their arguments are evaluated. Generated images
have distinct code IDs and virtual `.tcld-generated/*.tcl` source paths, captured
source text, verified command maps and the code address that generated them.
Dynamic execution frames share their caller's variables; `return` levels count Tcl
procedure frames, while break/continue and errors propagate through generated frames.
Multi-argument `eval` uses real Tcl `concat` semantics. Generated procedures are
activated when their declaration executes. Their bodies and formal arguments
undergo the same source admission checks as statically declared procedures.

`source ?-encoding name? file` reads through Tcl's real channels and captures each
loaded revision with its file path, source maps and originating call site. Source
return levels and `info script` retain file semantics. `namespace eval` and
`namespace inscope` use persistent namespace variables and qualify declarations
when they execute. `uplevel` shares the selected relative/absolute caller scope; `upvar` links that scope, including array-element aliases.
Namespace frames count as Tcl levels; eval/source implementation frames do not.

`namespace code`, `apply`, timed/idle `after` callbacks, `update`, `update idletasks`,
`vwait` and `lsort -command` are scheduler-owned. Callback bodies and comparators
can pause, step, hit source/PC breakpoints and edit their actual execution scope.
The event queue supports `after info` and cancellation, removes a callback before
execution, and reports background errors through an authored `bgerror` procedure
or stderr. `vwait` uses Tcl write/unset traces so host leaf mutations also wake it;
without pending supported events it reports the dead wait. Registered channel
events also keep `vwait` waiting for actual Tcl notifier readiness. Comparator sorting uses a stable merge sort
with ordering, index, stride, unique and indices options. All discovered source
revisions remain inspectable until restart/reset.

Variable `trace add/remove/info` and `fileevent` / `chan event` callbacks use the
real Tcl runtime. Tcl owns triggering, callback order, recursive trace guards,
read/write/unset behavior, handler removal and background errors. Visitor callback
bodies enter mapped debug images with their registration call site, so they can
pause, step, hit breakpoints and edit the actual execution scope. A stackful
continuation preserves the triggering Tcl operation while a trace is paused; it
is neither replayed nor deferred. Native builds use guarded coroutine stacks;
Emscripten builds use Asyncify fibers. Inspecting the Variables panel suppresses
visitor read callbacks; explicit debugger evaluation still executes them.

Channel readiness comes from Tcl's notifier. Browser workers use the upstream
select notifier's single-thread branch and virtual filesystem channels, with
network access blocked. Native channels retain their normal host capabilities.
Command rename/delete and all four execution-trace operations (`enter`, `leave`,
`enterstep`, `leavestep`) expose mapped callback bodies. Compiler-owned commands
participate through evaluated-argument and completion boundaries, including
exception unwinding and real Tcl ensemble mappings. The adapter invokes Tcl’s
extension execution hooks (`TclCheckExecutionTraces`, `TclCheckInterpTraces`);
Tcl owns trace ordering, registration changes and recursion guards. Internal
implementation calls are masked to avoid duplicate events. These are Tcl command
execution traces; instruction stepping remains the mapped debugger’s own operation.

`coroutine`, `yield` and `yieldto` use Tcl's actual NRE coroutine evaluator.
Mapped frames and pending command completions are retained across suspension;
resuming a coroutine returns Tcl's real resume value to the suspended instruction.
Bodies loaded through `source`, generated `eval`, namespaces and nested calls
retain their source provenance and code-qualified breakpoints. A pause can inspect
or edit the active coroutine's locals, then continue without replaying the body.
`yieldto` runs a mapped target in the caller's actual variable context, including
nested transfers and caller-coroutine yields. Tcl owns command lifetime, resume
arity, already-running checks and NRE stack unwinding; deleting a suspended
coroutine destroys its mapped local scopes and runs real Tcl unset traces.

Unmapped evaluator families still report explicit diagnostics: Tk/native extension
callbacks, `interp eval`, and other evaluator families
without a resumable scheduler adapter. Computed command names and
subcommands undergo the same effect check at runtime. They cannot bypass this
boundary through the opaque host leaf bridge. `vm-strict` admits runtime-created
source under the same mode and rejects forms requiring the host runtime.
A source/admission failure is structured for scripts and CI;
generated failures are catchable Tcl errors raised at the generating call:

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

`tailcall` uses the real Tcl command to prepare a pending tail in the current
procedure's namespace. A caught return retains that pending tail; a later
no-argument tailcall clears it, and errors preempt it. After local cleanup and
procedure leave traces, Tcl's native call-frame destruction and NRE scheduler
run the mapped target continuation. The target retains its producing source
address, inspectable generated invocation, procedure body and breakpoints. The
departed procedure is absent from the debugger stack; repeated transfers replace
hidden redirectors rather than growing VM or native evaluation stacks. Procedure
and coroutine targets share this path, including targets that yield, resume other
coroutines or transfer through `yieldto`. Dynamic command names and `{*}` argument
expansion are evaluated once before the transfer. Native pending-tail tokens are
validated against the owning interpreter before use; visitor input is never
interpreted as a native pointer.


Dictionary script bodies (`dict for`, `dict map`, `dict update`, `dict with`,
`dict filter ... script`)
enter mapped source frames using the native Tcl NRE control adapter. Computed
bodies retain their exact text and generating caller; repeated for/map/filter bodies
retain code identity and breakpoints on subsequent iterations. Debugger edits
change the actual bound variables used by native result collection/write-back.
Native Tcl retains its pending finalizers across coroutine suspension, including
errors, return levels, break/continue, tail transfers and coroutine destruction.
The dict-with variable resolver applies only to its exact native control frame
in the active coroutine, preserving ordinary namespace lookup and caller aliases.

Filter predicates use an NRE continuation port of the upstream native filter loop.
The same native dictionary search holds original key/value pairs while mapped
predicate frames may yield, be inspected, or be edited. Tcl converts each result
to a boolean; continue skips a pair, break returns the partial selection, and other
completions propagate after releasing the search. Coroutine destruction and debug
restart run the pending cleanup. Ordinary filter predicates are compared against
unmodified Tcl; yield support is an explicit extension to stock Tcl's synchronous
filter evaluator. No JavaScript filter evaluator or helper procedure is used.
