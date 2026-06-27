import {
  filterRouteCommandItems,
  type RouteCommandItem,
} from "../common/route-command-types.ts";
import type { Translate } from "./types.ts";
import {
  FACET_REGISTRY,
  type FacetDefinition,
} from "../../lib/facets/registry.ts";
import type { MetadataSearchResults } from "../../lib/hooks/use-global-search.ts";
import type { Facet, TitleRecord } from "../../lib/types/titles.ts";

export type GlobalSearchTabKey = "all" | "library" | "actions" | Facet;

export const GLOBAL_SEARCH_ALL_CATALOG_RESULT_LIMIT = 4;
export const GLOBAL_SEARCH_ALL_METADATA_RESULT_LIMIT = 6;
export const GLOBAL_SEARCH_ALL_ROUTE_COMMAND_LIMIT = 4;
export const GLOBAL_SEARCH_ALL_ROUTE_COMMAND_DESKTOP_LIMIT = 6;

export type GlobalSearchTab = {
  key: GlobalSearchTabKey;
  label: string;
  count: number;
};

export type CatalogSearchSections = Record<Facet, TitleRecord[]>;

export type VisibleCatalogResult = {
  facet: Facet;
  title: TitleRecord;
};

export type MetadataSearchActionState = {
  isInCatalog: boolean;
  isUnavailable: boolean;
  opensRequestDialog: boolean;
  disabled: boolean;
  actionLabel: string;
  actionTitle: string;
  inlineActionLabel: string;
};

export function catalogFacetFromString(facet: string): Facet {
  return facet === "movie" ? "movie" : facet === "anime" ? "anime" : "series";
}

export function buildMetadataSearchActionState({
  isInCatalog,
  canAdd,
  canRequest,
  resultName,
  t,
}: {
  isInCatalog: boolean;
  canAdd: boolean;
  canRequest: boolean;
  resultName: string;
  t: Translate;
}): MetadataSearchActionState {
  const opensRequestDialog = !canAdd && canRequest;
  const isUnavailable = !isInCatalog && !canAdd && !canRequest;
  const disabled = isInCatalog || isUnavailable;
  const actionLabel = isInCatalog
    ? t("search.alreadyCataloged")
    : isUnavailable
      ? t("search.unavailable")
      : opensRequestDialog
        ? t("search.request")
        : t("search.configureAdd");
  const inlineActionLabel = isInCatalog
    ? t("search.cataloged")
    : isUnavailable
      ? t("search.unavailable")
      : opensRequestDialog
        ? t("search.request")
        : t("search.add");

  return {
    isInCatalog,
    isUnavailable,
    opensRequestDialog,
    disabled,
    actionLabel,
    actionTitle: `${actionLabel}: ${resultName}`,
    inlineActionLabel,
  };
}

export function buildCatalogSearchSections(
  catalogSearchResults: TitleRecord[],
  searchValue: string,
): CatalogSearchSections {
  const query = searchValue.trim().toLowerCase();
  const rank = (title: TitleRecord) => {
    const name = title.name.trim().toLowerCase();
    if (!query || name === query) return 0;
    if (name.startsWith(query)) return 1;
    const matchIndex = name.indexOf(query);
    return matchIndex >= 0 ? 2 + matchIndex : 3 + name.length;
  };

  const buckets = Object.fromEntries(
    FACET_REGISTRY.map((f) => [f.id, [] as TitleRecord[]]),
  ) as CatalogSearchSections;
  for (const title of catalogSearchResults) {
    buckets[catalogFacetFromString(title.facet)].push(title);
  }
  if (query) {
    for (const facet of FACET_REGISTRY) {
      buckets[facet.id].sort((a, b) => rank(a) - rank(b));
    }
  }
  return buckets;
}

export function buildMetadataResultCounts(
  metadataSearchResults: MetadataSearchResults,
): Record<Facet, number> {
  return Object.fromEntries(
    FACET_REGISTRY.map((f) => [
      f.id,
      (metadataSearchResults[f.metadataKey] ?? []).length,
    ]),
  ) as Record<Facet, number>;
}

export function countMetadataResults(
  metadataResultCounts: Record<Facet, number>,
): number {
  return FACET_REGISTRY.reduce(
    (total, f) => total + metadataResultCounts[f.id],
    0,
  );
}

export function filterGlobalSearchRouteCommands(
  routeCommandItems: RouteCommandItem[],
  searchValue: string,
): RouteCommandItem[] {
  if (searchValue.trim().length === 0) {
    return routeCommandItems;
  }
  return filterRouteCommandItems(routeCommandItems, searchValue);
}

export function getVisibleRouteCommandResults(
  activeTab: GlobalSearchTabKey,
  routeCommandResults: RouteCommandItem[],
  allLimit = GLOBAL_SEARCH_ALL_ROUTE_COMMAND_LIMIT,
): RouteCommandItem[] {
  if (activeTab === "all") {
    return routeCommandResults.slice(0, allLimit);
  }
  if (activeTab === "actions") {
    return routeCommandResults;
  }
  return [];
}

export function countHiddenRouteCommandResults(
  activeTab: GlobalSearchTabKey,
  routeCommandResults: RouteCommandItem[],
  visibleRouteCommandResults: RouteCommandItem[],
): number {
  if (activeTab !== "all") {
    return 0;
  }
  return Math.max(
    routeCommandResults.length - visibleRouteCommandResults.length,
    0,
  );
}

export function getVisibleMetadataResults<T>(
  activeTab: GlobalSearchTabKey,
  metadataResults: T[],
  allLimit = GLOBAL_SEARCH_ALL_METADATA_RESULT_LIMIT,
): T[] {
  if (activeTab === "all") {
    return metadataResults.slice(0, allLimit);
  }
  if (activeTab === "library" || activeTab === "actions") {
    return [];
  }
  return metadataResults;
}

export function countHiddenMetadataResults<T>(
  activeTab: GlobalSearchTabKey,
  metadataResults: T[],
  visibleMetadataResults: T[],
): number {
  if (activeTab !== "all") {
    return 0;
  }
  return Math.max(metadataResults.length - visibleMetadataResults.length, 0);
}

export function countHiddenCatalogResults(
  activeTab: GlobalSearchTabKey,
  visibleCatalogCount: number,
  visibleCatalogResults: VisibleCatalogResult[],
): number {
  if (activeTab !== "all") {
    return 0;
  }
  return Math.max(visibleCatalogCount - visibleCatalogResults.length, 0);
}

export function getVisibleCatalogFacets(
  activeTab: GlobalSearchTabKey,
  canViewCatalog: boolean,
): FacetDefinition[] {
  if (!canViewCatalog) {
    return [];
  }

  if (activeTab === "actions") {
    return [];
  }

  return activeTab === "all" || activeTab === "library"
    ? FACET_REGISTRY
    : FACET_REGISTRY.filter((f) => f.id === activeTab);
}

export function getVisibleMetadataFacets(
  activeTab: GlobalSearchTabKey,
): FacetDefinition[] {
  if (activeTab === "library" || activeTab === "actions") {
    return [];
  }

  return activeTab === "all"
    ? FACET_REGISTRY
    : FACET_REGISTRY.filter((f) => f.id === activeTab);
}

export function getMetadataSectionFacets({
  activeTab,
  metadataSearchLoading,
  metadataResultCounts,
}: {
  activeTab: GlobalSearchTabKey;
  metadataSearchLoading: boolean;
  metadataResultCounts: Record<Facet, number>;
}): FacetDefinition[] {
  const visibleMetadataFacets = getVisibleMetadataFacets(activeTab);
  return metadataSearchLoading
    ? visibleMetadataFacets
    : visibleMetadataFacets.filter((f) => metadataResultCounts[f.id] > 0);
}

export function countVisibleCatalogResults(
  visibleCatalogFacets: FacetDefinition[],
  catalogSearchSections: CatalogSearchSections,
): number {
  return visibleCatalogFacets.reduce(
    (total, f) => total + (catalogSearchSections[f.id]?.length ?? 0),
    0,
  );
}

export function getVisibleCatalogResults({
  activeTab,
  canViewCatalog,
  catalogSearchSections,
  visibleCatalogFacets,
  allLimit,
}: {
  activeTab: GlobalSearchTabKey;
  canViewCatalog: boolean;
  catalogSearchSections: CatalogSearchSections;
  visibleCatalogFacets: FacetDefinition[];
  allLimit: number;
}): VisibleCatalogResult[] {
  const picked: VisibleCatalogResult[] = [];
  if (!canViewCatalog) {
    return picked;
  }

  if (activeTab === "actions") {
    return picked;
  }

  if (activeTab !== "all" && activeTab !== "library") {
    return (catalogSearchSections[activeTab] ?? []).map((title) => ({
      facet: activeTab,
      title,
    }));
  }

  const indices: Record<string, number> = {};
  while (picked.length < allLimit) {
    let added = false;
    for (const f of visibleCatalogFacets) {
      if (picked.length >= allLimit) break;
      const bucket = catalogSearchSections[f.id] ?? [];
      const idx = indices[f.id] ?? 0;
      if (idx < bucket.length) {
        picked.push({ facet: f.id, title: bucket[idx] });
        indices[f.id] = idx + 1;
        added = true;
      }
    }
    if (!added) break;
  }
  return picked;
}

export function buildGlobalSearchTabs({
  canViewCatalog,
  catalogSearchSections,
  metadataResultCount,
  metadataResultCounts,
  routeCommandResultCount,
  visibleCatalogResultCount,
  t,
}: {
  canViewCatalog: boolean;
  catalogSearchSections: CatalogSearchSections;
  metadataResultCount: number;
  metadataResultCounts: Record<Facet, number>;
  routeCommandResultCount: number;
  visibleCatalogResultCount: number;
  t: Translate;
}): GlobalSearchTab[] {
  return [
    {
      key: "all",
      label: t("search.tabAll"),
      count:
        visibleCatalogResultCount +
        metadataResultCount +
        routeCommandResultCount,
    },
    ...(canViewCatalog
      ? [
          {
            key: "library" as GlobalSearchTabKey,
            label: t("search.tabLibrary"),
            count: visibleCatalogResultCount,
          },
        ]
      : []),
    ...FACET_REGISTRY.map((f) => ({
      key: f.id as GlobalSearchTabKey,
      label: t(f.navLabelKey),
      count:
        (canViewCatalog ? (catalogSearchSections[f.id]?.length ?? 0) : 0) +
        metadataResultCounts[f.id],
    })),
    ...(routeCommandResultCount > 0
      ? [
          {
            key: "actions" as GlobalSearchTabKey,
            label: t("search.actionsAndSettings"),
            count: routeCommandResultCount,
          },
        ]
      : []),
  ];
}
