import assert from "node:assert/strict";
import test from "node:test";

import { buildCalendarEventHref } from "./calendar-event-href.ts";

test("calendar series events link directly to the episode", () => {
  assert.equal(
    buildCalendarEventHref({
      id: "episode-42",
      titleId: "title-7",
      titleFacet: "series",
      titleSlug: "example-show",
      librarySlug: "series",
    }),
    "/series/example-show?episodeId=episode-42",
  );
});

test("calendar movie events link to the movie without an episode parameter", () => {
  assert.equal(
    buildCalendarEventHref({
      id: "movie-calendar-record",
      titleId: "movie-7",
      titleFacet: "movie",
      titleSlug: "example-movie",
      librarySlug: "movies",
    }),
    "/movies/example-movie",
  );
});

test("calendar event links fall back to title IDs when slugs are unavailable", () => {
  assert.equal(
    buildCalendarEventHref({
      id: "episode-42",
      titleId: "title-7",
      titleFacet: "anime",
    }),
    "/anime?id=title-7&episodeId=episode-42",
  );
});
