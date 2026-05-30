import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useClient } from "urql";
import type { MetadataTvdbSearchItem } from "@/lib/graphql/smg-queries";
import type { ExternalId, Facet, TitleRecord } from "@/lib/types";
import type { ViewCategoryId } from "@/lib/types/quality-profiles";
import type { LocaleCode } from "@/lib/i18n";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import {
  catalogSearchTitlesQuery,
  globalSearchInitQuery,
  metadataMovieQuery,
  metadataSeriesQuery,
  requestableLibrariesQuery,
  searchMetadataMultiQuery,
  searchMetadataQuery,
  titlesByExternalIdsQuery,
} from "@/lib/graphql/queries";
import {
  isAbortError,
  makeAbortableFetch,
} from "@/lib/graphql/urql-client";
import { addTitleMutation, submitMediaRequestMutation } from "@/lib/graphql/mutations";
import {
  ANIME_INTER_SEASON_MOVIES_KEY,
  ANIME_MONITOR_SPECIALS_KEY,
  QUALITY_PROFILE_CATALOG_KEY,
  QUALITY_PROFILE_ID_KEY,
  QUALITY_PROFILE_INHERIT_VALUE,
} from "@/lib/constants/settings";
import {
  coerceProfileSetting,
  qualityProfileSettingsToCategoryOverrides,
} from "@/lib/utils/quality-profiles";
import { FACET_REGISTRY, facetById } from "@/lib/facets/registry";
import { useSettingsSubscription } from "@/lib/hooks/use-settings-subscription";

export type MetadataSearchResults = Record<string, MetadataTvdbSearchItem[]>;

export type CatalogQualityProfileOption = {
  id: string;
  name: string;
};

export type MetadataCatalogMonitorType =
  | "monitored"
  | "unmonitored"
  | "futureEpisodes"
  | "missingAndFutureEpisodes"
  | "allEpisodes"
  | "none";

export type { RootFolderOption } from "@/lib/types/titles";
import type { LibraryRecord, RootFolderOption } from "@/lib/types/titles";

export type MetadataCatalogAddOptions = {
  libraryId?: string;
  qualityProfileId: string;
  seasonFolder: boolean;
  monitorType: MetadataCatalogMonitorType;
  minAvailability?: string;
  monitorSpecials?: boolean;
  interSeasonMovies?: boolean;
  rootFolder?: string;
};

export type MetadataCatalogRequestOptions = {
  libraryId: string;
};

export type AnimeCatalogDefaults = {
  monitorSpecials: boolean;
  interSeasonMovies: boolean;
};

function isMetadataEmpty(results: MetadataSearchResults): boolean {
  return Object.values(results).every((arr) => arr.length === 0);
}

function normalizeOrderedLookupValues(values: string[]): string[] {
  const seen = new Set<string>();
  const normalized: string[] = [];
  for (const value of values) {
    const trimmed = value.trim();
    if (!trimmed || seen.has(trimmed)) {
      continue;
    }
    seen.add(trimmed);
    normalized.push(trimmed);
  }
  return normalized;
}

function titleTvdbIds(title: TitleRecord): string[] {
  return (title.externalIds ?? [])
    .filter((externalId) => externalId.source.toLowerCase() === "tvdb")
    .map((externalId) => externalId.value.trim())
    .filter(Boolean);
}

function buildCatalogTitleLookupByTvdbId(titles: TitleRecord[]): Record<string, TitleRecord> {
  const lookup: Record<string, TitleRecord> = {};
  for (const title of titles) {
    for (const tvdbId of titleTvdbIds(title)) {
      if (!(tvdbId in lookup)) {
        lookup[tvdbId] = title;
      }
    }
  }
  return lookup;
}

function metadataResultTvdbId(result: MetadataTvdbSearchItem): string {
  return String(result.tvdbId).trim();
}

function isMetadataResultCataloged(
  lookup: Record<string, TitleRecord>,
  result: MetadataTvdbSearchItem,
): boolean {
  const tvdbId = metadataResultTvdbId(result);
  return tvdbId !== "" && lookup[tvdbId] !== undefined;
}

function filterCatalogedMetadataResults(
  results: MetadataTvdbSearchItem[],
  lookup: Record<string, TitleRecord>,
): MetadataTvdbSearchItem[] {
  return results.filter((result) => !isMetadataResultCataloged(lookup, result));
}

function mergeCatalogResults(
  prioritized: TitleRecord[],
  fallback: TitleRecord[],
): TitleRecord[] {
  const merged: TitleRecord[] = [];
  const seen = new Set<string>();
  for (const title of [...prioritized, ...fallback]) {
    if (seen.has(title.id)) {
      continue;
    }
    seen.add(title.id);
    merged.push(title);
  }
  return merged;
}

function sameExternalIds(
  previous: ExternalId[] | null | undefined,
  next: ExternalId[] | null | undefined,
): boolean {
  const previousIds = previous ?? [];
  const nextIds = next ?? [];
  return (
    previousIds.length === nextIds.length &&
    previousIds.every(
      (item, index) =>
        item.source === nextIds[index]?.source &&
        item.value === nextIds[index]?.value,
    )
  );
}

function sameTitleList(
  previous: TitleRecord[],
  next: TitleRecord[],
): boolean {
  return (
    previous.length === next.length &&
    previous.every((item, index) => {
      const nextItem = next[index];
      return (
        nextItem !== undefined &&
        item.id === nextItem.id &&
        item.name === nextItem.name &&
        item.facet === nextItem.facet &&
        item.monitored === nextItem.monitored &&
        (item.slug ?? null) === (nextItem.slug ?? null) &&
        (item.year ?? null) === (nextItem.year ?? null) &&
        (item.posterUrl ?? null) === (nextItem.posterUrl ?? null) &&
        (item.posterSourceUrl ?? null) === (nextItem.posterSourceUrl ?? null) &&
        (item.metadataFetchedAt ?? null) === (nextItem.metadataFetchedAt ?? null) &&
        sameExternalIds(item.externalIds, nextItem.externalIds)
      );
    })
  );
}

function sameCatalogLookup(
  previous: Record<string, TitleRecord>,
  next: Record<string, TitleRecord>,
): boolean {
  const previousKeys = Object.keys(previous);
  const nextKeys = Object.keys(next);
  return (
    previousKeys.length === nextKeys.length &&
    previousKeys.every((key) => previous[key]?.id === next[key]?.id)
  );
}

const AUTOCOMPLETE_MIN_CHARS = 2;
const AUTOCOMPLETE_DEBOUNCE_MS = 250;
const AUTOCOMPLETE_LIMIT = 10;

type UseGlobalSearchArgs = {
  queueFacet: Facet;
  uiLanguage: LocaleCode;
};

export interface UseGlobalSearchResult {
  globalSearch: string;
  setGlobalSearch: (value: string) => void;
  globalSearchInputRef: React.RefObject<HTMLInputElement | null>;
  searching: boolean;
  catalogSearchLoading: boolean;
  metadataSearchLoading: boolean;
  tvdbCandidates: MetadataTvdbSearchItem[];
  runTvdbSearch: (query: string) => Promise<MetadataTvdbSearchItem[]>;
  forceSearchGlobal: () => Promise<void>;
  setTvdbCandidates: (value: MetadataTvdbSearchItem[]) => void;
  catalogSearchResults: TitleRecord[];
  metadataSearchResults: MetadataSearchResults;
  isGlobalSearchPanelOpen: boolean;
  openGlobalSearchPanel: (force?: boolean) => void;
  closeGlobalSearchPanel: () => void;
  resetGlobalSearch: () => void;
  catalogQualityProfileOptions: CatalogQualityProfileOption[];
  catalogConfigLoading: boolean;
  ensureCatalogConfigReady: (facet: Facet) => Promise<void>;
  isCatalogConfigReady: (facet: Facet) => boolean;
  resolveDefaultQualityProfileIdForFacet: (facet: Facet) => string;
  animeCatalogDefaults: AnimeCatalogDefaults;
  addMetadataSearchResultToCatalog: (
    result: MetadataTvdbSearchItem,
    facet: Facet,
    options: MetadataCatalogAddOptions,
  ) => Promise<string | null>;
  requestMetadataSearchResult: (
    result: MetadataTvdbSearchItem,
    facet: Facet,
    options: MetadataCatalogRequestOptions,
  ) => Promise<boolean>;
  isMetadataSearchResultInCatalog: (
    facet: Facet,
    result: MetadataTvdbSearchItem,
  ) => boolean;
  rootFoldersByFacet: Record<Facet, RootFolderOption[]>;
  librariesByFacet: Record<Facet, LibraryRecord[]>;
  requestableLibrariesByFacet: Record<Facet, LibraryRecord[]>;
  queueFacet: Facet;
  setQueueFacet: (value: Facet) => void;
  catalogChangeSignal: number;
}

function monitorTypeToMonitored(monitorType: MetadataCatalogMonitorType): boolean {
  return monitorType !== "unmonitored" && monitorType !== "none";
}

function normalizeCatalogAddRequestKey(
  facet: Facet,
  externalIds: ExternalId[],
): string {
  const normalizedIds = [...externalIds]
    .map((externalId) => ({
      source: externalId.source.trim().toLowerCase(),
      value: externalId.value.trim(),
    }))
    .filter((externalId) => externalId.source && externalId.value)
    .sort((left, right) => {
      const sourceCompare = left.source.localeCompare(right.source);
      if (sourceCompare !== 0) {
        return sourceCompare;
      }
      return left.value.localeCompare(right.value);
    })
    .map((externalId) => `${externalId.source}:${externalId.value}`)
    .join("|");

  return `${facet}|${normalizedIds}`;
}

function librariesByFacetFromList(libraries: LibraryRecord[]): Record<Facet, LibraryRecord[]> {
  return libraries.reduce(
    (acc: Record<Facet, LibraryRecord[]>, library: LibraryRecord) => {
      acc[library.facet]?.push(library);
      return acc;
    },
    { movie: [], series: [], anime: [] },
  );
}

function sameLibrariesByFacet(
  previous: Record<Facet, LibraryRecord[]>,
  next: Record<Facet, LibraryRecord[]>,
): boolean {
  return (["movie", "series", "anime"] as Facet[]).every((facet) => {
    const previousFacetLibraries = previous[facet];
    const nextFacetLibraries = next[facet];
    return (
      previousFacetLibraries.length === nextFacetLibraries.length &&
      previousFacetLibraries.every((entry, index) => {
        const candidate = nextFacetLibraries[index];
        return (
          candidate &&
          entry.id === candidate.id &&
          entry.name === candidate.name &&
          entry.slug === candidate.slug &&
          entry.roots.length === candidate.roots.length
        );
      })
    );
  });
}

export function useGlobalSearch({
  queueFacet: initialQueueFacet,
  uiLanguage,
}: UseGlobalSearchArgs): UseGlobalSearchResult {
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const client = useClient();
  const [queueFacet, setQueueFacet] = useState<Facet>(initialQueueFacet);
  const catalogChangeSignal = 0;
  const sortByRelevance = useCallback((results: MetadataTvdbSearchItem[], query: string) => {
    const q = query.trim().toLowerCase();

    function score(item: MetadataTvdbSearchItem): number {
      const name = (item.name || "").toLowerCase();
      const pop = Math.max(item.popularity ?? 0, 1);
      if (name === q) return 1e9 + pop;
      if (name.startsWith(q)) return pop * 5;
      if (name.includes(q)) return pop * 3;
      return pop;
    }

    return [...results].sort((left, right) => {
      const ls = score(left);
      const rs = score(right);
      if (ls !== rs) return rs - ls;
      return (right.year ?? 0) - (left.year ?? 0);
    });
  }, []);

  const [globalSearch, setGlobalSearch] = useState("");
  const globalSearchInputRef = useRef<HTMLInputElement>(null);
  const [searching, setSearching] = useState(false);
  const [catalogSearchLoading, setCatalogSearchLoading] = useState(false);
  const [metadataSearchLoading, setMetadataSearchLoading] = useState(false);
  const [tvdbCandidates, setTvdbCandidates] = useState<MetadataTvdbSearchItem[]>([]);
  const [catalogSearchResults, setCatalogSearchResults] = useState<TitleRecord[]>([]);
  const [catalogTitlesByTvdbId, setCatalogTitlesByTvdbId] = useState<
    Record<string, TitleRecord>
  >({});
  const [metadataSearchResults, setMetadataSearchResults] = useState<MetadataSearchResults>(
    () => Object.fromEntries(FACET_REGISTRY.map((f) => [f.metadataKey, []])),
  );
  const [catalogQualityProfileOptions, setCatalogQualityProfileOptions] = useState<
    CatalogQualityProfileOption[]
  >([]);
  const [globalQualityProfileId, setGlobalQualityProfileId] = useState<string>(
    QUALITY_PROFILE_INHERIT_VALUE,
  );
  const [animeCatalogDefaults, setAnimeCatalogDefaults] = useState<AnimeCatalogDefaults>({
    monitorSpecials: true,
    interSeasonMovies: true,
  });
  const [categoryQualityProfileOverrides, setCategoryQualityProfileOverrides] = useState<
    Record<ViewCategoryId, string>
  >(
    () => Object.fromEntries(FACET_REGISTRY.map((f) => [f.scopeId, QUALITY_PROFILE_INHERIT_VALUE])) as Record<ViewCategoryId, string>,
  );
  const [isGlobalSearchPanelOpen, setIsGlobalSearchPanelOpen] = useState(false);
  const [catalogConfigLoading, setCatalogConfigLoading] = useState(false);
  const [rootFoldersByFacet, setRootFoldersByFacet] = useState<Record<Facet, RootFolderOption[]>>(
    () => ({ movie: [], series: [], anime: [] }),
  );
  const [librariesByFacet, setLibrariesByFacet] = useState<Record<Facet, LibraryRecord[]>>(
    () => ({ movie: [], series: [], anime: [] }),
  );
  const [requestableLibrariesByFacet, setRequestableLibrariesByFacet] = useState<
    Record<Facet, LibraryRecord[]>
  >(() => ({ movie: [], series: [], anime: [] }));
  const forcedOpenRef = useRef(false);
  const autocompleteRequestId = useRef(0);
  const autocompleteAbortRef = useRef<AbortController | null>(null);
  const pendingCatalogAddKeysRef = useRef<Set<string>>(new Set());
  const pendingRequestKeysRef = useRef<Set<string>>(new Set());
  const catalogConfigRefreshPromiseRef = useRef<Promise<void> | null>(null);

  const cancelAutocomplete = useCallback(() => {
    autocompleteRequestId.current += 1;
    autocompleteAbortRef.current?.abort();
    autocompleteAbortRef.current = null;
    setSearching(false);
    setCatalogSearchLoading(false);
    setMetadataSearchLoading(false);
  }, []);

  const catalogQualityProfileIdSet = useMemo(
    () => new Set(catalogQualityProfileOptions.map((profile) => profile.id)),
    [catalogQualityProfileOptions],
  );

  const resolveDefaultQualityProfileIdForFacet = useCallback(
    (facet: Facet) => {
      const scopeId = facetById(facet)?.scopeId ?? "movie";
      const overrideProfileId = coerceProfileSetting(
        categoryQualityProfileOverrides[scopeId],
      );
      if (
        overrideProfileId &&
        overrideProfileId !== QUALITY_PROFILE_INHERIT_VALUE &&
        catalogQualityProfileIdSet.has(overrideProfileId)
      ) {
        return overrideProfileId;
      }

      const normalizedGlobalProfileId = coerceProfileSetting(globalQualityProfileId);
      if (
        normalizedGlobalProfileId &&
        normalizedGlobalProfileId !== QUALITY_PROFILE_INHERIT_VALUE &&
        catalogQualityProfileIdSet.has(normalizedGlobalProfileId)
      ) {
        return normalizedGlobalProfileId;
      }

      return catalogQualityProfileOptions[0]?.id ?? "";
    },
    [
      catalogQualityProfileIdSet,
      catalogQualityProfileOptions,
      categoryQualityProfileOverrides,
      globalQualityProfileId,
    ],
  );

  const isCatalogConfigReady = useCallback(
    (facet: Facet) =>
      requestableLibrariesByFacet[facet].length > 0 ||
      (catalogQualityProfileOptions.length > 0 &&
        (librariesByFacet[facet].length > 0 || rootFoldersByFacet[facet].length > 0)),
    [
      catalogQualityProfileOptions,
      librariesByFacet,
      requestableLibrariesByFacet,
      rootFoldersByFacet,
    ],
  );

  const refreshCatalogQualityProfileState = useCallback(async () => {
    if (catalogConfigRefreshPromiseRef.current) {
      return catalogConfigRefreshPromiseRef.current;
    }

    const refreshPromise = (async () => {
      setCatalogConfigLoading(true);
      try {
        const { data, error } = await client
          .query(globalSearchInitQuery, {}, { requestPolicy: "network-only" })
          .toPromise();
        if (error) throw error;

        const parsedProfiles = (data.qualityProfileSettings?.profiles ?? []).map(
          (profile: { id: string; name: string }) => ({
            id: profile.id.trim(),
            name: profile.name.trim() || profile.id.trim(),
          }),
        );

        setCatalogQualityProfileOptions((previous) =>
          previous.length === parsedProfiles.length &&
          previous.every(
            (item, index) =>
              item.id === parsedProfiles[index]?.id &&
              item.name === parsedProfiles[index]?.name,
          )
            ? previous
            : parsedProfiles,
        );

        const nextGlobalProfileId = coerceProfileSetting(
          data.qualityProfileSettings?.globalProfileId ?? "",
        );
        setGlobalQualityProfileId((previous) =>
          previous === nextGlobalProfileId ? previous : nextGlobalProfileId,
        );

        const nextOverrides: Record<ViewCategoryId, string> =
          qualityProfileSettingsToCategoryOverrides(data.qualityProfileSettings);
        setCategoryQualityProfileOverrides((previous) =>
          previous.movie === nextOverrides.movie &&
          previous.series === nextOverrides.series &&
          previous.anime === nextOverrides.anime
            ? previous
            : nextOverrides,
        );

        const nextAnimeDefaults: AnimeCatalogDefaults = {
          monitorSpecials: data.animeSettings?.monitorSpecials ?? false,
          interSeasonMovies: data.animeSettings?.interSeasonMovies ?? true,
        };
        setAnimeCatalogDefaults((previous) =>
          previous.monitorSpecials === nextAnimeDefaults.monitorSpecials &&
          previous.interSeasonMovies === nextAnimeDefaults.interSeasonMovies
            ? previous
            : nextAnimeDefaults,
        );

        const nextRootFolders: Record<Facet, RootFolderOption[]> = {
          movie: data.movieSettings?.rootFolders ?? [],
          series: data.seriesSettings?.rootFolders ?? [],
          anime: data.animeSettings?.rootFolders ?? [],
        };
        setRootFoldersByFacet((previous) => {
          const same = (["movie", "series", "anime"] as Facet[]).every((f) => {
            const prev = previous[f];
            const next = nextRootFolders[f];
            return prev.length === next.length && prev.every((e, i) => e.path === next[i]?.path && e.isDefault === next[i]?.isDefault);
          });
          return same ? previous : nextRootFolders;
        });

        const nextLibrariesByFacet = librariesByFacetFromList(
          data.manageableLibraries ?? [],
        );
        const nextRequestableLibrariesByFacet = librariesByFacetFromList(
          data.requestableLibraries ?? [],
        );
        setLibrariesByFacet((previous) => {
          return sameLibrariesByFacet(previous, nextLibrariesByFacet)
            ? previous
            : nextLibrariesByFacet;
        });
        setRequestableLibrariesByFacet((previous) => {
          return sameLibrariesByFacet(previous, nextRequestableLibrariesByFacet)
            ? previous
            : nextRequestableLibrariesByFacet;
        });
      } catch {
        try {
          const { data, error } = await client
            .query(requestableLibrariesQuery, {}, { requestPolicy: "network-only" })
            .toPromise();
          if (error) throw error;
          const nextRequestableLibrariesByFacet = librariesByFacetFromList(
            data?.requestableLibraries ?? [],
          );
          setRequestableLibrariesByFacet((previous) => {
            return sameLibrariesByFacet(previous, nextRequestableLibrariesByFacet)
              ? previous
              : nextRequestableLibrariesByFacet;
          });
        } catch {
          // ignore requestable library fallback failures here; search remains functional
        }
        // ignore settings fetch failures here; search remains functional
      } finally {
        setCatalogConfigLoading(false);
        catalogConfigRefreshPromiseRef.current = null;
      }
    })();

    catalogConfigRefreshPromiseRef.current = refreshPromise;
    return refreshPromise;
  }, [client]);

  const ensureCatalogConfigReady = useCallback(
    async (facet: Facet) => {
      if (isCatalogConfigReady(facet)) {
        return;
      }
      await refreshCatalogQualityProfileState();
    },
    [isCatalogConfigReady, refreshCatalogQualityProfileState],
  );

  useEffect(() => {
    void refreshCatalogQualityProfileState();
  }, [refreshCatalogQualityProfileState]);

  // Re-fetch search config when settings change (cross-client via WebSocket).
  const searchSettingsKeys = useMemo(
    () =>
      new Set([
        QUALITY_PROFILE_CATALOG_KEY,
        QUALITY_PROFILE_ID_KEY,
        ...FACET_REGISTRY.map((f) => f.rootFoldersKey),
        ...FACET_REGISTRY.map((f) => f.folderSettingKey),
        ANIME_MONITOR_SPECIALS_KEY,
        ANIME_INTER_SEASON_MOVIES_KEY,
      ]),
    [],
  );

  useSettingsSubscription(
    useCallback(
      (keys: string[]) => {
        if (keys.some((k) => searchSettingsKeys.has(k))) {
          void refreshCatalogQualityProfileState();
        }
      },
      [searchSettingsKeys, refreshCatalogQualityProfileState],
    ),
  );

  const isMetadataSearchResultInAnyCatalog = useCallback(
    (result: MetadataTvdbSearchItem) => isMetadataResultCataloged(catalogTitlesByTvdbId, result),
    [catalogTitlesByTvdbId],
  );

  const isMetadataSearchResultInCatalog = useCallback(
    (_facet: Facet, result: MetadataTvdbSearchItem) => isMetadataSearchResultInAnyCatalog(result),
    [isMetadataSearchResultInAnyCatalog],
  );

  const mapFacetToTvdbType = useCallback((facet: Facet) => {
    return facetById(facet)?.tvdbSearchType ?? "series";
  }, []);

  const resolveCatalogPosterUrl = useCallback(
    async (title: TitleRecord): Promise<TitleRecord> => {
      if (title.posterUrl) {
        return title;
      }

      const tvdbId = (title.externalIds ?? [])
        .find((externalId) => externalId.source.toLowerCase() === "tvdb")
        ?.value.trim();
      if (!tvdbId) {
        return title;
      }

      try {
        if (title.facet === "movie") {
          const tvdbIdNum = parseInt(tvdbId, 10);
          if (isNaN(tvdbIdNum)) return title;
          const { data, error } = await client.query(metadataMovieQuery, {
            tvdbId: tvdbIdNum,
            language: uiLanguage,
          }).toPromise();
          if (error || !data?.metadataMovie?.posterUrl) return title;
          return { ...title, posterUrl: data.metadataMovie.posterUrl };
        }

        const { data, error } = await client.query(metadataSeriesQuery, {
          id: tvdbId,
          includeEpisodes: false,
          language: uiLanguage,
        }).toPromise();
        if (error || !data?.metadataSeries?.posterUrl) return title;
        return { ...title, posterUrl: data.metadataSeries.posterUrl };
      } catch {
        return title;
      }
    },
    [client, uiLanguage],
  );

  const emptyMetadataSearchResults = useMemo<MetadataSearchResults>(
    () => Object.fromEntries(FACET_REGISTRY.map((f) => [f.metadataKey, []])),
    [],
  );
  const emptyCatalogTitlesByTvdbId = useMemo<Record<string, TitleRecord>>(() => ({}), []);

  const lookupCatalogTitlesByExternalIds = useCallback(
    async (
      source: string,
      values: string[],
      fetchOverride?: typeof fetch,
    ): Promise<TitleRecord[]> => {
      const normalizedValues = normalizeOrderedLookupValues(values);
      if (!source.trim() || normalizedValues.length === 0) {
        return [];
      }

      const { data, error } = await client.query(
        titlesByExternalIdsQuery,
        {
          source: source.trim(),
          values: normalizedValues,
        },
        fetchOverride ? { fetch: fetchOverride } : undefined,
      ).toPromise();
      if (error) {
        throw error;
      }
      return (data?.titlesByExternalIds ?? []) as TitleRecord[];
    },
    [client],
  );

  const lookupCatalogTitlesByTvdbIds = useCallback(
    async (tvdbIds: string[], fetchOverride?: typeof fetch): Promise<TitleRecord[]> =>
      lookupCatalogTitlesByExternalIds("tvdb", tvdbIds, fetchOverride),
    [lookupCatalogTitlesByExternalIds],
  );

  const runTvdbSearch = useCallback(
    async (query: string) => {
      setGlobalStatus(t("status.searchingTvdb", { query }));
      try {
        const { data: searchData, error: searchError } = await client.query(searchMetadataQuery, {
          query,
          type: mapFacetToTvdbType(queueFacet),
          limit: 12,
          language: uiLanguage,
        }).toPromise();
        if (searchError) throw searchError;
        const rankedMatches = sortByRelevance(
          (searchData.searchMetadata || []) as MetadataTvdbSearchItem[],
          query,
        );
        const catalogLookup = buildCatalogTitleLookupByTvdbId(
          await lookupCatalogTitlesByTvdbIds(
            rankedMatches.map((item) => metadataResultTvdbId(item)),
          ),
        );
        const matches = rankedMatches.filter(
          (item: MetadataTvdbSearchItem) => !isMetadataResultCataloged(catalogLookup, item),
        );
        setTvdbCandidates(matches);
        setGlobalStatus(matches.length ? t("status.foundTvdb", { count: matches.length }) : t("status.nothingFound"));
        return matches;
      } catch (error) {
        setGlobalStatus(error instanceof Error ? error.message : t("status.apiError"));
        setTvdbCandidates([]);
        return [];
      }
    },
    [
      client,
      lookupCatalogTitlesByTvdbIds,
      mapFacetToTvdbType,
      queueFacet,
      setGlobalStatus,
      sortByRelevance,
      t,
      uiLanguage,
    ],
  );

  const runMetadataAutocomplete = useCallback(
    async (query: string) => {
      const trimmed = query.trim();
      if (!trimmed) {
        setCatalogSearchLoading(false);
        setMetadataSearchLoading(false);
        setCatalogSearchResults((previous) => (previous.length === 0 ? previous : []));
        setMetadataSearchResults((previous) => {
          if (isMetadataEmpty(previous)) {
            return previous;
          }
          return emptyMetadataSearchResults;
        });
        setCatalogTitlesByTvdbId((previous) =>
          Object.keys(previous).length === 0 ? previous : emptyCatalogTitlesByTvdbId,
        );
        setGlobalStatus(t("label.ready"));
        return;
      }

      const requestId = ++autocompleteRequestId.current;
      setSearching(true);
      setCatalogSearchLoading(true);
      setMetadataSearchLoading(true);

      // Abort previous in-flight autocomplete HTTP requests so cancellation
      // propagates through Rust all the way to the SMG database query.
      autocompleteAbortRef.current?.abort();
      const abortController = new AbortController();
      autocompleteAbortRef.current = abortController;
      const abortableFetch = makeAbortableFetch(abortController.signal);
      let directCatalogEntries: TitleRecord[] = [];
      let promotedCatalogEntries: TitleRecord[] = [];

      // Fire both queries in parallel but render each result as it arrives
      // so the fast catalog query populates immediately while the metadata
      // spinner keeps spinning.

      const catalogPromise = client.query(catalogSearchTitlesQuery, {
        query: trimmed,
        facet: null,
      }, { fetch: abortableFetch }).toPromise()
        .then(async ({ data, error }) => {
          if (error) throw error;
          if (requestId !== autocompleteRequestId.current) return;
          const catalogEntries = (data?.titles ?? []) as TitleRecord[];
          const enriched = await Promise.all(
            catalogEntries.map((title: TitleRecord) => resolveCatalogPosterUrl(title)),
          );
          if (requestId !== autocompleteRequestId.current) return;
          directCatalogEntries = enriched;
          const next = directCatalogEntries.slice(0, AUTOCOMPLETE_LIMIT);
          setCatalogSearchResults((previous) =>
            sameTitleList(previous, next) ? previous : next,
          );
        })
        .finally(() => {
          if (requestId !== autocompleteRequestId.current) return;
          setCatalogSearchLoading(false);
        });

      const metadataPromise = client.query(searchMetadataMultiQuery, {
        query: trimmed,
        limit: AUTOCOMPLETE_LIMIT,
        language: uiLanguage,
      }, { fetch: abortableFetch }).toPromise()
        .then(async ({ data, error }) => {
          if (error) throw error;
          if (requestId !== autocompleteRequestId.current) return;
          const multi = data.searchMetadataMulti ?? { movies: [], series: [], anime: [] };
          const rankedMovies = sortByRelevance(
            (multi.movies || []) as MetadataTvdbSearchItem[],
            trimmed,
          );
          const rankedAnime = sortByRelevance(
            (multi.anime || []) as MetadataTvdbSearchItem[],
            trimmed,
          );
          const rankedSeries = sortByRelevance(
            (multi.series || []) as MetadataTvdbSearchItem[],
            trimmed,
          );
          promotedCatalogEntries = await lookupCatalogTitlesByTvdbIds(
            [
              ...rankedMovies.map((item) => metadataResultTvdbId(item)),
              ...rankedAnime.map((item) => metadataResultTvdbId(item)),
              ...rankedSeries.map((item) => metadataResultTvdbId(item)),
            ],
            abortableFetch,
          );
          if (requestId !== autocompleteRequestId.current) return;
          const nextCatalogLookup = buildCatalogTitleLookupByTvdbId(promotedCatalogEntries);
          setCatalogTitlesByTvdbId((previous) =>
            sameCatalogLookup(previous, nextCatalogLookup) ? previous : nextCatalogLookup,
          );
          const movieResults = filterCatalogedMetadataResults(rankedMovies, nextCatalogLookup);
          const animeResults = filterCatalogedMetadataResults(rankedAnime, nextCatalogLookup);
          const animeTvdbIds = new Set(animeResults.map((item) => metadataResultTvdbId(item)));
          const seriesResults = filterCatalogedMetadataResults(
            rankedSeries,
            nextCatalogLookup,
          ).filter((item) => !animeTvdbIds.has(metadataResultTvdbId(item)));
          const nextMetadata: MetadataSearchResults = {
            movie: movieResults,
            series: seriesResults,
            anime: animeResults,
          };
          setMetadataSearchResults((previous) => {
            const unchanged = Object.keys(nextMetadata).every((key) => {
              const prev = previous[key] ?? [];
              const next = nextMetadata[key] ?? [];
              return prev.length === next.length && prev.every((item, i) => item.tvdbId === next[i]?.tvdbId);
            });
            return unchanged ? previous : nextMetadata;
          });
        })
        .finally(() => {
          if (requestId !== autocompleteRequestId.current) return;
          setMetadataSearchLoading(false);
        });

      const [catalogResult, metadataResult] = await Promise.allSettled([
        catalogPromise,
        metadataPromise,
      ]);

      if (requestId !== autocompleteRequestId.current) return;

      // Surface errors from either leg (suppress AbortError — the request
      // was intentionally cancelled by a newer autocomplete keystroke).
      if (catalogResult.status === "rejected" && !isAbortError(catalogResult.reason)) {
        const msg = catalogResult.reason instanceof Error ? catalogResult.reason.message : t("status.apiError");
        setGlobalStatus(msg);
      }
      if (metadataResult.status === "rejected" && !isAbortError(metadataResult.reason)) {
        const msg = metadataResult.reason instanceof Error ? metadataResult.reason.message : t("status.apiError");
        setGlobalStatus(msg);
        setMetadataSearchResults((prev) => (isMetadataEmpty(prev) ? prev : emptyMetadataSearchResults));
        setCatalogTitlesByTvdbId((previous) =>
          Object.keys(previous).length === 0 ? previous : emptyCatalogTitlesByTvdbId,
        );
        promotedCatalogEntries = [];
      }

      const mergedCatalogEntries = mergeCatalogResults(
        promotedCatalogEntries,
        directCatalogEntries,
      );
      const nextCatalogResults = (
        await Promise.all(mergedCatalogEntries.map((title) => resolveCatalogPosterUrl(title)))
      ).slice(0, AUTOCOMPLETE_LIMIT);
      if (requestId !== autocompleteRequestId.current) return;
      setCatalogSearchResults((previous) =>
        sameTitleList(previous, nextCatalogResults) ? previous : nextCatalogResults,
      );

      setSearching(false);
    },
    [
      client,
      emptyCatalogTitlesByTvdbId,
      emptyMetadataSearchResults,
      lookupCatalogTitlesByTvdbIds,
      resolveCatalogPosterUrl,
      setGlobalStatus,
      sortByRelevance,
      t,
      uiLanguage,
    ],
  );

  useEffect(() => {
    const trimmed = globalSearch.trim();

    if (trimmed.length < AUTOCOMPLETE_MIN_CHARS) {
      cancelAutocomplete();
      setCatalogSearchResults((previous) => (previous.length === 0 ? previous : []));
      setMetadataSearchResults((previous) => {
        if (previous.movie.length === 0 && previous.series.length === 0 && previous.anime.length === 0) {
          return previous;
        }
        return emptyMetadataSearchResults;
      });
      setCatalogTitlesByTvdbId((previous) =>
        Object.keys(previous).length === 0 ? previous : emptyCatalogTitlesByTvdbId,
      );
      // Don't auto-close when the panel was force-opened (mobile overlay).
      if (!forcedOpenRef.current) {
        setIsGlobalSearchPanelOpen((isOpen) => (isOpen ? false : isOpen));
      }
      return;
    }

    const debounceTimer = window.setTimeout(() => {
      void runMetadataAutocomplete(trimmed);
    }, AUTOCOMPLETE_DEBOUNCE_MS);

    return () => {
      window.clearTimeout(debounceTimer);
    };
  }, [
    cancelAutocomplete,
    emptyCatalogTitlesByTvdbId,
    emptyMetadataSearchResults,
    globalSearch,
    runMetadataAutocomplete,
  ]);

  useEffect(() => {
    return () => {
      autocompleteAbortRef.current?.abort();
    };
  }, []);

  const openGlobalSearchPanel = useCallback((force?: boolean) => {
    if (force) {
      forcedOpenRef.current = true;
      setIsGlobalSearchPanelOpen(true);
      return;
    }
    if (globalSearch.trim().length >= AUTOCOMPLETE_MIN_CHARS) {
      setIsGlobalSearchPanelOpen(true);
    }
  }, [globalSearch]);

  const closeGlobalSearchPanel = useCallback(() => {
    forcedOpenRef.current = false;
    setIsGlobalSearchPanelOpen(false);
  }, []);

  const resetGlobalSearch = useCallback(() => {
    forcedOpenRef.current = false;
    cancelAutocomplete();
    setGlobalSearch("");
    setCatalogSearchResults((previous) => (previous.length === 0 ? previous : []));
    setMetadataSearchResults((previous) =>
      isMetadataEmpty(previous) ? previous : emptyMetadataSearchResults,
    );
    setCatalogTitlesByTvdbId((previous) =>
      Object.keys(previous).length === 0 ? previous : emptyCatalogTitlesByTvdbId,
    );
    setIsGlobalSearchPanelOpen(false);
  }, [
    cancelAutocomplete,
    emptyCatalogTitlesByTvdbId,
    emptyMetadataSearchResults,
  ]);

  useEffect(() => {
    const handleShortcut = (event: KeyboardEvent) => {
      if (event.key !== "/") {
        return;
      }

      if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) {
        return;
      }

      const target = event.target as HTMLElement | null;
      if (
        target?.tagName === "INPUT" ||
        target?.tagName === "TEXTAREA" ||
        target?.isContentEditable ||
        target?.tagName === "SELECT"
      ) {
        return;
      }

      event.preventDefault();
      globalSearchInputRef.current?.focus();
      globalSearchInputRef.current?.select();
    };

    window.addEventListener("keydown", handleShortcut);
    return () => window.removeEventListener("keydown", handleShortcut);
  }, []);

  const addMetadataSearchResultToCatalog = useCallback(
    async (
      result: MetadataTvdbSearchItem,
      facet: Facet,
      options: MetadataCatalogAddOptions,
    ) => {
      const name = result.name.trim();
      if (!name) {
        setGlobalStatus(t("status.titleRequired"));
        return null;
      }

      const qualityProfileId = (
        options.qualityProfileId || resolveDefaultQualityProfileIdForFacet(facet)
      ).trim();
      if (!qualityProfileId) {
        setGlobalStatus(t("search.addConfigNoQualityProfiles"));
        return null;
      }

      const monitored = monitorTypeToMonitored(options.monitorType);

      const tvdbId = String(result.tvdbId).trim();
      const imdbId = result.imdbId?.trim();
      const externalIds = [
        ...(tvdbId ? [{ source: "tvdb", value: tvdbId }] : []),
        ...(imdbId ? [{ source: "imdb", value: imdbId }] : []),
      ];
      const requestKey = normalizeCatalogAddRequestKey(facet, externalIds);
      if (pendingCatalogAddKeysRef.current.has(requestKey)) {
        return null;
      }
      pendingCatalogAddKeysRef.current.add(requestKey);
      try {
        const { data: addData, error: addError } = await client.mutation(addTitleMutation, {
          input: {
            name,
            facet,
            libraryId: options.libraryId || undefined,
            monitored,
            tags: [],
            options: {
              qualityProfileId: qualityProfileId || undefined,
              rootFolderPath: options.rootFolder || undefined,
              monitorType: options.monitorType,
              ...(facet === "movie"
                ? {}
                : { useSeasonFolders: options.seasonFolder }),
              ...(facet === "anime"
                ? {
                    monitorSpecials: options.monitorSpecials !== false,
                    interSeasonMovies: options.interSeasonMovies !== false,
                  }
                : {}),
            },
            externalIds,
            ...(facet === "movie" && options.minAvailability ? { minAvailability: options.minAvailability } : {}),
            posterUrl: result.posterUrl || undefined,
            year: result.year ?? undefined,
            overview: result.overview || undefined,
            sortTitle: result.sortTitle || undefined,
            slug: result.slug || undefined,
            runtimeMinutes: result.runtimeMinutes ?? undefined,
            language: result.language || undefined,
            contentStatus: result.status || undefined,
          },
        }).toPromise();
        if (addError) throw addError;
        setGlobalStatus(
          t(
            monitored
              ? "status.catalogAddSuccessAutoSearch"
              : "status.catalogAddSuccess",
            { name: addData.addTitle.title.name },
          ),
        );
        await runMetadataAutocomplete(globalSearch.trim());
        return addData.addTitle?.title?.id?.trim() || null;
      } catch (error) {
        setGlobalStatus(error instanceof Error ? error.message : t("status.queueFailed"));
        return null;
      } finally {
        pendingCatalogAddKeysRef.current.delete(requestKey);
      }
    },
    [
      globalSearch,
      resolveDefaultQualityProfileIdForFacet,
      runMetadataAutocomplete,
      client,
      setGlobalStatus,
      t,
    ],
  );

  const requestMetadataSearchResult = useCallback(
    async (
      result: MetadataTvdbSearchItem,
      facet: Facet,
      options: MetadataCatalogRequestOptions,
    ) => {
      const name = result.name.trim();
      const libraryId = options.libraryId.trim();
      if (!name || !libraryId) {
        setGlobalStatus(t("status.titleRequired"));
        return false;
      }

      const tvdbId = String(result.tvdbId).trim();
      const imdbId = result.imdbId?.trim();
      const externalIds = [
        ...(tvdbId ? [{ source: "tvdb", value: tvdbId }] : []),
        ...(imdbId ? [{ source: "imdb", value: imdbId }] : []),
      ];
      const requestKey = normalizeCatalogAddRequestKey(facet, externalIds);
      if (pendingRequestKeysRef.current.has(requestKey)) {
        return false;
      }
      pendingRequestKeysRef.current.add(requestKey);
      try {
        const { error } = await client.mutation(submitMediaRequestMutation, {
          input: {
            libraryId,
            facet,
            title: name,
            externalIds,
            posterUrl: result.posterUrl || undefined,
            year: result.year ?? undefined,
            overview: result.overview || undefined,
            sortTitle: result.sortTitle || undefined,
            slug: result.slug || undefined,
            runtimeMinutes: result.runtimeMinutes ?? undefined,
            language: result.language || undefined,
            contentStatus: result.status || undefined,
          },
        }).toPromise();
        if (error) throw error;
        setGlobalStatus(t("status.requestSubmitted", { name }));
        await runMetadataAutocomplete(globalSearch.trim());
        return true;
      } catch (error) {
        setGlobalStatus(error instanceof Error ? error.message : t("status.queueFailed"));
        return false;
      } finally {
        pendingRequestKeysRef.current.delete(requestKey);
      }
    },
    [client, globalSearch, runMetadataAutocomplete, setGlobalStatus, t],
  );

  /** Force-trigger global search (bypasses autocomplete min-char threshold). */
  const forceSearchGlobal = useCallback(async () => {
    const trimmed = globalSearch.trim();
    if (!trimmed) return;
    setIsGlobalSearchPanelOpen(true);
    await runMetadataAutocomplete(trimmed);
  }, [globalSearch, runMetadataAutocomplete]);

  return {
    globalSearch,
    setGlobalSearch,
    globalSearchInputRef,
    searching,
    catalogSearchLoading,
    metadataSearchLoading,
    tvdbCandidates,
    runTvdbSearch,
    forceSearchGlobal,
    setTvdbCandidates,
    catalogSearchResults,
    metadataSearchResults,
    isGlobalSearchPanelOpen,
    openGlobalSearchPanel,
    closeGlobalSearchPanel,
    resetGlobalSearch,
    catalogQualityProfileOptions,
    catalogConfigLoading,
    ensureCatalogConfigReady,
    isCatalogConfigReady,
    resolveDefaultQualityProfileIdForFacet,
    animeCatalogDefaults,
    addMetadataSearchResultToCatalog,
    requestMetadataSearchResult,
    isMetadataSearchResultInCatalog,
    rootFoldersByFacet,
    librariesByFacet,
    requestableLibrariesByFacet,
    queueFacet,
    setQueueFacet,
    catalogChangeSignal,
  };
}
