import * as React from "react";
import { ChevronDown, Loader2, Search } from "lucide-react";
import { Link } from "react-router-dom";
import { useClient } from "urql";

import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { LibraryMultiSelect } from "@/components/common/library-multi-select";
import { TitlePoster } from "@/components/title-poster";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { Input } from "@/components/ui/input";
import type { ViewId } from "@/components/root/types";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";
import { facetForView } from "@/lib/facets/registry";
import {
  bindPendingImportMutation,
  ignorePendingImportMutation,
  resolvePendingImportMutation,
} from "@/lib/graphql/mutations";
import {
  pendingImportBindingPreviewQuery,
  pendingImportsQuery,
  librariesQuery,
  searchMetadataQuery,
} from "@/lib/graphql/queries";
import { isAbortError, makeAbortableFetch } from "@/lib/graphql/urql-client";
import type {
  PendingImportBindingEpisode,
  PendingImportBindingPreview,
  PendingImportConnection,
  PendingImportItem,
  PendingImportStatus,
  LibraryRecord,
  ResolvePendingImportResult,
} from "@/lib/types";
import { pendingImportFacetValueForView } from "@/lib/types";
import { dispatchNavigationBadgesRefresh } from "@/lib/events/navigation-badges";
import {
  normalizeLibraryFilterSelection,
  selectedLibraryIdsToQueryValue,
} from "@/lib/utils/library-filter";
import { buildOverviewDetailPath } from "@/lib/utils/routing";

type PendingImportsContainerProps = {
  view: ViewId;
  onNavigateBackToOverview: () => void;
};

const PAGE_SIZE = 50;

type MetadataSearchResult = {
  tvdbId: string;
  name: string;
  imdbId: string | null;
  slug: string | null;
  type: string | null;
  year: number | null;
  status: string | null;
  overview: string | null;
  popularity: number | null;
  posterUrl: string | null;
  language: string | null;
  runtimeMinutes: number | null;
  sortTitle: string | null;
};

const GENERIC_PENDING_IMPORT_QUERY_SEEDS = new Set([
  "download",
  "file",
  "movie",
  "unknown",
  "video",
]);

function basenameFromPath(path: string | null | undefined): string {
  if (!path) {
    return "";
  }

  const segments = path.split(/[\\/]/).filter(Boolean);
  return segments.at(-1)?.trim() ?? "";
}

function folderSearchSeedFromPath(path: string | null | undefined): string {
  return basenameFromPath(path)
    .replace(/[._]+/g, " ")
    .replace(/\s+/g, " ")
    .trim();
}

function isObfuscatedPendingImportSeed(value: string): boolean {
  return value
    .split(/[^A-Za-z0-9]+/)
    .filter((token) => token.length >= 8)
    .some((token) => {
      const hasAlpha = /[A-Za-z]/.test(token);
      const hasDigit = /\d/.test(token);
      const isHexLike = /^[A-Fa-f0-9]+$/.test(token);
      return (hasAlpha && hasDigit) || isHexLike;
    });
}

function isUsablePendingImportSearchSeed(value: string | null | undefined): boolean {
  const trimmed = value?.trim() ?? "";
  if (!trimmed) {
    return false;
  }

  if (GENERIC_PENDING_IMPORT_QUERY_SEEDS.has(trimmed.toLowerCase())) {
    return false;
  }

  return !isObfuscatedPendingImportSeed(trimmed);
}

function summarizePendingImport(item: PendingImportItem): string {
  const parts = [item.reason];

  if (item.query.trim()) {
    parts.push(item.query.trim());
  }

  if (typeof item.yearHint === "number") {
    parts.push(String(item.yearHint));
  }

  return parts.join(" • ");
}

function viewForPendingImportFacet(
  facet: PendingImportItem["facet"],
): Extract<ViewId, "movies" | "series" | "anime"> {
  switch (facet) {
    case "movie":
      return "movies";
    case "series":
      return "series";
    case "anime":
      return "anime";
  }
}

function pendingImportKnownTitleHref(item: PendingImportItem): string | null {
  const titleId = item.titleId?.trim();
  if (!titleId) {
    return null;
  }

  const titleSlug = item.titleSlug?.trim() || null;
  const librarySlug = item.librarySlug?.trim() || null;
  const path = buildOverviewDetailPath(viewForPendingImportFacet(item.facet), librarySlug, titleSlug);
  if (titleSlug && librarySlug) {
    return path;
  }

  const params = new URLSearchParams({ id: titleId });
  return `${path}?${params.toString()}`;
}

function pendingImportKnownTitleLabel(item: PendingImportItem): string {
  return item.titleName?.trim() || item.titleId?.trim() || "";
}

function formatBindingEpisodeKey(episode: PendingImportBindingEpisode): string | null {
  const season = episode.seasonNumber?.trim();
  const episodeNumber = episode.episodeNumber?.trim();
  if (season && episodeNumber) {
    return `S${season.padStart(2, "0")}E${episodeNumber.padStart(2, "0")}`;
  }
  if (episodeNumber) {
    return `Episode ${episodeNumber}`;
  }
  return null;
}

function formatBindingEpisodeLabel(episode: PendingImportBindingEpisode): string {
  return episode.episodeLabel?.trim() || episode.title?.trim() || episode.id;
}

function formatBindingEpisodeDisplay(episode: PendingImportBindingEpisode) {
  const key = formatBindingEpisodeKey(episode);
  const label = formatBindingEpisodeLabel(episode);
  return {
    key,
    label,
    showSeparateLabel: Boolean(key && label !== key),
  };
}

function bindingSeasonKeyForEpisode(episode: PendingImportBindingEpisode): string {
  return episode.seasonNumber?.trim() || "specials";
}

function bindingSeasonKeysForSelection(
  episodes: PendingImportBindingEpisode[],
  selectedEpisodeIds: string[],
): string[] {
  if (selectedEpisodeIds.length === 0) {
    return [];
  }

  const selectedIds = new Set(selectedEpisodeIds);
  const expandedKeys = new Set<string>();
  for (const episode of episodes) {
    if (selectedIds.has(episode.id)) {
      expandedKeys.add(bindingSeasonKeyForEpisode(episode));
    }
  }
  return Array.from(expandedKeys);
}

function groupBindingEpisodes(episodes: PendingImportBindingEpisode[]) {
  const groups = new Map<string, PendingImportBindingEpisode[]>();
  for (const episode of episodes) {
    const key = bindingSeasonKeyForEpisode(episode);
    const group = groups.get(key);
    if (group) {
      group.push(episode);
    } else {
      groups.set(key, [episode]);
    }
  }

  for (const group of groups.values()) {
    group.sort((left, right) => {
      const leftNumber = Number.parseInt(left.episodeNumber?.replace(/\D/g, "") || "0", 10);
      const rightNumber = Number.parseInt(right.episodeNumber?.replace(/\D/g, "") || "0", 10);
      return leftNumber - rightNumber;
    });
  }

  return Array.from(groups.entries()).sort(([left], [right]) => {
    if (left === "specials") return 1;
    if (right === "specials") return -1;
    return Number.parseInt(left, 10) - Number.parseInt(right, 10);
  });
}

export const PendingImportsContainer = React.memo(function PendingImportsContainer({
  view,
  onNavigateBackToOverview,
}: PendingImportsContainerProps) {
  const client = useClient();
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const facet = facetForView(view);

  const [pendingConnection, setPendingConnection] = React.useState<PendingImportConnection>({
    total: 0,
    items: [],
  });
  const [ignoredConnection, setIgnoredConnection] = React.useState<PendingImportConnection>({
    total: 0,
    items: [],
  });
  const [pendingOffset, setPendingOffset] = React.useState(0);
  const [ignoredOffset, setIgnoredOffset] = React.useState(0);
  const [pendingLoading, setPendingLoading] = React.useState(false);
  const [ignoredLoading, setIgnoredLoading] = React.useState(false);
  const [pendingLoaded, setPendingLoaded] = React.useState(false);
  const [ignoredLoaded, setIgnoredLoaded] = React.useState(false);
  const [libraries, setLibraries] = React.useState<LibraryRecord[]>([]);
  const [librariesLoading, setLibrariesLoading] = React.useState(false);
  const [selectedLibraryIds, setSelectedLibraryIds] = React.useState<string[]>([]);
  const [pendingError, setPendingError] = React.useState<string | null>(null);
  const [ignoredError, setIgnoredError] = React.useState<string | null>(null);
  const [ignoredOpen, setIgnoredOpen] = React.useState(false);
  const [activeItemRef, setActiveItemRef] = React.useState<{
    id: string;
    status: PendingImportStatus;
  } | null>(null);
  const [searchQuery, setSearchQuery] = React.useState("");
  const [searchResults, setSearchResults] = React.useState<MetadataSearchResult[]>([]);
  const [searching, setSearching] = React.useState(false);
  const [bindingPreview, setBindingPreview] = React.useState<PendingImportBindingPreview | null>(null);
  const [bindingLoading, setBindingLoading] = React.useState(false);
  const [bindingError, setBindingError] = React.useState<string | null>(null);
  const [selectedEpisodeIds, setSelectedEpisodeIds] = React.useState<string[]>([]);
  const [expandedBindingSeasonKeys, setExpandedBindingSeasonKeys] = React.useState<string[]>([]);
  const [resolvingItemId, setResolvingItemId] = React.useState<string | null>(null);
  const [ignoringItemId, setIgnoringItemId] = React.useState<string | null>(null);
  const [ignoreTargetItem, setIgnoreTargetItem] = React.useState<PendingImportItem | null>(null);
  const libraryNameById = React.useMemo(
    () => new Map(libraries.map((library) => [library.id, library.name])),
    [libraries],
  );
  const librarySlugById = React.useMemo(
    () => new Map(libraries.map((library) => [library.id, library.slug])),
    [libraries],
  );

  React.useEffect(() => {
    let cancelled = false;
    setLibrariesLoading(true);
    void client
      .query(
        librariesQuery,
        { facet: pendingImportFacetValueForView(view), permission: "resolveImports" },
        { requestPolicy: "network-only" },
      )
      .toPromise()
      .then(({ data, error }) => {
        if (cancelled) {
          return;
        }
        if (error) {
          throw error;
        }
        const nextLibraries = (data?.libraries ?? []) as LibraryRecord[];
        setLibraries(nextLibraries);
        setSelectedLibraryIds((current) =>
          normalizeLibraryFilterSelection(current, nextLibraries),
        );
      })
      .catch((error) => {
        if (!cancelled) {
          setGlobalStatus(error instanceof Error ? error.message : t("status.failedToLoad"));
        }
      })
      .finally(() => {
        if (!cancelled) {
          setLibrariesLoading(false);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [client, setGlobalStatus, t, view]);

  const clearActiveItem = React.useCallback(() => {
    setActiveItemRef(null);
    setSearchQuery("");
    setSearchResults([]);
    setSearching(false);
    setBindingPreview(null);
    setBindingLoading(false);
    setBindingError(null);
    setSelectedEpisodeIds([]);
    setExpandedBindingSeasonKeys([]);
    setIgnoreTargetItem(null);
  }, []);

  const activeItem = React.useMemo(
    () => {
      if (!activeItemRef) {
        return null;
      }

      const items = activeItemRef.status === "ignored"
        ? ignoredConnection.items
        : pendingConnection.items;
      return items.find((item) => item.id === activeItemRef.id) ?? null;
    },
    [activeItemRef, ignoredConnection.items, pendingConnection.items],
  );
  const pendingHasPrevPage = pendingOffset > 0;
  const pendingHasNextPage = pendingOffset + PAGE_SIZE < pendingConnection.total;
  const pendingPageStart = pendingConnection.total === 0 ? 0 : pendingOffset + 1;
  const pendingPageEnd = Math.min(pendingOffset + PAGE_SIZE, pendingConnection.total);
  const ignoredHasPrevPage = ignoredOffset > 0;
  const ignoredHasNextPage = ignoredOffset + PAGE_SIZE < ignoredConnection.total;
  const ignoredPageStart = ignoredConnection.total === 0 ? 0 : ignoredOffset + 1;
  const ignoredPageEnd = Math.min(ignoredOffset + PAGE_SIZE, ignoredConnection.total);

  const fetchPendingImportsPage = React.useCallback(
    async (
      status: PendingImportStatus,
      pageOffset: number,
    ): Promise<PendingImportConnection> => {
      const { data, error: queryError } = await client
        .query(pendingImportsQuery, {
          facet: pendingImportFacetValueForView(view),
          libraryIds: selectedLibraryIdsToQueryValue(selectedLibraryIds),
          status,
          limit: PAGE_SIZE,
          offset: pageOffset,
        })
        .toPromise();
      if (queryError) {
        throw queryError;
      }

      const connection = (data?.pendingImports ?? {
        total: 0,
        items: [],
      }) as PendingImportConnection;
      return {
        ...connection,
        items: connection.items.map((item) => ({
          ...item,
          librarySlug: item.librarySlug ?? librarySlugById.get(item.libraryId) ?? null,
        })),
      };
    },
    [client, librarySlugById, selectedLibraryIds, view],
  );

  const refresh = React.useCallback(
    async (
      status: PendingImportStatus,
      pageOffset: number,
    ): Promise<PendingImportConnection | null> => {
      const setLoading = status === "ignored" ? setIgnoredLoading : setPendingLoading;
      const setLoaded = status === "ignored" ? setIgnoredLoaded : setPendingLoaded;
      const setError = status === "ignored" ? setIgnoredError : setPendingError;
      const setOffset = status === "ignored" ? setIgnoredOffset : setPendingOffset;
      const setConnection = status === "ignored" ? setIgnoredConnection : setPendingConnection;

      setLoading(true);
      setError(null);

      try {
        let nextOffset = Math.max(0, pageOffset);
        let nextConnection = await fetchPendingImportsPage(status, nextOffset);

        if (nextConnection.total > 0 && nextOffset >= nextConnection.total) {
          nextOffset = Math.max(
            0,
            Math.floor((nextConnection.total - 1) / PAGE_SIZE) * PAGE_SIZE,
          );
          nextConnection = await fetchPendingImportsPage(status, nextOffset);
        }

        setOffset(nextOffset);
        setConnection(nextConnection);
        return nextConnection;
      } catch (err) {
        setError(err instanceof Error ? err.message : t("pendingImports.loadFailed"));
        return null;
      } finally {
        setLoading(false);
        setLoaded(true);
      }
    },
    [fetchPendingImportsPage, t],
  );

  const refreshAll = React.useCallback(async () => {
    await Promise.all([
      refresh("pending", pendingOffset),
      refresh("ignored", ignoredOffset),
    ]);
  }, [ignoredOffset, pendingOffset, refresh]);

  React.useEffect(() => {
    void refresh("pending", pendingOffset);
  }, [pendingOffset, refresh]);

  React.useEffect(() => {
    void refresh("ignored", ignoredOffset);
  }, [ignoredOffset, refresh]);

  React.useEffect(() => {
    setPendingOffset(0);
    setIgnoredOffset(0);
    setPendingLoaded(false);
    setIgnoredLoaded(false);
    setIgnoredOpen(false);
    clearActiveItem();
  }, [clearActiveItem, selectedLibraryIds, view]);

  React.useEffect(() => {
    if (
      !pendingLoaded ||
      !ignoredLoaded ||
      pendingLoading ||
      ignoredLoading ||
      pendingError ||
      ignoredError
    ) {
      return;
    }

    if (pendingConnection.total > 0 || ignoredConnection.total > 0) {
      return;
    }

    onNavigateBackToOverview();
  }, [
    ignoredConnection.total,
    ignoredError,
    ignoredLoaded,
    ignoredLoading,
    onNavigateBackToOverview,
    pendingConnection.total,
    pendingError,
    pendingLoaded,
    pendingLoading,
  ]);

  React.useEffect(() => {
    if (!activeItemRef) {
      setSearchResults([]);
      setSearching(false);
      return;
    }

    if (!searchQuery.trim()) {
      setSearchResults([]);
      setSearching(false);
      return;
    }

    const abortController = new AbortController();
    const abortableFetch = makeAbortableFetch(abortController.signal);
    let active = true;
    const timeoutId = window.setTimeout(() => {
      setSearching(true);
      client
        .query(searchMetadataQuery, {
          query: searchQuery.trim(),
          type: facet?.tvdbSearchType ?? "movie",
          limit: 8,
          year: activeItem?.yearHint ?? null,
        }, { fetch: abortableFetch })
        .toPromise()
        .then(({ data, error: queryError }) => {
          if (queryError) {
            throw queryError;
          }
          if (!active) {
            return;
          }
          const items = (data?.searchMetadata ?? []) as MetadataSearchResult[];
          setSearchResults(items);
        })
        .catch((err: unknown) => {
          if (!active || isAbortError(err)) {
            return;
          }
          setSearchResults([]);
          setGlobalStatus(
            err instanceof Error ? err.message : t("pendingImports.searchFailed"),
          );
        })
        .finally(() => {
          if (active) {
            setSearching(false);
          }
        });
    }, 220);

    return () => {
      active = false;
      window.clearTimeout(timeoutId);
      abortController.abort();
    };
  }, [
    activeItem?.yearHint,
    activeItemRef,
    client,
    facet?.tvdbSearchType,
    searchQuery,
    setGlobalStatus,
    t,
  ]);

  React.useEffect(() => {
    if (!activeItemRef) {
      return;
    }

    const items = activeItemRef.status === "ignored"
      ? ignoredConnection.items
      : pendingConnection.items;

    if (items.some((item) => item.id === activeItemRef.id)) {
      return;
    }

    clearActiveItem();
  }, [activeItemRef, clearActiveItem, ignoredConnection.items, pendingConnection.items]);

  const handleOpenSearch = React.useCallback((item: PendingImportItem) => {
    setActiveItemRef({ id: item.id, status: item.status });
    setSearchResults([]);
    setBindingPreview(null);
    setBindingError(null);
    setSelectedEpisodeIds([]);
    setExpandedBindingSeasonKeys([]);

    if (item.titleId) {
      setSearchQuery("");
      setBindingLoading(true);
      void client
        .query(pendingImportBindingPreviewQuery, {
          pendingImportId: item.id,
        })
        .toPromise()
        .then(({ data, error }) => {
          if (error) throw error;
          const preview = data?.pendingImportBindingPreview as PendingImportBindingPreview | undefined;
          const bindingPreview = preview ?? null;
          const suggestedEpisodeIds = bindingPreview?.file.suggestedEpisodeIds ?? [];
          setBindingPreview(bindingPreview);
          setSelectedEpisodeIds(suggestedEpisodeIds);
          setExpandedBindingSeasonKeys(
            bindingSeasonKeysForSelection(
              bindingPreview?.availableEpisodes ?? [],
              suggestedEpisodeIds,
            ),
          );
        })
        .catch((err: unknown) => {
          const message = err instanceof Error ? err.message : "Failed to load binding preview";
          setBindingError(message);
          setGlobalStatus(message);
        })
        .finally(() => setBindingLoading(false));
      return;
    }

    const seedQuery = isUsablePendingImportSearchSeed(item.query)
      ? item.query.trim()
      : folderSearchSeedFromPath(item.folderPath) || item.displayName.trim();
    setSearchQuery(seedQuery);
  }, [client, setGlobalStatus]);

  const handleRequestIgnore = React.useCallback((item: PendingImportItem) => {
    if (item.status !== "pending") {
      return;
    }

    setIgnoreTargetItem(item);
  }, []);

  const handleResolve = React.useCallback(async (tvdbId: string) => {
    if (!activeItem) {
      return;
    }

    setResolvingItemId(activeItem.id);
    try {
      const { data, error: mutationError } = await client
        .mutation(resolvePendingImportMutation, {
          input: {
            pendingImportId: activeItem.id,
            tvdbId,
          },
        })
        .toPromise();
      if (mutationError) {
        throw mutationError;
      }

      const result = data?.resolvePendingImport as ResolvePendingImportResult | undefined;
      dispatchNavigationBadgesRefresh();
      await refreshAll();

      setGlobalStatus(
        t("pendingImports.resolveSuccess", {
          name: result?.title?.name?.trim() || activeItem.displayName,
        }),
      );
      clearActiveItem();
    } catch (err) {
      setGlobalStatus(err instanceof Error ? err.message : t("pendingImports.resolveFailed"));
    } finally {
      setResolvingItemId(null);
    }
  }, [activeItem, clearActiveItem, client, refreshAll, setGlobalStatus, t]);

  const handleBind = React.useCallback(async () => {
    if (!activeItem) {
      return;
    }
    if (selectedEpisodeIds.length === 0) {
      setGlobalStatus("Select at least one episode.");
      return;
    }

    setResolvingItemId(activeItem.id);
    try {
      const { data, error: mutationError } = await client
        .mutation(bindPendingImportMutation, {
          input: {
            pendingImportId: activeItem.id,
            episodeIds: selectedEpisodeIds,
          },
        })
        .toPromise();
      if (mutationError) {
        throw mutationError;
      }

      const result = data?.bindPendingImport as ResolvePendingImportResult | undefined;
      dispatchNavigationBadgesRefresh();
      await refreshAll();
      setGlobalStatus(
        t("pendingImports.resolveSuccess", {
          name: result?.title?.name?.trim() || activeItem.displayName,
        }),
      );
      clearActiveItem();
    } catch (err) {
      setGlobalStatus(err instanceof Error ? err.message : "Failed to bind pending import.");
    } finally {
      setResolvingItemId(null);
    }
  }, [activeItem, clearActiveItem, client, refreshAll, selectedEpisodeIds, setGlobalStatus, t]);

  const handleIgnore = React.useCallback(async () => {
    if (!ignoreTargetItem || ignoreTargetItem.status !== "pending") {
      return;
    }

    setIgnoringItemId(ignoreTargetItem.id);
    try {
      const { error: mutationError } = await client
        .mutation(ignorePendingImportMutation, {
          pendingImportId: ignoreTargetItem.id,
        })
        .toPromise();
      if (mutationError) {
        throw mutationError;
      }

      dispatchNavigationBadgesRefresh();
      await refreshAll();
      setGlobalStatus(
        t("pendingImports.ignoreSuccess", {
          name: ignoreTargetItem.displayName,
        }),
      );

      if (activeItemRef?.id === ignoreTargetItem.id && activeItemRef.status === "pending") {
        clearActiveItem();
      } else {
        setIgnoreTargetItem(null);
      }
    } catch (err) {
      setGlobalStatus(err instanceof Error ? err.message : t("pendingImports.ignoreFailed"));
    } finally {
      setIgnoringItemId(null);
    }
  }, [activeItemRef, clearActiveItem, client, ignoreTargetItem, refreshAll, setGlobalStatus, t]);

  const toggleEpisodeSelection = React.useCallback((episodeId: string, checked: boolean) => {
    setSelectedEpisodeIds((current) => {
      if (checked) {
        return current.includes(episodeId) ? current : [...current, episodeId];
      }
      return current.filter((value) => value !== episodeId);
    });
  }, []);

  const bindingGroups = React.useMemo(
    () => groupBindingEpisodes(bindingPreview?.availableEpisodes ?? []),
    [bindingPreview],
  );

  const renderPagination = React.useCallback((
    status: PendingImportStatus,
    total: number,
    hasPrevPage: boolean,
    hasNextPage: boolean,
    pageStart: number,
    pageEnd: number,
  ) => {
    if (total <= PAGE_SIZE) {
      return null;
    }

    const setOffset = status === "ignored" ? setIgnoredOffset : setPendingOffset;
    const currentOffset = status === "ignored" ? ignoredOffset : pendingOffset;

    return (
      <div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
        <Button
          className="w-full sm:w-auto"
          size="sm"
          variant="outline"
          disabled={!hasPrevPage}
          onClick={() => setOffset(Math.max(0, currentOffset - PAGE_SIZE))}
        >
          {t("pendingImports.prev")}
        </Button>
        <span className="text-sm text-muted-foreground">
          {t("pendingImports.pageRange", {
            start: pageStart,
            end: pageEnd,
            total,
          })}
        </span>
        <Button
          className="w-full sm:w-auto"
          size="sm"
          variant="outline"
          disabled={!hasNextPage}
          onClick={() => setOffset(currentOffset + PAGE_SIZE)}
        >
          {t("pendingImports.next")}
        </Button>
      </div>
    );
  }, [ignoredOffset, pendingOffset, t]);

  const renderItems = React.useCallback((items: PendingImportItem[]) => (
    <div className="space-y-4">
      {items.map((item) => {
        const isActive = activeItemRef
          ? item.id === activeItemRef.id && item.status === activeItemRef.status
          : false;
        const isResolving = resolvingItemId === item.id;
        const isIgnoring = ignoringItemId === item.id;
        const isBusy = isResolving || isIgnoring || (isActive && bindingLoading);
        const knownTitleHref = pendingImportKnownTitleHref(item);
        const knownTitleLabel = pendingImportKnownTitleLabel(item);
        const libraryLabel = item.libraryName ?? libraryNameById.get(item.libraryId) ?? item.libraryId;

        return (
          <Card key={item.id} className="border-border/80 bg-card/60">
            <CardHeader className="space-y-2">
              <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
                <div className="space-y-1">
                  <CardTitle className="text-base">{item.displayName}</CardTitle>
                  <p className="text-sm text-muted-foreground">{summarizePendingImport(item)}</p>
                  <p className="text-xs text-muted-foreground">Library: {libraryLabel}</p>
                </div>
                <div className="flex flex-col gap-2 sm:flex-row">
                  <Button
                    type="button"
                    size="sm"
                    variant={isActive ? "secondary" : item.status === "ignored" ? "outline" : "default"}
                    onClick={() => handleOpenSearch(item)}
                    disabled={isBusy}
                  >
                    {item.titleId ? null : <Search className="mr-2 h-4 w-4" />}
                    {item.titleId ? "Bind Episodes" : t("pendingImports.searchAction")}
                  </Button>
                  {item.status === "pending" ? (
                    <Button
                      type="button"
                      size="sm"
                      variant="destructive"
                      onClick={() => handleRequestIgnore(item)}
                      disabled={isBusy}
                    >
                      {t("pendingImports.ignore")}
                    </Button>
                  ) : null}
                </div>
              </div>
            </CardHeader>
            <CardContent className="space-y-3 text-sm">
              <div>
                <span className="font-medium text-foreground">{t("pendingImports.path")}:</span>{" "}
                <span className="break-all text-muted-foreground">{item.path}</span>
              </div>
              {item.folderPath ? (
                <div>
                  <span className="font-medium text-foreground">{t("pendingImports.folderPath")}:</span>{" "}
                  <span className="break-all text-muted-foreground">{item.folderPath}</span>
                </div>
              ) : null}
              {item.titleId ? (
                <div>
                  <span className="font-medium text-foreground">Known title:</span>{" "}
                  {knownTitleHref ? (
                    <Link
                      to={knownTitleHref}
                      className="break-all text-primary underline-offset-4 hover:underline"
                    >
                      {knownTitleLabel}
                    </Link>
                  ) : (
                    <span className="break-all text-muted-foreground">{knownTitleLabel}</span>
                  )}
                </div>
              ) : null}
              {isActive ? (
                <div className="space-y-3 rounded-lg border border-border/80 bg-background/60 p-3">
                  {item.titleId ? (
                    <>
                      {bindingLoading ? (
                        <div className="flex items-center gap-2 text-sm text-muted-foreground">
                          <Loader2 className="h-4 w-4 animate-spin" />
                          Loading episode bindings...
                        </div>
                      ) : null}
                      {bindingError ? (
                        <div className="text-sm text-destructive">{bindingError}</div>
                      ) : null}
                      {bindingPreview ? (
                        <div className="space-y-4">
                          <div className="space-y-1">
                            <div className="text-sm font-medium text-foreground">
                              {bindingPreview.title.name}
                            </div>
                            <div className="text-xs text-muted-foreground">
                              {bindingPreview.file.fileName}
                            </div>
                            <div className="text-xs text-muted-foreground">
                              Parsed hints:
                              {bindingPreview.file.parsedSeason != null
                                ? ` season ${bindingPreview.file.parsedSeason}`
                                : ""}
                              {bindingPreview.file.parsedEpisodes.length > 0
                                ? ` • episodes ${bindingPreview.file.parsedEpisodes.join(", ")}`
                                : ""}
                              {bindingPreview.file.parsedAbsoluteNumbers.length > 0
                                ? ` • absolute ${bindingPreview.file.parsedAbsoluteNumbers.join(", ")}`
                                : ""}
                            </div>
                          </div>

                          <div className="flex flex-wrap gap-2">
                            <Button
                              type="button"
                              size="sm"
                              variant="outline"
                              disabled={isBusy}
                              onClick={() => {
                                const suggestedEpisodeIds = bindingPreview.file.suggestedEpisodeIds;
                                setSelectedEpisodeIds(suggestedEpisodeIds);
                                setExpandedBindingSeasonKeys(
                                  bindingSeasonKeysForSelection(
                                    bindingPreview.availableEpisodes,
                                    suggestedEpisodeIds,
                                  ),
                                );
                              }}
                            >
                              Use Suggested
                            </Button>
                            <Button
                              type="button"
                              size="sm"
                              variant="outline"
                              disabled={isBusy}
                              onClick={() => {
                                setSelectedEpisodeIds([]);
                                setExpandedBindingSeasonKeys([]);
                              }}
                            >
                              Clear
                            </Button>
                          </div>

                          <div className="space-y-4">
                            {bindingGroups.map(([seasonKey, episodes]) => {
                              const seasonOpen = expandedBindingSeasonKeys.includes(seasonKey);
                              return (
                                <Collapsible
                                  key={seasonKey}
                                  open={seasonOpen}
                                  onOpenChange={(open) =>
                                    setExpandedBindingSeasonKeys((current) =>
                                      open
                                        ? current.includes(seasonKey)
                                          ? current
                                          : [...current, seasonKey]
                                        : current.filter((key) => key !== seasonKey),
                                    )
                                  }
                                  className="space-y-2"
                                >
                                  <div className="flex items-center justify-between gap-3">
                                    <CollapsibleTrigger asChild>
                                      <button
                                        type="button"
                                        className="flex min-w-0 items-center gap-2 text-left text-sm font-medium text-foreground"
                                      >
                                        <ChevronDown
                                          className={`h-4 w-4 shrink-0 transition-transform ${
                                            seasonOpen ? "rotate-0" : "-rotate-90"
                                          }`}
                                        />
                                        <span>
                                          {seasonKey === "specials"
                                            ? "Specials"
                                            : `Season ${seasonKey}`}
                                        </span>
                                      </button>
                                    </CollapsibleTrigger>
                                    <Button
                                      type="button"
                                      size="sm"
                                      variant="ghost"
                                      disabled={isBusy}
                                      onClick={() => {
                                        setSelectedEpisodeIds((current) => {
                                          const next = new Set(current);
                                          for (const episode of episodes) {
                                            next.add(episode.id);
                                          }
                                          return Array.from(next);
                                        });
                                        setExpandedBindingSeasonKeys((current) =>
                                          current.includes(seasonKey)
                                            ? current
                                            : [...current, seasonKey],
                                        );
                                      }}
                                    >
                                      Select Season
                                    </Button>
                                  </div>
                                  <CollapsibleContent>
                                    <div className="space-y-2 rounded-md border border-border/70 p-3">
                                      {episodes.map((episode) => {
                                        const episodeDisplay =
                                          formatBindingEpisodeDisplay(episode);
                                        return (
                                          <label
                                            key={episode.id}
                                            className="flex items-start gap-3 text-sm text-foreground"
                                          >
                                            <Checkbox
                                              checked={selectedEpisodeIds.includes(episode.id)}
                                              onCheckedChange={(checked) =>
                                                toggleEpisodeSelection(
                                                  episode.id,
                                                  Boolean(checked),
                                                )
                                              }
                                              disabled={isBusy}
                                            />
                                            <span className="min-w-0">
                                              {episodeDisplay.showSeparateLabel ? (
                                                <>
                                                  <span className="font-medium">
                                                    {episodeDisplay.key}
                                                  </span>
                                                  <span className="ml-2 text-muted-foreground">
                                                    {episodeDisplay.label}
                                                  </span>
                                                </>
                                              ) : (
                                                <span>
                                                  {episodeDisplay.key ?? episodeDisplay.label}
                                                </span>
                                              )}
                                            </span>
                                          </label>
                                        );
                                      })}
                                    </div>
                                  </CollapsibleContent>
                                </Collapsible>
                              );
                            })}
                          </div>

                          <div className="flex justify-end">
                            <Button
                              type="button"
                              disabled={isBusy || selectedEpisodeIds.length === 0}
                              onClick={() => void handleBind()}
                            >
                              {isResolving ? <Loader2 className="mr-2 h-4 w-4 animate-spin" /> : null}
                              Bind Selected Episodes
                            </Button>
                          </div>
                        </div>
                      ) : null}
                    </>
                  ) : (
                    <>
                      <Input
                        value={searchQuery}
                        onChange={(event) => setSearchQuery(event.target.value)}
                        placeholder={t("pendingImports.searchPlaceholder")}
                        disabled={isBusy}
                      />

                      {searching ? (
                        <div className="flex items-center gap-2 text-sm text-muted-foreground">
                          <Loader2 className="h-4 w-4 animate-spin" />
                          {t("pendingImports.searching")}
                        </div>
                      ) : null}

                      {!searching && searchQuery.trim() && searchResults.length === 0 ? (
                        <div className="text-sm text-muted-foreground">
                          {t("pendingImports.noSearchResults")}
                        </div>
                      ) : null}

                      <div className="space-y-3">
                        {searchResults.map((result) => (
                          <div
                            key={`${item.id}-${result.tvdbId}`}
                            className="flex gap-3 rounded-lg border border-border bg-card/40 p-3"
                          >
                            <div className="h-24 w-16 flex-none overflow-hidden rounded-md border border-border bg-muted">
                              {result.posterUrl ? (
                                <TitlePoster src={result.posterUrl} alt={result.name} />
                              ) : null}
                            </div>
                            <div className="min-w-0 flex-1 space-y-1">
                              <div className="flex flex-wrap items-center gap-2">
                                <span className="font-medium text-foreground">{result.name}</span>
                                {result.year ? (
                                  <span className="text-xs text-muted-foreground">{result.year}</span>
                                ) : null}
                                <span className="text-xs text-muted-foreground">TVDB {result.tvdbId}</span>
                              </div>
                              {result.status ? (
                                <div className="text-xs text-muted-foreground">{result.status}</div>
                              ) : null}
                              {result.overview ? (
                                <p className="line-clamp-3 text-sm text-muted-foreground">
                                  {result.overview}
                                </p>
                              ) : null}
                            </div>
                            <div className="flex flex-none items-start">
                              <Button
                                type="button"
                                size="sm"
                                onClick={() => void handleResolve(String(result.tvdbId))}
                                disabled={isBusy}
                              >
                                {t("pendingImports.match")}
                              </Button>
                            </div>
                          </div>
                        ))}
                      </div>
                    </>
                  )}

                  <div className="flex justify-end">
                    <Button
                      type="button"
                      variant="ghost"
                      onClick={clearActiveItem}
                      disabled={isBusy}
                    >
                      {t("label.cancel")}
                    </Button>
                  </div>
                </div>
              ) : null}
            </CardContent>
          </Card>
        );
      })}
    </div>
  ), [
    activeItemRef,
    bindingError,
    bindingGroups,
    bindingLoading,
    bindingPreview,
    clearActiveItem,
    expandedBindingSeasonKeys,
    handleBind,
    handleOpenSearch,
    handleRequestIgnore,
    handleResolve,
    ignoringItemId,
    libraryNameById,
    resolvingItemId,
    searchQuery,
    searchResults,
    searching,
    selectedEpisodeIds,
    t,
    toggleEpisodeSelection,
  ]);

  return (
    <>
    <div className="space-y-4">
      <Card className="border-border/80 bg-card/70">
        <CardHeader>
          <CardTitle>{t("pendingImports.title")}</CardTitle>
          <p className="text-sm text-muted-foreground">
            {t("pendingImports.description", {
              facet: facet ? t(facet.navLabelKey) : view,
            })}
          </p>
        </CardHeader>
        <CardContent>
          <LibraryMultiSelect
            libraries={libraries}
            selectedLibraryIds={selectedLibraryIds}
            onSelectedLibraryIdsChange={(libraryIds) => {
              setSelectedLibraryIds(libraryIds);
              setPendingOffset(0);
              setIgnoredOffset(0);
            }}
            disabled={librariesLoading || libraries.length === 0}
            triggerClassName="w-full sm:w-[220px]"
          />
        </CardContent>
      </Card>

      {pendingLoading || ignoredLoading ? (
        <Card>
          <CardContent className="flex items-center gap-2 py-6 text-sm text-muted-foreground">
            <Loader2 className="h-4 w-4 animate-spin" />
            {t("pendingImports.loading")}
          </CardContent>
        </Card>
      ) : null}

      {pendingError ? (
        <Card className="border-red-500/30 bg-red-500/10">
          <CardContent className="py-4 text-sm text-red-200">{pendingError}</CardContent>
        </Card>
      ) : null}

      {!pendingLoading && !pendingError && pendingConnection.total === 0 ? (
        <Card>
          <CardContent className="py-6 text-sm text-muted-foreground">
            {t("pendingImports.empty")}
          </CardContent>
        </Card>
      ) : null}

      {renderItems(pendingConnection.items)}

      {renderPagination(
        "pending",
        pendingConnection.total,
        pendingHasPrevPage,
        pendingHasNextPage,
        pendingPageStart,
        pendingPageEnd,
      )}

      {ignoredConnection.total > 0 ? (
        <Card className="border-border/80 bg-card/50">
          <Collapsible open={ignoredOpen} onOpenChange={setIgnoredOpen}>
            <CollapsibleTrigger asChild>
              <button
                type="button"
                className="flex w-full items-center justify-between gap-3 px-4 py-4 text-left"
              >
                <div className="text-base font-semibold text-foreground">
                  {t("pendingImports.ignoredSection")}
                </div>
                <ChevronDown className={`h-4 w-4 text-muted-foreground transition-transform ${ignoredOpen ? "rotate-180" : ""}`} />
              </button>
            </CollapsibleTrigger>
            <CollapsibleContent className="space-y-4 px-4 pb-4">
              {ignoredError ? (
                <Card className="border-red-500/30 bg-red-500/10">
                  <CardContent className="py-4 text-sm text-red-200">{ignoredError}</CardContent>
                </Card>
              ) : null}

              {renderItems(ignoredConnection.items)}

              {renderPagination(
                "ignored",
                ignoredConnection.total,
                ignoredHasPrevPage,
                ignoredHasNextPage,
                ignoredPageStart,
                ignoredPageEnd,
              )}
            </CollapsibleContent>
          </Collapsible>
        </Card>
      ) : null}
    </div>
    <ConfirmDialog
      open={Boolean(ignoreTargetItem && ignoreTargetItem.status === "pending")}
      title={t("pendingImports.ignoreConfirmTitle")}
      description=""
      confirmLabel={t("pendingImports.ignore")}
      cancelLabel={t("label.cancel")}
      isBusy={Boolean(ignoreTargetItem && ignoringItemId === ignoreTargetItem.id)}
      onConfirm={() => void handleIgnore()}
      onCancel={() => setIgnoreTargetItem(null)}
    />
    </>
  );
});
