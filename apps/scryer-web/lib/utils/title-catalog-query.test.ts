import assert from "node:assert/strict";
import test from "node:test";

import {
  EMPTY_TITLE_QUICK_FILTERS,
  buildTitleCatalogQueryVariables,
  titleCatalogQueryKey,
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
