import assert from "node:assert/strict";
import test from "node:test";

import {
  selectedOverviewUsesMovieRecord,
  selectedSeriesSidePanelTitleId,
  selectedSidePanelOwner,
} from "./selected-overview-policy.ts";

test("movie selected overview uses the movie record side panel", () => {
  assert.equal(selectedSidePanelOwner("movies"), "movie-record");
  assert.equal(selectedOverviewUsesMovieRecord("movies"), true);
});

test("series selected overview uses the series side panel container", () => {
  assert.equal(selectedSidePanelOwner("series"), "series-container");
  assert.equal(selectedOverviewUsesMovieRecord("series"), false);
});

test("anime selected overview uses the series side panel container", () => {
  assert.equal(selectedSidePanelOwner("anime"), "series-container");
  assert.equal(selectedOverviewUsesMovieRecord("anime"), false);
});

test("series and anime can render the side panel from selected title id", () => {
  assert.equal(selectedSeriesSidePanelTitleId("series", "title-1"), "title-1");
  assert.equal(selectedSeriesSidePanelTitleId("anime", "title-2"), "title-2");
  assert.equal(selectedSeriesSidePanelTitleId("movies", "title-3"), null);
});
