import assert from "node:assert/strict";
import test from "node:test";

import type { Translate } from "@/components/root/types";
import type { RouteCommandItem } from "@/components/common/route-command-types";
import type { MetadataTvdbSearchItem } from "@/lib/graphql/smg-queries";
import type { Facet, TitleRecord } from "@/lib/types";
import {
  buildCatalogSearchSections,
  buildGlobalSearchTabs,
  buildMetadataSearchActionState,
  buildMetadataResultCounts,
  countHiddenCatalogResults,
  countHiddenMetadataResults,
  countHiddenRouteCommandResults,
  countMetadataResults,
  GLOBAL_SEARCH_ALL_CATALOG_RESULT_LIMIT,
  GLOBAL_SEARCH_ALL_METADATA_RESULT_LIMIT,
  GLOBAL_SEARCH_ALL_ROUTE_COMMAND_DESKTOP_LIMIT,
  GLOBAL_SEARCH_ALL_ROUTE_COMMAND_LIMIT,
  getVisibleCatalogFacets,
  getVisibleCatalogResults,
  getVisibleMetadataResults,
  getVisibleRouteCommandResults,
} from "./global-search-model.ts";

const t: Translate = (key) => key;

function title(id: string, name: string, facet: Facet): TitleRecord {
  return {
    id,
    name,
    facet,
    libraryId: `${facet}-library`,
    monitored: true,
    tags: [],
  };
}

function metadata(name: string): MetadataTvdbSearchItem {
  return {
    tvdbId: `tvdb-${name}`,
    name,
    imdbId: null,
    slug: null,
    type: null,
    year: null,
    status: null,
    overview: null,
    popularity: null,
    posterUrl: null,
    language: null,
    runtimeMinutes: null,
    sortTitle: null,
  };
}

test("buildCatalogSearchSections buckets by facet and ranks query matches", () => {
  const sections = buildCatalogSearchSections(
    [
      title("m3", "The Green Mile", "movie"),
      title("a1", "Green Green", "anime"),
      title("m2", "Green Zone", "movie"),
      title("m1", "Green", "movie"),
      title("s1", "Greenleaf", "series"),
    ],
    "green",
  );

  assert.deepEqual(
    sections.movie.map((entry) => entry.id),
    ["m1", "m2", "m3"],
  );
  assert.deepEqual(
    sections.series.map((entry) => entry.id),
    ["s1"],
  );
  assert.deepEqual(
    sections.anime.map((entry) => entry.id),
    ["a1"],
  );
});

test("getVisibleCatalogResults interleaves all-tab library results and preserves type tabs", () => {
  const sections = buildCatalogSearchSections(
    [
      title("m1", "Movie One", "movie"),
      title("m2", "Movie Two", "movie"),
      title("s1", "Series One", "series"),
      title("a1", "Anime One", "anime"),
    ],
    "",
  );

  const allRows = getVisibleCatalogResults({
    activeTab: "all",
    canViewCatalog: true,
    catalogSearchSections: sections,
    visibleCatalogFacets: getVisibleCatalogFacets("all", true),
    allLimit: GLOBAL_SEARCH_ALL_CATALOG_RESULT_LIMIT,
  });

  assert.deepEqual(
    allRows.map(({ facet, title: entry }) => `${facet}:${entry.id}`),
    ["movie:m1", "series:s1", "anime:a1"],
  );
  assert.equal(countHiddenCatalogResults("all", 4, allRows), 1);

  const movieRows = getVisibleCatalogResults({
    activeTab: "movie",
    canViewCatalog: true,
    catalogSearchSections: sections,
    visibleCatalogFacets: getVisibleCatalogFacets("movie", true),
    allLimit: 1,
  });

  assert.deepEqual(
    movieRows.map(({ facet, title: entry }) => `${facet}:${entry.id}`),
    ["movie:m1", "movie:m2"],
  );
  assert.equal(countHiddenCatalogResults("movie", 2, movieRows), 0);
});

test("getVisibleRouteCommandResults previews commands in All and shows all commands in Actions", () => {
  const commands: RouteCommandItem[] = Array.from(
    { length: 8 },
    (_, index) => ({
      id: `command-${index}`,
      label: `Command ${index}`,
      description: `Command description ${index}`,
      onSelect: () => {},
    }),
  );

  assert.deepEqual(
    getVisibleRouteCommandResults("all", commands).map((command) => command.id),
    commands
      .slice(0, GLOBAL_SEARCH_ALL_ROUTE_COMMAND_LIMIT)
      .map((command) => command.id),
  );
  assert.deepEqual(
    getVisibleRouteCommandResults(
      "all",
      commands,
      GLOBAL_SEARCH_ALL_ROUTE_COMMAND_DESKTOP_LIMIT,
    ).map((command) => command.id),
    commands
      .slice(0, GLOBAL_SEARCH_ALL_ROUTE_COMMAND_DESKTOP_LIMIT)
      .map((command) => command.id),
  );
  assert.deepEqual(
    getVisibleRouteCommandResults("actions", commands).map(
      (command) => command.id,
    ),
    commands.map((command) => command.id),
  );
  assert.deepEqual(getVisibleRouteCommandResults("movie", commands), []);

  const allPreview = getVisibleRouteCommandResults("all", commands);
  assert.equal(
    countHiddenRouteCommandResults("all", commands, allPreview),
    commands.length - GLOBAL_SEARCH_ALL_ROUTE_COMMAND_LIMIT,
  );
  assert.equal(
    countHiddenRouteCommandResults(
      "all",
      commands,
      getVisibleRouteCommandResults(
        "all",
        commands,
        GLOBAL_SEARCH_ALL_ROUTE_COMMAND_DESKTOP_LIMIT,
      ),
    ),
    commands.length - GLOBAL_SEARCH_ALL_ROUTE_COMMAND_DESKTOP_LIMIT,
  );
  assert.equal(
    countHiddenRouteCommandResults(
      "actions",
      commands,
      getVisibleRouteCommandResults("actions", commands),
    ),
    0,
  );
  assert.equal(
    countHiddenRouteCommandResults("movie", commands, []),
    0,
  );
});

test("getVisibleMetadataResults previews rails in All and expands type tabs", () => {
  const results = Array.from({ length: 8 }, (_, index) => `result-${index}`);

  assert.deepEqual(
    getVisibleMetadataResults("all", results),
    results.slice(0, GLOBAL_SEARCH_ALL_METADATA_RESULT_LIMIT),
  );
  assert.deepEqual(getVisibleMetadataResults("movie", results), results);
  assert.deepEqual(getVisibleMetadataResults("library", results), []);
  assert.deepEqual(getVisibleMetadataResults("actions", results), []);

  const allPreview = getVisibleMetadataResults("all", results);
  assert.equal(
    countHiddenMetadataResults("all", results, allPreview),
    results.length - GLOBAL_SEARCH_ALL_METADATA_RESULT_LIMIT,
  );
  assert.equal(
    countHiddenMetadataResults("series", results, results),
    0,
  );
});

test("buildGlobalSearchTabs keeps catalog, metadata, and route command counts aligned", () => {
  const catalogSearchSections = buildCatalogSearchSections(
    [title("m1", "Movie One", "movie"), title("s1", "Series One", "series")],
    "",
  );
  const metadataResultCounts = buildMetadataResultCounts({
    movie: [metadata("Remote Movie")],
    series: [metadata("Remote Series"), metadata("Another Series")],
    anime: [],
  });
  const metadataResultCount = countMetadataResults(metadataResultCounts);

  const tabs = buildGlobalSearchTabs({
    canViewCatalog: true,
    catalogSearchSections,
    metadataResultCount,
    metadataResultCounts,
    routeCommandResultCount: 2,
    visibleCatalogResultCount: 2,
    t,
  });

  assert.deepEqual(
    tabs.map((tab) => [tab.key, tab.count]),
    [
      ["all", 7],
      ["library", 2],
      ["movie", 2],
      ["series", 3],
      ["anime", 0],
      ["actions", 2],
    ],
  );
});

test("buildGlobalSearchTabs hides the actions tab when no commands match", () => {
  const catalogSearchSections = buildCatalogSearchSections(
    [title("m1", "Movie One", "movie")],
    "",
  );
  const metadataResultCounts = buildMetadataResultCounts({
    movie: [],
    series: [],
    anime: [],
  });

  const tabs = buildGlobalSearchTabs({
    canViewCatalog: true,
    catalogSearchSections,
    metadataResultCount: 0,
    metadataResultCounts,
    routeCommandResultCount: 0,
    visibleCatalogResultCount: 1,
    t,
  });

  assert.equal(
    tabs.some((tab) => tab.key === "actions"),
    false,
  );
});

test("buildMetadataSearchActionState preserves add, request, cataloged, and unavailable behavior", () => {
  assert.deepEqual(
    buildMetadataSearchActionState({
      isInCatalog: true,
      canAdd: true,
      canRequest: true,
      resultName: "Cataloged",
      t,
    }),
    {
      isInCatalog: true,
      isUnavailable: false,
      opensRequestDialog: false,
      disabled: true,
      actionLabel: "search.alreadyCataloged",
      actionTitle: "search.alreadyCataloged: Cataloged",
      inlineActionLabel: "search.cataloged",
    },
  );

  assert.equal(
    buildMetadataSearchActionState({
      isInCatalog: false,
      canAdd: false,
      canRequest: true,
      resultName: "Requestable",
      t,
    }).opensRequestDialog,
    true,
  );

  assert.equal(
    buildMetadataSearchActionState({
      isInCatalog: false,
      canAdd: true,
      canRequest: false,
      resultName: "Addable",
      t,
    }).inlineActionLabel,
    "search.add",
  );

  assert.equal(
    buildMetadataSearchActionState({
      isInCatalog: false,
      canAdd: false,
      canRequest: false,
      resultName: "Unavailable",
      t,
    }).disabled,
    true,
  );
});
