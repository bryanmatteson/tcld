# Third-Party Notices

Tcld is licensed under Apache-2.0. The following third-party material retains
their original permissive terms:

Desktop distributions include Tcl 9.0.2 with the opcode-runtime patch under
`third_party/tcl/patches/`. Tcl's redistribution terms are included in the
desktop package as `licenses/license.terms`. Its script library and shared
library remain under those terms. The CLI archives contain the loadable
debugger extensions and do not embed a Tcl runtime.

- `apps/tcld/tests/fixtures/` contains TclPro compiler fixtures from Ajuba
  Solutions and ActiveState. Its notice is reproduced at
  `apps/tcld/tests/fixtures/license.terms`.
- `testing/fixtures/bytecode/tcl8/`, `testing/fixtures/bytecode/tcl9/`, and
  matching fuzz seeds derive from the same corpus.
  `testing/fixtures/licenses/tclpro-corpus.terms` and
  `testing/fuzz/corpus/license.terms` point to that notice.

`testing/fixtures/public/public_complex_fixture.tcl` is original Tcld material
licensed under Apache-2.0. Its bytecode was generated with the open-source Tcl
compiler. The compiler itself is not distributed here.

The native regular-expression implementation in `crates/tcld-vm/src/regex/`
is a Rust port of Henry Spencer's engine as shipped in Tcl 9.0.2. Original
copyright notices and redistribution terms are retained in the corresponding
source files. Tcl-specific locale tables and command integration in
`crates/tcld-vm/src/regexp_command.rs` retain their original notices; Tcl's
complete terms are reproduced in `crates/tcld-vm/src/regex/license.terms`.
The oracle fixture `crates/tcld-vm/tests/fixtures/tcl9-reg.test` is an unmodified
copy of Tcl 9.0.2's `tests/reg.test`, under those same terms with its original
notice retained.
The port replaces C pointers and interpreter-owned objects with Rust arenas,
indices, and VM values, and omits Tcl's internal diagnostic switches.

The command comparison merge sort in `crates/tcld-vm/src/lsort.rs` is ported
from Tcl 9.0.2's `generic/tclCmdIL.c`. Its source header retains the original
copyright notices, under the same Tcl terms in
`crates/tcld-vm/src/regex/license.terms`.

Native `switch` matching and argument handling in
`crates/tcld-vm/src/engine/switch.rs` are ported from Tcl 9.0.2's
`generic/tclCmdMZ.c`, retaining its copyright notices under the same Tcl terms.

Native `lmap` iteration and argument handling in
`crates/tcld-vm/src/engine/lmap.rs` are ported from Tcl 9.0.2's
`generic/tclCmdAH.c`, retaining its copyright notices under the same Tcl terms.

Native `try` parsing, handler selection and completion handling in
`crates/tcld-vm/src/engine/try_cmd.rs` are ported from Tcl 9.0.2's
`generic/tclCmdMZ.c`, retaining its copyright notices under the same Tcl terms.
