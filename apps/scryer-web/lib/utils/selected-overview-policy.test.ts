import assert from "node:assert/strict";
import test from "node:test";

import {
  selectedOverviewDetailOwner,
  selectedOverviewNativeTitleId,
  selectedOverviewUsesPanelDetail,
} from "./selected-overview-policy.ts";

test("movie selected overview uses panel detail hydration", () => {
  assert.equal(selectedOverviewDetailOwner("movies"), "panel");
  assert.equal(selectedOverviewUsesPanelDetail("movies"), true);
});

test("series selected overview uses native overview hydration", () => {
  assert.equal(selectedOverviewDetailOwner("series"), "native-series-overview");
  assert.equal(selectedOverviewUsesPanelDetail("series"), false);
});

test("anime selected overview uses native overview hydration", () => {
  assert.equal(selectedOverviewDetailOwner("anime"), "native-series-overview");
  assert.equal(selectedOverviewUsesPanelDetail("anime"), false);
});

test("series and anime can render native overview from selected title id", () => {
  assert.equal(selectedOverviewNativeTitleId("series", "title-1"), "title-1");
  assert.equal(selectedOverviewNativeTitleId("anime", "title-2"), "title-2");
  assert.equal(selectedOverviewNativeTitleId("movies", "title-3"), null);
});
