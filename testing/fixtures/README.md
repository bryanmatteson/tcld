# Test Fixtures

## Public fixture

`public_complex_fixture.tcl` is an original, redistributable Tcl program
written for Tcld. `public_complex_fixture.tbc` was generated from that source
with the BSD-licensed Tcl compiler against Tcl 8.4.20. Together they exercise
namespaced procedures, nested command mappings, compound control flow, arrays,
debugger navigation, and hybrid execution.

## Compatibility corpus

`bytecode/tcl8/` and `bytecode/tcl9/` contain TclPro loader fixtures for both
bytecode formats. Golden fixtures under `apps/tcld/tests/fixtures/` derive from
the Ajuba Solutions and ActiveState compiler test suite. Redistribution terms
are recorded in `apps/tcld/tests/fixtures/license.terms` and referenced from
the other corpus directories. See `THIRD_PARTY_NOTICES.md`.

## Debugging samples

The scripts under `apps/` contain small, intentional defects for debugger
practice.

## Run

```bash
tclsh testing/fixtures/apps/buggy_budget_app.tcl
tclsh testing/fixtures/apps/buggy_contact_book.tcl
tclsh testing/fixtures/apps/buggy_log_monitor.tcl
tclsh testing/fixtures/apps/buggy_rollout_scheduler.tcl
```

- `buggy_budget_app.tcl` — numeric errors in a budget report
- `buggy_contact_book.tcl` — a missed state update
- `buggy_log_monitor.tcl` — parsing and classification errors
- `buggy_rollout_scheduler.tcl` — ordering and selection errors
