import assert from "node:assert/strict";
import test from "node:test";
import { precompiledStepAdvisory } from "@/lib/debug-workflows";

// The advisory exists to explain why the source marker won't leave line 1 when
// stepping a precompiled `.tbc` on a runtime without instruction stepping —
// the exact "stuck at line 1" symptom. It must fire only in that situation.

const NO_INSTR: Record<string, boolean> = { instruction_step: false };
const HAS_INSTR: Record<string, boolean> = { instruction_step: true };

test("fires for a .tbc target pinned at line 1 without instruction stepping", () => {
  const msg = precompiledStepAdvisory({ file: "public_complex_fixture.tbc", line: 1 }, "/testing/fixtures/public/public_complex_fixture.tbc", NO_INSTR);
  assert.ok(msg, "expected an advisory");
  assert.match(msg!, /precompiled bytecode/);
  assert.match(msg!, /instruction_step capability/);
  assert.match(msg!, /^public_complex_fixture\.tbc\b/, "leads with the target basename");
});

test("detects precompiled via scriptPath when the stopped file is bare", () => {
  const msg = precompiledStepAdvisory({ file: "", line: 0 }, "/x/public_complex_fixture.TBC", NO_INSTR);
  assert.ok(msg, "case-insensitive .tbc on scriptPath should qualify");
});

test("reports positive instruction stepping support once available", () => {
  const msg = precompiledStepAdvisory({ file: "public_complex_fixture.tbc", line: 1 }, "/testing/fixtures/public/public_complex_fixture.tbc", HAS_INSTR);
  assert.ok(msg, "expected a positive advisory");
  assert.match(msg!, /instruction stepping is available/);
});

test("stays silent for ordinary .tcl source", () => {
  assert.equal(precompiledStepAdvisory({ file: "public_complex_fixture.tcl", line: 1 }, "/testing/fixtures/public/public_complex_fixture.tcl", NO_INSTR), null);
});

test("stays silent once the marker has moved off the collapsed start line", () => {
  // A real line means source stepping is actually working — no advisory.
  assert.equal(precompiledStepAdvisory({ file: "public_complex_fixture.tbc", line: 42 }, "/testing/fixtures/public/public_complex_fixture.tbc", NO_INSTR), null);
});
