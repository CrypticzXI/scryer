import assert from "node:assert/strict";
import test from "node:test";

import {
  buildTitlesQuery,
  buildReactiveRefreshQuery,
  titleOverviewNativeQuery,
} from "./queries.ts";

test("reactive catalog title refresh uses catalog list projection", () => {
  const result = buildReactiveRefreshQuery([
    {
      key: "catalogTitle:title-1",
      kind: "catalogTitle",
      titleId: "title-1",
      projection: {
        episodes: true,
        runtime: true,
      },
    },
  ]);

  assert.equal(result.query.includes("title(id:"), true);
  assert.equal(result.query.includes("episodesOwned"), true);
  assert.equal(result.query.includes("episodesMonitored"), true);
  assert.equal(result.query.includes("episodesTotal"), true);
  assert.equal(result.query.includes("runtimeMinutes"), true);
  assert.equal(result.query.includes("overview"), false);
  assert.equal(result.query.includes("backgroundUrl"), false);
  assert.equal(result.query.includes("backgroundSourceUrl"), false);
  assert.equal(result.query.includes("canonicalTags"), false);
  assert.equal(result.query.includes("externalIds"), false);
  assert.equal(result.query.includes("monitorType"), false);
});

test("reactive catalog title refresh omits episodic fields by default", () => {
  const result = buildReactiveRefreshQuery([
    {
      key: "catalogTitle:title-1",
      kind: "catalogTitle",
      titleId: "title-1",
    },
  ]);

  assert.equal(result.query.includes("posterUrl"), true);
  assert.equal(result.query.includes("posterSourceUrl"), true);
  assert.equal(result.query.includes("metadataFetchedAt"), true);
  assert.equal(result.query.includes("episodesOwned"), false);
  assert.equal(result.query.includes("episodesMonitored"), false);
  assert.equal(result.query.includes("episodesTotal"), false);
});

test("title catalog query can omit page metadata for quiet refreshes", () => {
  const query = buildTitlesQuery({}, { includePageMetadata: false });

  assert.equal(query.includes("items {"), true);
  assert.equal(query.includes("hasMore"), false);
  assert.equal(query.includes("totalCount"), false);
  assert.equal(query.includes("filterCounts"), false);
});

test("reactive title overview native refresh omits acquisition diagnostics", () => {
  const result = buildReactiveRefreshQuery([
    {
      key: "titleOverviewNative:title-1:300",
      kind: "titleOverviewNative",
      titleId: "title-1",
      blocklistLimit: 300,
    },
  ]);

  assert.equal(result.query.includes("titleAcquisitionDiagnostics"), false);
  assert.equal(result.query.includes("title(id:"), true);
  assert.equal(result.query.includes("titleHistory("), true);
  assert.equal(result.query.includes("titleReleaseBlocklist("), true);
  assert.equal(result.query.includes("externalSubtitles("), true);
  assert.equal(result.query.includes("setupStatus"), true);
  assert.equal(
    Object.hasOwn(result.actionPlans[0] ?? {}, "titleAcquisitionDiagnosticsAlias"),
    false,
  );
});

test("full title overview native loader still includes acquisition diagnostics", () => {
  assert.equal(titleOverviewNativeQuery.includes("titleAcquisitionDiagnostics"), true);
});
