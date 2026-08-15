import assert from "node:assert/strict";
import test from "node:test";

import { facetById, facetByMetadataKey } from "./registry.ts";

test("metadata facet keys resolve to canonical GraphQL enum values", () => {
  assert.equal(facetByMetadataKey("movie")?.id, "MOVIE");
  assert.equal(facetByMetadataKey("series")?.id, "SERIES");
  assert.equal(facetByMetadataKey("anime")?.id, "ANIME");
});

test("canonical facets use the canonical facet lookup", () => {
  assert.equal(facetById("MOVIE")?.metadataKey, "movie");
  assert.equal(facetById("SERIES")?.metadataKey, "series");
  assert.equal(facetById("ANIME")?.metadataKey, "anime");

  if (false) {
    // @ts-expect-error Canonical GraphQL enum values are not metadata keys.
    facetByMetadataKey("ANIME");
  }
});
