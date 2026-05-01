import assert from "node:assert/strict";
import test from "node:test";
import {
  acceptsReadyDebugSession,
  acceptsRuntimeDataDebugSession,
  canLaunchDebugSession,
  canStopDebugSession,
  isActiveDebugSession,
  normalizeBackendDebugState,
} from "@/lib/debug-session-machine";
import type { DebugSessionState } from "@/types";

test("state predicates reflect debug lifecycle gates", () => {
  const activeStates: DebugSessionState[] = ["launching", "connected", "running", "pausing", "paused"];
  for (const state of activeStates) {
    assert.equal(isActiveDebugSession(state), true);
  }
  assert.equal(isActiveDebugSession("idle"), false);
  assert.equal(isActiveDebugSession("terminated"), false);

  assert.equal(canLaunchDebugSession("idle"), true);
  assert.equal(canLaunchDebugSession("terminated"), true);
  assert.equal(canLaunchDebugSession("running"), false);

  assert.equal(canStopDebugSession("idle"), false);
  assert.equal(canStopDebugSession("terminated"), false);
  assert.equal(canStopDebugSession("paused"), true);

  assert.equal(acceptsReadyDebugSession("launching"), true);
  assert.equal(acceptsReadyDebugSession("connected"), true);
  assert.equal(acceptsReadyDebugSession("running"), false);

  assert.equal(acceptsRuntimeDataDebugSession("paused"), true);
  assert.equal(acceptsRuntimeDataDebugSession("running"), true);
  assert.equal(acceptsRuntimeDataDebugSession("connected"), false);
});

test("backend state normalization accepts only supported sync states", () => {
  assert.equal(normalizeBackendDebugState("idle"), "idle");
  assert.equal(normalizeBackendDebugState("running"), "running");
  assert.equal(normalizeBackendDebugState("paused"), "paused");
  assert.equal(normalizeBackendDebugState("pausing"), null);
  assert.equal(normalizeBackendDebugState("stopping"), null);
  assert.equal(normalizeBackendDebugState("unknown"), null);
});
