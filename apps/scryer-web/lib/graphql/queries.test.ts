import assert from "node:assert/strict";
import test from "node:test";

import {
  buildTitlesQuery,
  buildReactiveRefreshQuery,
  calendarEpisodesQuery,
  downloadQueuePageQuery,
  downloadQueueSyncSubscription,
  episodeSidePanelDetailQuery,
  globalSearchInitQuery,
  movieSidePanelTitleQuery,
  movieSidePanelOverviewQuery,
  seriesSidePanelOverviewQuery,
  titleMoreLikeThisQuery,
  wantedNavigationCountsQuery,
} from "./queries.ts";

test("calendar hover query includes its artwork and synopsis fields", () => {
  assert.equal(calendarEpisodesQuery.includes("overview"), true);
  assert.equal(calendarEpisodesQuery.includes("imageUrl"), true);
});

test("calendar uses compact episode availability instead of querying media files", () => {
  assert.equal(calendarEpisodesQuery.includes("mediaAvailability"), true);
  assert.equal(calendarEpisodesQuery.includes("primaryQualityLabel"), true);
  assert.equal(calendarEpisodesQuery.includes("mediaFiles"), false);
});

test("wanted navigation loads every badge total without table rows", () => {
  assert.equal(wantedNavigationCountsQuery.includes("wantedItems("), true);
  assert.equal(
    wantedNavigationCountsQuery.includes("cutoffUnmetTitlesPage("),
    true,
  );
  assert.equal(wantedNavigationCountsQuery.includes("pendingReleases("), true);
  assert.equal(
    wantedNavigationCountsQuery.match(/\btotalCount\b/g)?.length,
    3,
  );
  assert.equal(wantedNavigationCountsQuery.includes("items {"), false);
});

test("activity queue uses paged cache reads and revision-only sync", () => {
  assert.equal(downloadQueuePageQuery.includes("downloadQueuePage("), true);
  assert.equal(downloadQueuePageQuery.includes("$limit: Int = 50"), true);
  assert.equal(downloadQueuePageQuery.includes("revision"), true);
  assert.equal(downloadQueuePageQuery.includes("stale"), true);
  assert.equal(downloadQueueSyncSubscription.includes("downloadQueueSync"), true);
  assert.equal(downloadQueueSyncSubscription.includes("items"), false);
});

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
  // Background art is part of the catalog list projection (poster/list cards
  // render it), so the reactive refresh fetches it for row parity.
  assert.equal(result.query.includes("backgroundUrl"), true);
  assert.equal(result.query.includes("backgroundSourceUrl"), true);
  assert.equal(result.query.includes("overview"), false);
  assert.equal(result.query.includes("canonicalTags"), false);
  assert.equal(result.query.includes("externalIds"), false);
  // Edit prefill and post-mutation verification consume option fields from
  // the same catalog-row projection, including reactive row refreshes.
  assert.equal(result.query.includes("qualityProfileId"), true);
  assert.equal(result.query.includes("monitorType"), true);
  assert.equal(result.query.includes("fillerPolicy"), true);
});

test("title catalog rows include option fields used by edit and mutation verification", () => {
  const query = buildTitlesQuery();

  assert.equal(query.includes("qualityProfileId"), true);
  assert.equal(query.includes("rootFolderId"), true);
  assert.equal(query.includes("monitorType"), true);
  assert.equal(query.includes("useSeasonFolders"), true);
  assert.equal(query.includes("monitorSpecials"), true);
  assert.equal(query.includes("interSeasonMovies"), true);
  assert.equal(query.includes("fillerPolicy"), true);
  assert.equal(query.includes("recapPolicy"), true);
});

test("global search loads the manageable library quality-profile override", () => {
  const manageableLibrariesSelection = globalSearchInitQuery.slice(
    globalSearchInitQuery.indexOf("manageableLibraries:"),
    globalSearchInitQuery.indexOf("requestableLibraries:"),
  );

  assert.equal(manageableLibrariesSelection.includes("qualityProfileId"), true);
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

test("title catalog page metadata includes the scoped managed-byte aggregate", () => {
  const query = buildTitlesQuery();

  assert.equal(query.includes("managedBytes"), true);
});

test("reactive movie side panel refresh omits acquisition diagnostics", () => {
  const result = buildReactiveRefreshQuery([
    {
      key: "titleSidePanelOverview:title-1:300:movie",
      kind: "titleSidePanelOverview",
      titleId: "title-1",
      blocklistLimit: 300,
      projection: "MOVIE",
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

test("movie side panel overview includes acquisition diagnostics", () => {
  assert.equal(movieSidePanelOverviewQuery.includes("titleAcquisitionDiagnostics"), true);
});

test("movie side panel title query includes files without overview extras", () => {
  assert.equal(movieSidePanelTitleQuery.includes("mediaFiles {"), true);
  assert.equal(
    movieSidePanelTitleQuery.includes("titleAcquisitionDiagnostics"),
    false,
  );
  assert.equal(movieSidePanelTitleQuery.includes("titleHistory("), false);
  assert.equal(movieSidePanelTitleQuery.includes("titleReleaseBlocklist("), false);
  assert.equal(movieSidePanelTitleQuery.includes("externalSubtitles("), false);
  assert.equal(movieSidePanelTitleQuery.includes("setupStatus"), false);
});

test("side panel queries omit recommendations", () => {
  assert.equal(movieSidePanelOverviewQuery.includes("moreLikeThis("), false);
  assert.equal(seriesSidePanelOverviewQuery.includes("moreLikeThis("), false);
});

test("title more-like-this query fetches full discovery item detail", () => {
  assert.equal(titleMoreLikeThisQuery.includes("moreLikeThis(limit: $limit)"), true);
  // Card actions feed metadataResultForDiscoveryItem directly instead of
  // issuing a follow-up discoveryItemDetail fetch, so the strip query must
  // carry the full detail projection.
  assert.equal(titleMoreLikeThisQuery.includes("externalRatings"), true);
  assert.equal(titleMoreLikeThisQuery.includes("externalIds"), true);
  assert.equal(titleMoreLikeThisQuery.includes("canonicalTags"), true);
  assert.equal(titleMoreLikeThisQuery.includes("targetKey"), true);
  assert.equal(titleMoreLikeThisQuery.includes("ownedInInput"), true);
});

test("reactive side panel refresh omits recommendations", () => {
  const result = buildReactiveRefreshQuery([
    {
      key: "titleSidePanelOverview:title-1:300:movie",
      kind: "titleSidePanelOverview",
      titleId: "title-1",
      blocklistLimit: 300,
      projection: "MOVIE",
    },
  ]);

  assert.equal(result.query.includes("moreLikeThis("), false);
});

test("series side panel overview uses compact media availability for collapsed rows", () => {
  assert.equal(seriesSidePanelOverviewQuery.includes("aliases"), false);
  assert.equal(seriesSidePanelOverviewQuery.includes("mediaAvailability {"), true);
  assert.equal(seriesSidePanelOverviewQuery.includes("primaryQualityLabel"), true);
  assert.equal(seriesSidePanelOverviewQuery.includes("mediaFiles {"), false);
  assert.equal(seriesSidePanelOverviewQuery.includes("wantedItems"), false);
  assert.equal(seriesSidePanelOverviewQuery.includes("titleHistory("), false);
  assert.equal(seriesSidePanelOverviewQuery.includes("titleAcquisitionDiagnostics"), false);
  assert.equal(seriesSidePanelOverviewQuery.includes("overview"), true);
  assert.equal(/episodes\s*\{[^}]*\boverview\b/s.test(seriesSidePanelOverviewQuery), false);
  assert.equal(/episodes\s*\{[^}]*\bimageUrl\b/s.test(seriesSidePanelOverviewQuery), false);
  assert.equal(seriesSidePanelOverviewQuery.includes("sizeBytes"), false);
  assert.equal(seriesSidePanelOverviewQuery.includes("qualityLabel"), false);
});

test("series reactive side panel refresh uses compact media availability", () => {
  const result = buildReactiveRefreshQuery([
    {
      key: "titleSidePanelOverview:title-1:300:series",
      kind: "titleSidePanelOverview",
      titleId: "title-1",
      blocklistLimit: 300,
      projection: "SERIES",
    },
  ]);

  assert.equal(result.query.includes("mediaAvailability {"), true);
  assert.equal(result.query.includes("mediaFiles {"), false);
  assert.equal(result.query.includes("episodeMediaFiles("), false);
  assert.equal(result.query.includes("titleHistory("), false);
  assert.equal(result.query.includes("titleAcquisitionDiagnostics"), false);
});

test("movie side panel overview still includes title media files", () => {
  assert.equal(movieSidePanelOverviewQuery.includes("mediaFiles {"), true);
});

test("episode side panel detail query loads nested media files", () => {
  assert.equal(episodeSidePanelDetailQuery.includes("episode("), true);
  assert.equal(episodeSidePanelDetailQuery.includes("episodeMediaFiles("), false);
  assert.equal(episodeSidePanelDetailQuery.includes("overview"), true);
  assert.equal(episodeSidePanelDetailQuery.includes("imageUrl"), true);
  assert.equal(episodeSidePanelDetailQuery.includes("mediaAvailability {"), true);
  assert.equal(episodeSidePanelDetailQuery.includes("primaryQualityLabel"), true);
  assert.equal(episodeSidePanelDetailQuery.includes("mediaFiles {"), true);
  assert.equal(episodeSidePanelDetailQuery.includes("filePath"), true);
});

test("overview queries do not export old native or panel-detail documents", async () => {
  const queries = await import("./queries.ts");

  assert.equal(Object.hasOwn(queries, "seriesTitlePanelDetailQuery"), false);
  assert.equal(Object.hasOwn(queries, "titleOverviewNativeQuery"), false);
  assert.equal(Object.hasOwn(queries, "seriesTitleOverviewNativeQuery"), false);
  assert.equal(Object.hasOwn(queries, "titlePanelDetailQuery"), false);
  assert.equal(Object.hasOwn(queries, "episodeMediaFilesQuery"), false);
});
