import assert from "node:assert/strict";
import test from "node:test";

import { authSessionPersistence } from "./auth-session-persistence.ts";

test("persistent external login sessions survive browser restarts", () => {
  assert.equal(authSessionPersistence(true), "persistent");
});

test("non-persistent external login sessions stay in the current tab", () => {
  assert.equal(authSessionPersistence(false), "tab");
});
