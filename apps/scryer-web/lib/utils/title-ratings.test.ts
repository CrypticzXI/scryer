import assert from "node:assert/strict";
import test from "node:test";

import { ratingSourceInfo, ratingValueLabel } from "./title-ratings.ts";

test("MDB Rotten Tomatoes sources render as percentages with logos", () => {
  for (const source of ["rottentomatoes", "tomatoes"]) {
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

test("MDB Rotten Tomatoes audience sources use the Popcornmeter logo", () => {
  for (const source of ["audience", "popcorn", "popcornmeter"]) {
    const info = ratingSourceInfo(source);
    assert.equal(info.logoSrc, "/rating-sources/popcornmeter.svg");
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

test("MAL rating pill reuses the existing media-site logo", () => {
  for (const source of ["mal", "myanimelist", "my-anime-list", "myanimelist.net"]) {
    const info = ratingSourceInfo(source);
    assert.equal(info.label, "MyAnimeList");
    assert.equal(info.logoSrc, "/media-sites/mal.svg");
  }
});

test("TVDB rating pill reuses the existing media-site logo", () => {
  for (const source of ["tvdb", "thetvdb", "the-tvdb"]) {
    const info = ratingSourceInfo(source);
    assert.equal(info.label, "TVDB");
    assert.equal(info.logoSrc, "/media-sites/tvdb.svg");
  }
});
