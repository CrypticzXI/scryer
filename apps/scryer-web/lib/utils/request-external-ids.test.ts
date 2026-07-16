import assert from "node:assert/strict";
import test from "node:test";

import {
  formatExternalIdSourceLabel,
  getRawRequestExternalIds,
} from "./request-external-ids.ts";

test("raw request external IDs exclude linked providers and keep unknown IDs", () => {
  assert.deepEqual(
    getRawRequestExternalIds([
      { source: "IMDB", value: "tt1234567" },
      { source: "tvdb", value: "9010" },
      { source: "kitsu", value: "444004" },
      { source: "trakt", value: "888008" },
      { source: "trakt", value: "888008" },
    ]),
    [
      { source: "kitsu", value: "444004" },
      { source: "trakt", value: "888008" },
    ],
  );
});

test("raw request external ID labels are compact source names", () => {
  assert.equal(formatExternalIdSourceLabel("alt_tvdb"), "ALT TVDB");
  assert.equal(formatExternalIdSourceLabel("provider-id"), "PROVIDER ID");
});
