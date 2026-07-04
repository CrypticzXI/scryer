import assert from "node:assert/strict";
import test from "node:test";

import {
  EMPTY_TITLE_QUICK_FILTERS,
  buildTitleCatalogQueryVariables,
  titleCatalogProjectionForTable,
  titleCatalogQueryKey,
  titleCatalogSortInput,
} from "./title-catalog-query.ts";

test("title catalog variables include quick filters like 0.16.6", () => {
  const variables = buildTitleCatalogQueryVariables({
    facet: "series",
    libraryIds: ["series-main"],
    query: " Fringe ",
    filters: {
      monitored: true,
      unmonitored: false,
      continuing: true,
      ended: false,
    },
    sort: { key: "name", direction: "asc" },
    limit: 300,
    offset: 0,
  });

  assert.deepEqual(variables, {
    facet: "series",
    libraryIds: ["series-main"],
    query: "Fringe",
    filter: {
      monitored: true,
      contentStatuses: ["continuing"],
    },
    sort: { key: "title", direction: "asc" },
    limit: 300,
    offset: 0,
  });
});

test("title catalog variables send all libraries as null", () => {
  const variables = buildTitleCatalogQueryVariables({
    facet: "movie",
    libraryIds: [],
    query: "",
    filters: EMPTY_TITLE_QUICK_FILTERS,
    sort: { key: "added", direction: "desc" },
    limit: 300,
    offset: 0,
  });

  assert.equal(variables.libraryIds, null);
  assert.equal(variables.query, null);
  assert.equal(variables.filter, null);
  assert.deepEqual(variables.sort, { key: "added", direction: "desc" });
});

test("title catalog query key changes when quick filters change", () => {
  const base = {
    facet: "anime",
    query: "",
    libraryIds: [],
    sort: { key: "name", direction: "asc" },
  };

  const unfilteredKey = titleCatalogQueryKey({
    ...base,
    filters: EMPTY_TITLE_QUICK_FILTERS,
  });
  const filteredKey = titleCatalogQueryKey({
    ...base,
    filters: {
      ...EMPTY_TITLE_QUICK_FILTERS,
      ended: true,
    },
  });

  assert.notEqual(unfilteredKey, filteredKey);
});

test("title catalog sort input maps optional table columns", () => {
  assert.deepEqual(titleCatalogSortInput({ key: "runtime", direction: "desc" }), {
    key: "runtime",
    direction: "desc",
  });
  assert.deepEqual(
    titleCatalogSortInput({ key: "ratingMetacriticUser", direction: "desc" }),
    {
      key: "rating_metacritic_user",
      direction: "desc",
    },
  );
  assert.deepEqual(
    titleCatalogSortInput({ key: "audioCodec", direction: "asc" }),
    {
      key: "media_audio_codec",
      direction: "asc",
    },
  );
});

test("title catalog projection requests only visible or sorted optional fields", () => {
  const movieProjection = titleCatalogProjectionForTable({
    facet: "movie",
    visibleColumns: {
      resolution: true,
      ratingImdb: true,
      episodes: true,
    },
    sort: { key: "popularity", direction: "desc" },
  });

  assert.equal(movieProjection.movieMedia, true);
  assert.equal(movieProjection.ratings, true);
  assert.equal(movieProjection.popularity, true);
  assert.equal(movieProjection.episodes, false);

  const seriesProjection = titleCatalogProjectionForTable({
    facet: "series",
    visibleColumns: {
      runtime: true,
      episodes: true,
      popularity: true,
      hdr: true,
    },
    sort: { key: "name", direction: "asc" },
  });

  assert.equal(seriesProjection.runtime, true);
  assert.equal(seriesProjection.episodes, true);
  assert.equal(seriesProjection.popularity, false);
  assert.equal(seriesProjection.movieMedia, false);
});

test("title catalog projection ignores ratings unsupported by active facet", () => {
  const projection = titleCatalogProjectionForTable({
    facet: "series",
    visibleColumns: {
      ratingAnilist: true,
      ratingAnidb: true,
    },
    sort: { key: "ratingAnilist", direction: "desc" },
  });

  assert.equal(projection.ratings, false);
});

test("title catalog query key changes when projection changes", () => {
  const base = {
    facet: "movie",
    query: "",
    libraryIds: [],
    filters: EMPTY_TITLE_QUICK_FILTERS,
    sort: { key: "name", direction: "asc" },
  };

  const baseKey = titleCatalogQueryKey({
    ...base,
    projection: titleCatalogProjectionForTable({
      facet: "movie",
      visibleColumns: {},
      sort: base.sort,
    }),
  });
  const projectedKey = titleCatalogQueryKey({
    ...base,
    projection: titleCatalogProjectionForTable({
      facet: "movie",
      visibleColumns: { ratingImdb: true },
      sort: base.sort,
    }),
  });

  assert.notEqual(baseKey, projectedKey);
});
