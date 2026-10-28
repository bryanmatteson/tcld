# VM test fixtures

`tcl9-reg.test` is an unmodified copy of Tcl 9.0.2's `tests/reg.test`.
The native regular-expression oracle test embeds it so that a committed source
export can run the same corpus without an untracked Tcl source checkout.
Its original copyright notice is retained; Tcl's complete redistribution terms
are in `../../src/regex/license.terms`.

`hybrid-corpus.toml` selects Tcld's runtime comparison fixtures and expectations.
