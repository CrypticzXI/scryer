import * as React from "react";
import { Loader2, Search } from "lucide-react";
import { useNavigate } from "react-router-dom";
import { useClient } from "urql";

import { TitlePoster } from "@/components/title-poster";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import type { ViewId } from "@/components/root/types";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";
import { resolvePendingImportMutation } from "@/lib/graphql/mutations";
import { pendingImportsQuery, searchMetadataQuery } from "@/lib/graphql/queries";
import { facetForView } from "@/lib/facets/registry";
import type {
  PendingImportConnection,
  PendingImportItem,
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
  const navigate = useNavigate();
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const facet = facetForView(view);

  const [connection, setConnection] = React.useState<PendingImportConnection>({
    total: 0,
    items: [],
  });
  const [offset, setOffset] = React.useState(0);
  const [loading, setLoading] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [activeItemId, setActiveItemId] = React.useState<string | null>(null);
  const [searchQuery, setSearchQuery] = React.useState("");
  const [searchResults, setSearchResults] = React.useState<MetadataSearchResult[]>([]);
  const [searching, setSearching] = React.useState(false);
  const [resolvingItemId, setResolvingItemId] = React.useState<string | null>(null);

  const activeItem = React.useMemo(
    () => connection.items.find((item) => item.id === activeItemId) ?? null,
    [activeItemId, connection.items],
  );
  const hasPrevPage = offset > 0;
  const hasNextPage = offset + PAGE_SIZE < connection.total;
  const pageStart = connection.total === 0 ? 0 : offset + 1;
  const pageEnd = Math.min(offset + PAGE_SIZE, connection.total);

  const fetchPendingImportsPage = React.useCallback(
    async (pageOffset: number): Promise<PendingImportConnection> => {
      const { data, error: queryError } = await client
        .query(pendingImportsQuery, {
          facet: pendingImportFacetValueForView(view),
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
    async (pageOffset: number): Promise<PendingImportConnection | null> => {
      setLoading(true);
      setError(null);

      try {
        let nextOffset = Math.max(0, pageOffset);
        let nextConnection = await fetchPendingImportsPage(nextOffset);

        if (nextConnection.total > 0 && nextOffset >= nextConnection.total) {
          nextOffset = Math.max(
            0,
            Math.floor((nextConnection.total - 1) / PAGE_SIZE) * PAGE_SIZE,
          );
          nextConnection = await fetchPendingImportsPage(nextOffset);
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

  React.useEffect(() => {
    void refresh(offset);
  }, [offset, refresh]);

  React.useEffect(() => {
    setOffset(0);
    setActiveItemId(null);
    setSearchQuery("");
    setSearchResults([]);
  }, [view]);

  React.useEffect(() => {
    if (!activeItemId) {
      setSearchResults([]);
      setSearching(false);
      return;
    }
  }, [activeItemId]);

  React.useEffect(() => {
    if (!activeItemId) {
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
  }, [activeItemId, client, facet?.tvdbSearchType, searchQuery, setGlobalStatus, t]);

  React.useEffect(() => {
    if (!activeItemId) {
      return;
    }

    if (connection.items.some((item) => item.id === activeItemId)) {
      return;
    }

    setActiveItemId(null);
    setSearchQuery("");
    setSearchResults([]);
  }, [activeItemId, connection.items]);

  const handleOpenSearch = React.useCallback((item: PendingImportItem) => {
    const seedQuery = item.query.trim() || item.displayName.trim();
    setActiveItemId(item.id);
    setSearchQuery(seedQuery);
    setSearchResults([]);
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

      const refreshed = await refresh(offset);
      if (!refreshed) {
        return;
      }

      setGlobalStatus(
        t("pendingImports.resolveSuccess", {
          name: result?.title?.name?.trim() || activeItem.displayName,
        }),
      );

      if (refreshed.total === 0) {
        navigate(`/${view}`);
        return;
      }

      setActiveItemId(null);
      setSearchQuery("");
      setSearchResults([]);
    } catch (err) {
      setGlobalStatus(err instanceof Error ? err.message : t("pendingImports.resolveFailed"));
    } finally {
      setResolvingItemId(null);
    }
  }, [activeItem, client, navigate, offset, refresh, setGlobalStatus, t, view]);

  return (
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

      {loading ? (
        <Card>
          <CardContent className="flex items-center gap-2 py-6 text-sm text-muted-foreground">
            <Loader2 className="h-4 w-4 animate-spin" />
            {t("pendingImports.loading")}
          </CardContent>
        </Card>
      ) : null}

      {error ? (
        <Card className="border-red-500/30 bg-red-500/10">
          <CardContent className="py-4 text-sm text-red-200">{error}</CardContent>
        </Card>
      ) : null}

      {!loading && !error && connection.total === 0 ? (
        <Card>
          <CardContent className="py-6 text-sm text-muted-foreground">
            {t("pendingImports.empty")}
          </CardContent>
        </Card>
      ) : null}

      <div className="space-y-4">
        {connection.items.map((item) => {
          const isActive = item.id === activeItemId;
          const isResolving = resolvingItemId === item.id;

          return (
            <Card key={item.id} className="border-border/80 bg-card/60">
              <CardHeader className="space-y-2">
                <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
                  <div className="space-y-1">
                    <CardTitle className="text-base">{item.displayName}</CardTitle>
                    <p className="text-sm text-muted-foreground">{summarizePendingImport(item)}</p>
                  </div>
                  <Button
                    type="button"
                    size="sm"
                    variant={isActive ? "secondary" : "default"}
                    onClick={() => handleOpenSearch(item)}
                    disabled={isResolving}
                  >
                    <Search className="mr-2 h-4 w-4" />
                    {t("pendingImports.searchAction")}
                  </Button>
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
                {item.searchAttempts.length > 0 ? (
                  <div className="space-y-1">
                    <div className="font-medium text-foreground">{t("pendingImports.searchAttempts")}</div>
                    <div className="space-y-1 text-muted-foreground">
                      {item.searchAttempts.map((attempt) => (
                        <div key={`${item.id}-${attempt.query}-${attempt.resultCount}`}>{attempt.summary}</div>
                      ))}
                    </div>
                  </div>
                ) : null}

                {isActive ? (
                  <div className="space-y-3 rounded-lg border border-border/80 bg-background/60 p-3">
                    <Input
                      value={searchQuery}
                      onChange={(event) => setSearchQuery(event.target.value)}
                      placeholder={t("pendingImports.searchPlaceholder")}
                      disabled={isResolving}
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
                      {searchResults.map((result) => {
                        return (
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
                                disabled={isResolving}
                              >
                                {isResolving ? (
                                  <>
                                    <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                                    {t("pendingImports.resolving")}
                                  </>
                                ) : (
                                  t("pendingImports.match")
                                )}
                              </Button>
                            </div>
                          </div>
                        );
                      })}
                    </div>

                    <div className="flex justify-end">
                      <Button
                        type="button"
                        variant="ghost"
                        onClick={() => setActiveItemId(null)}
                        disabled={isResolving}
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

      {!loading && !error && connection.total > PAGE_SIZE ? (
        <div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
          <Button
            className="w-full sm:w-auto"
            size="sm"
            variant="outline"
            disabled={!hasPrevPage}
            onClick={() => setOffset(Math.max(0, offset - PAGE_SIZE))}
          >
            {t("pendingImports.prev")}
          </Button>
          <span className="text-sm text-muted-foreground">
            {t("pendingImports.pageRange", {
              start: pageStart,
              end: pageEnd,
              total: connection.total,
            })}
          </span>
          <Button
            className="w-full sm:w-auto"
            size="sm"
            variant="outline"
            disabled={!hasNextPage}
            onClick={() => setOffset(offset + PAGE_SIZE)}
          >
            {t("pendingImports.next")}
          </Button>
        </div>
      ) : null}
    </div>
  );
});
