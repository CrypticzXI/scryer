import assert from "node:assert/strict";
import test from "node:test";

import { nonEmptySecret } from "./secret-input.ts";

test("non-empty secrets preserve leading and trailing whitespace", () => {
  assert.equal(nonEmptySecret("  password  "), "  password  ");
  assert.equal(nonEmptySecret("password"), "password");
});

test("whitespace-only secrets are preserved and only empty secrets are rejected", () => {
  assert.equal(nonEmptySecret(""), undefined);
  assert.equal(nonEmptySecret("   "), "   ");
});
