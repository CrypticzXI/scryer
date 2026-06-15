import test from "node:test";
import assert from "node:assert/strict";

import { jwtDateClaimToMillis } from "./jwt.ts";

test("jwtDateClaimToMillis parses JWT numeric date seconds", () => {
  assert.equal(jwtDateClaimToMillis(1_781_538_560), 1_781_538_560_000);
});

test("jwtDateClaimToMillis parses numeric string seconds", () => {
  assert.equal(jwtDateClaimToMillis("1781538560"), 1_781_538_560_000);
});

test("jwtDateClaimToMillis parses RFC3339 timestamps", () => {
  assert.equal(
    jwtDateClaimToMillis("2026-06-15T15:49:20Z"),
    Date.parse("2026-06-15T15:49:20Z"),
  );
});

test("jwtDateClaimToMillis rejects missing and malformed claims", () => {
  assert.equal(jwtDateClaimToMillis(null), null);
  assert.equal(jwtDateClaimToMillis(undefined), null);
  assert.equal(jwtDateClaimToMillis(""), null);
  assert.equal(jwtDateClaimToMillis("not-a-date"), null);
  assert.equal(jwtDateClaimToMillis(Number.NaN), null);
});
