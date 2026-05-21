import test from "node:test";
import assert from "node:assert/strict";

import { classifyStatusToastLevel } from "./status-toast.ts";

test("raw GraphQL validation queue failure classifies as error", () => {
  assert.equal(
    classifyStatusToastLevel(
      "[GraphQL] validation: no download client enabled for library movie_default_library",
    ),
    "error",
  );
});

test("normalized validation queue failure classifies as error", () => {
  assert.equal(
    classifyStatusToastLevel("no download client enabled for library movie_default_library"),
    "error",
  );
});

test("suppressed validation prompts still do not toast", () => {
  assert.equal(classifyStatusToastLevel("validation: title is required"), null);
  assert.equal(classifyStatusToastLevel("title is required"), null);
});
