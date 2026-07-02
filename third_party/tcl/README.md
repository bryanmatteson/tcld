# Tcl Runtime Patch

This directory contains the pinned Tcl 9 runtime patch used for opcode-level
debugger stepping.

Patch:

- `patches/0001-opcode-runtime-hooks.patch`

Build:

- `scripts/build_tcl9_opcode_runtime.sh`

The debug extension probes the runtime symbols before advertising
`instruction_step=1`. An unpatched Tcl 9 runtime reports
`instruction_step=0`.
