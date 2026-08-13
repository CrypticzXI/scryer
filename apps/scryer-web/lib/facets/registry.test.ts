import assert from "node:assert/strict";
import test from "node:test";

import { facetByMetadataKey } from "./registry.ts";

test("metadata facet keys resolve to canonical GraphQL enum values", () => {
  assert.equal(facetByMetadataKey("movie")?.id, "MOVIE");
  assert.equal(facetByMetadataKey("series")?.id, "SERIES");
  assert.equal(facetByMetadataKey("anime")?.id, "ANIME");
  assert.equal(facetByMetadataKey("unknown"), undefined);
});
