import assert from "node:assert/strict";
import test from "node:test";

import {
  shouldHandleTitleOverviewActivity,
  titleOverviewReactiveRefreshKinds,
  titleOverviewReactiveRefreshPlan,
} from "./title-overview-refresh-policy.ts";

const importKinds = new Set([
  "movie_downloaded",
  "series_episode_imported",
  "file_upgraded",
  "import_rejected",
]);

test("title overview activity gate ignores other and missing title ids", () => {
  assert.equal(shouldHandleTitleOverviewActivity("current", "other"), false);
  assert.equal(shouldHandleTitleOverviewActivity("current", null), false);
  assert.equal(shouldHandleTitleOverviewActivity("current", undefined), false);
  assert.equal(shouldHandleTitleOverviewActivity(null, "current"), false);
  assert.equal(shouldHandleTitleOverviewActivity("current", "current"), true);
});

test("title overview refresh kinds include policy-managed activity", () => {
  const kinds = titleOverviewReactiveRefreshKinds(importKinds);

  assert.equal(kinds.has("movie_downloaded"), true);
  assert.equal(kinds.has("series_episode_imported"), true);
  assert.equal(kinds.has("file_analyzed"), true);
  assert.equal(kinds.has("subtitle_downloaded"), true);
  assert.equal(kinds.has("metadata_hydration_started"), true);
  assert.equal(kinds.has("metadata_hydration_completed"), true);
  assert.equal(kinds.has("metadata_hydration_failed"), true);
});

test("file analyzed activity is debounced native-only refresh", () => {
  assert.deepEqual(titleOverviewReactiveRefreshPlan("file_analyzed", importKinds), {
    type: "refresh",
    downloadFeedback: false,
    mode: "bulk",
  });
});

test("subtitle activity is immediate native-only refresh", () => {
  assert.deepEqual(
    titleOverviewReactiveRefreshPlan("subtitle_downloaded", importKinds),
    {
      type: "refresh",
      downloadFeedback: false,
      mode: "immediate",
    },
  );
});

test("import lifecycle activity refreshes native and download feedback", () => {
  for (const kind of [
    "movie_downloaded",
    "series_episode_imported",
    "file_upgraded",
    "import_rejected",
  ]) {
    assert.deepEqual(titleOverviewReactiveRefreshPlan(kind, importKinds), {
      type: "refresh",
      downloadFeedback: true,
      mode: "immediate",
    });
  }
});

test("hydration activity reports UI-only transitions except completed", () => {
  assert.deepEqual(
    titleOverviewReactiveRefreshPlan(
      "metadata_hydration_started",
      importKinds,
    ),
    { type: "hydrationStarted" },
  );
  assert.deepEqual(
    titleOverviewReactiveRefreshPlan(
      "metadata_hydration_completed",
      importKinds,
    ),
    { type: "hydrationCompleted" },
  );
  assert.deepEqual(
    titleOverviewReactiveRefreshPlan("metadata_hydration_failed", importKinds),
    { type: "hydrationFailed" },
  );
});
