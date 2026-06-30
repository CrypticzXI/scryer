import assert from "node:assert/strict";
import test from "node:test";

import { ratingSourceInfo, ratingValueLabel } from "./title-ratings.ts";

test("MDB Rotten Tomatoes sources render as percentages with logos", () => {
  for (const source of ["rottentomatoes", "tomatoes", "audience", "popcorn"]) {
    const info = ratingSourceInfo(source);
    assert.equal(info.logoSrc, "/rating-sources/rotten-tomatoes.svg");
    assert.equal(info.format, "percent");
    assert.equal(
      ratingValueLabel(
        {
          source,
          value: 94,
          score: null,
          normalized: 9.4,
          votes: null,
          url: "",
        },
        info,
      ),
      "94%",
    );
  }
});

test("MDB normalized-only percentage ratings expand from a ten-point scale", () => {
  assert.equal(
    ratingValueLabel({
      source: "tomatoes",
      value: null,
      score: null,
      normalized: 8.7,
      votes: null,
      url: "",
    }),
    "87%",
  );
});

test("Metacritic ratings render on a hundred-point scale", () => {
  assert.equal(
    ratingValueLabel({
      source: "metacritic",
      value: 7.5,
      score: null,
      normalized: 7.5,
      votes: null,
      url: "",
    }),
    "75",
  );
});
