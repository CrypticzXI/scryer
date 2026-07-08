import assert from "node:assert/strict";
import test from "node:test";

import {
  buildTitlesQuery,
  buildReactiveRefreshQuery,
  episodeMediaFilesQuery,
  seriesTitleOverviewNativeQuery,
  titlePanelDetailQuery,
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

test("series title overview native loader omits title media files", () => {
  assert.equal(seriesTitleOverviewNativeQuery.includes("mediaFiles {"), false);
});

test("series reactive title overview native refresh omits title media files", () => {
  const result = buildReactiveRefreshQuery([
    {
      key: "titleOverviewNative:title-1:300",
      kind: "titleOverviewNative",
      titleId: "title-1",
      blocklistLimit: 300,
      projection: "series",
    },
  ]);

  assert.equal(result.query.includes("mediaFiles {"), false);
  assert.equal(result.query.includes("episodeMediaFiles("), false);
});

test("movie title overview native loader still includes title media files", () => {
  assert.equal(titleOverviewNativeQuery.includes("mediaFiles {"), true);
});

test("episode media files query remains the series file detail path", () => {
  assert.equal(episodeMediaFilesQuery.includes("episodeMediaFiles("), true);
  assert.equal(episodeMediaFilesQuery.includes("filePath"), true);
});

test("series overview does not expose a selected panel detail document", async () => {
  const queries = await import("./queries.ts");

  assert.equal(Object.hasOwn(queries, "seriesTitlePanelDetailQuery"), false);
  assert.equal(seriesTitleOverviewNativeQuery.includes("SeriesTitleOverviewNative"), true);
  assert.equal(titlePanelDetailQuery.includes("TitlePanelDetail"), true);
});
