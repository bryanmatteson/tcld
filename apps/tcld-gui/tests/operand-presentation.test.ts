import assert from "node:assert/strict";
import test from "node:test";

import {
  formatAddress,
  indexLiterals,
  operandCoversComment,
  presentOperand,
} from "@/lib/operand-presentation";
import type { Instruction, LiteralEntry } from "@/types";

function inst(overrides: Partial<Instruction>): Instruction {
  return {
    idx: 0,
    pc: 0,
    op: "push1",
    args: "",
    operands: [],
    argc: 0,
    group: 0,
    commandPcRange: [0, 4],
    groupStart: false,
    comment: "",
    size: 2,
    isJump: false,
    isJumpConditional: false,
    isJumpUnconditional: false,
    jumpTarget: null,
    fallthrough: null,
    primaryNextLocation: null,
    secondaryNextLocation: null,
    nextLocations: [],
    ...overrides,
  };
}

const literals = indexLiterals([
  { idx: 0, kind: "XString", value: "outerCounter" },
  { idx: 1, kind: "Int", value: "8" },
  { idx: 2, kind: "XString", value: "" },
] satisfies LiteralEntry[]);

test("push operands show the literal they push, quoted by literal kind", () => {
  assert.deepEqual(
    presentOperand(inst({ op: "push1", args: "0", operands: [0], comment: "outerCounter" }), literals),
    { kind: "literal", value: "outerCounter", quoted: true, index: 0 },
  );
  assert.deepEqual(
    presentOperand(inst({ op: "push4", args: "1", operands: [1] }), literals),
    { kind: "literal", value: "8", quoted: false, index: 1 },
  );
  // An empty string literal is still a value worth showing.
  assert.deepEqual(
    presentOperand(inst({ op: "push1", args: "2", operands: [2] }), literals),
    { kind: "literal", value: "", quoted: true, index: 2 },
  );
});

test("a push without a literal table entry falls back to the payload comment", () => {
  const empty = indexLiterals([]);
  assert.deepEqual(
    presentOperand(inst({ op: "push1", args: "5", operands: [5], comment: "puts" }), empty),
    { kind: "literal", value: "puts", quoted: true, index: 5 },
  );
  assert.deepEqual(
    presentOperand(inst({ op: "push1", args: "6", operands: [6], comment: "42" }), empty),
    { kind: "literal", value: "42", quoted: false, index: 6 },
  );
});

test("literal tables keyed by `index` resolve like `idx`", () => {
  const legacy = indexLiterals([{ index: 3, value: "set" } as unknown as LiteralEntry]);
  assert.equal(legacy.get(3)?.value, "set");
});

test("jumps show their target and direction; invokes count words", () => {
  assert.deepEqual(
    presentOperand(inst({ op: "jumpFalse1", pc: 21, args: "79", operands: [79], isJump: true, jumpTarget: 100 }), literals),
    { kind: "jump", target: 100, backward: false },
  );
  assert.deepEqual(
    presentOperand(inst({ op: "jump1", pc: 98, args: "-80", operands: [-80], isJump: true, jumpTarget: 18 }), literals),
    { kind: "jump", target: 18, backward: true },
  );
  assert.deepEqual(
    presentOperand(inst({ op: "invokeStk1", args: "3", operands: [3] }), literals),
    { kind: "words", count: 3 },
  );
});

test("other operands stay raw, and operandless instructions show nothing", () => {
  assert.deepEqual(presentOperand(inst({ op: "loadScalar1", args: "%v0", operands: [0] }), literals), { kind: "raw", text: "%v0" });
  assert.deepEqual(presentOperand(inst({ op: "pop" }), literals), { kind: "none" });
});

test("comments that repeat the operand cell are recognized", () => {
  const push = inst({ op: "push1", args: "0", operands: [0], comment: "outerCounter" });
  assert.equal(operandCoversComment(push, presentOperand(push, literals)), true);
  const jump = inst({ op: "jump1", pc: 4, args: "6", operands: [6], isJump: true, jumpTarget: 10, comment: "pc 10" });
  assert.equal(operandCoversComment(jump, presentOperand(jump, literals)), true);
  const invoke = inst({ op: "invokeStk1", args: "4", operands: [4], comment: "4" });
  assert.equal(operandCoversComment(invoke, presentOperand(invoke, literals)), true);
  const immediate = inst({ op: "incrScalarStkImm", args: "1", operands: [1], comment: "1" });
  assert.equal(operandCoversComment(immediate, presentOperand(immediate, literals)), true);
  const annotated = inst({ op: "loadScalar1", args: "%v0", operands: [0], comment: "var x" });
  assert.equal(operandCoversComment(annotated, presentOperand(annotated, literals)), false);
});

test("addresses use the address column's spelling", () => {
  assert.equal(formatAddress(0x64), "0064");
  assert.equal(formatAddress(0x1_2345, 5), "12345");
});
