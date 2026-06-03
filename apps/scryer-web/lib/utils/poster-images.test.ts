import assert from "node:assert/strict";
import test from "node:test";

import { selectPosterVariantUrl } from "./poster-images.ts";

test("rewriting a local poster variant drops the source variant version token", () => {
  assert.equal(
    selectPosterVariantUrl(
      "/images/titles/title-1/poster/w250?v=posterw250digest#poster",
      "w70",
    ),
    "/images/titles/title-1/poster/w70#poster",
  );
});

test("selecting the current local poster variant preserves its version token", () => {
  assert.equal(
    selectPosterVariantUrl("/images/titles/title-1/poster/w250?v=posterw250digest", "w250"),
    "/images/titles/title-1/poster/w250?v=posterw250digest",
  );
});
