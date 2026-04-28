import * as React from "react";
import { ChevronDown, Loader2, Search } from "lucide-react";
import { useClient } from "urql";

import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { TitlePoster } from "@/components/title-poster";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { Input } from "@/components/ui/input";
import type { ViewId } from "@/components/root/types";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";
import { facetForView } from "@/lib/facets/registry";
import { ignorePendingImportMutation, resolvePendingImportMutation } from "@/lib/graphql/mutations";
import { pendingImportsQuery, searchMetadataQuery } from "@/lib/graphql/queries";
import type {
  PendingImportConnection,
  PendingImportItem,
  PendingImportStatus,
  ResolvePendingImportResult,
} from "@/lib/types";
import { pendingImportFacetValueForView } from "@/lib/types";

type PendingImportsContainerProps = {
  view: ViewId;
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

export const PendingImportsContainer = React.memo(function PendingImportsContainer({
  view,
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
  const [resolvingItemId, setResolvingItemId] = React.useState<string | null>(null);
  const [ignoringItemId, setIgnoringItemId] = React.useState<string | null>(null);
  const [ignoreTargetItem, setIgnoreTargetItem] = React.useState<PendingImportItem | null>(null);

  const clearActiveItem = React.useCallback(() => {
    setActiveItemRef(null);
    setSearchQuery("");
    setSearchResults([]);
    setSearching(false);
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
          status,
          limit: PAGE_SIZE,
          offset: pageOffset,
        })
        .toPromise();
      if (queryError) {
        throw queryError;
      }

      return (data?.pendingImports ?? {
        total: 0,
        items: [],
      }) as PendingImportConnection;
    },
    [client, view],
  );

  const refresh = React.useCallback(
    async (
      status: PendingImportStatus,
      pageOffset: number,
    ): Promise<PendingImportConnection | null> => {
      const setLoading = status === "ignored" ? setIgnoredLoading : setPendingLoading;
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
    setIgnoredOpen(false);
    clearActiveItem();
  }, [clearActiveItem, view]);

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

    let cancelled = false;
    const timeoutId = window.setTimeout(() => {
      setSearching(true);
      client
        .query(searchMetadataQuery, {
          query: searchQuery.trim(),
          type: facet?.tvdbSearchType ?? "movie",
          limit: 8,
          year: activeItem?.yearHint ?? null,
        })
        .toPromise()
        .then(({ data, error: queryError }) => {
          if (queryError) {
            throw queryError;
          }
          if (cancelled) {
            return;
          }
          const items = (data?.searchMetadata ?? []) as MetadataSearchResult[];
          setSearchResults(items);
        })
        .catch((err: unknown) => {
          if (cancelled) {
            return;
          }
          setSearchResults([]);
          setGlobalStatus(
            err instanceof Error ? err.message : t("pendingImports.searchFailed"),
          );
        })
        .finally(() => {
          if (!cancelled) {
            setSearching(false);
          }
        });
    }, 220);

    return () => {
      cancelled = true;
      window.clearTimeout(timeoutId);
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
    const seedQuery = item.query.trim() || item.displayName.trim();
    setActiveItemRef({ id: item.id, status: item.status });
    setSearchQuery(seedQuery);
    setSearchResults([]);
  }, []);

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
      window.dispatchEvent(new CustomEvent("scryer:pendingImportsRefresh"));
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

  const handleIgnore = React.useCallback(async () => {
    if (!ignoreTargetItem || ignoreTargetItem.status !== "pending") {
      return;
    }

    setIgnoringItemId(ignoreTargetItem.id);
    try {
      const { error: mutationError } = await client
        .mutation(ignorePendingImportMutation, {
          input: {
            pendingImportId: ignoreTargetItem.id,
          },
        })
        .toPromise();
      if (mutationError) {
        throw mutationError;
      }

      window.dispatchEvent(new CustomEvent("scryer:pendingImportsRefresh"));
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
        const isBusy = isResolving || isIgnoring;

        return (
          <Card key={item.id} className="border-border/80 bg-card/60">
            <CardHeader className="space-y-2">
              <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
                <div className="space-y-1">
                  <CardTitle className="text-base">{item.displayName}</CardTitle>
                  <p className="text-sm text-muted-foreground">{summarizePendingImport(item)}</p>
                </div>
                <div className="flex flex-col gap-2 sm:flex-row">
                  <Button
                    type="button"
                    size="sm"
                    variant={isActive ? "secondary" : item.status === "ignored" ? "outline" : "default"}
                    onClick={() => handleOpenSearch(item)}
                    disabled={isBusy}
                  >
                    <Search className="mr-2 h-4 w-4" />
                    {t("pendingImports.searchAction")}
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
              {isActive ? (
                <div className="space-y-3 rounded-lg border border-border/80 bg-background/60 p-3">
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
    clearActiveItem,
    handleOpenSearch,
    handleRequestIgnore,
    handleResolve,
    ignoringItemId,
    resolvingItemId,
    searchQuery,
    searchResults,
    searching,
    t,
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
