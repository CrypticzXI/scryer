import { Fragment } from "react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { SearchResultBuckets } from "@/components/common/release-search-results";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { Loader2, Search, Zap } from "lucide-react";
import { Link } from "react-router-dom";
import { useTranslate } from "@/lib/context/translate-context";
import { useIsMobile } from "@/lib/hooks/use-mobile";
import { buildOverviewDetailPath } from "@/lib/utils/routing";
import type { Facet, Release } from "@/lib/types";
import type { ViewId } from "@/components/root/types";

export type CutoffUnmetItem = {
  titleId: string;
  titleName: string;
  titleSlug?: string | null;
  titleFacet: Facet;
  episodeId?: string | null;
  seasonNumber?: string | null;
  episodeNumber?: string | null;
  currentTier: string;
  targetTier: string;
};

type CutoffUnmetViewState = {
  items: CutoffUnmetItem[];
  loading: boolean;
  facetFilter: string | undefined;
  setFacetFilter: (v: string | undefined) => void;
  autoSearchingId: string | null;
  interactiveSearchingId: string | null;
  activeInteractiveItemId: string | null;
  searchResultsByItemId: Record<string, Release[]>;
  bulkSearching: boolean;
  bulkProgress: { current: number; total: number } | null;
  triggerAutoSearch: (item: CutoffUnmetItem) => Promise<void>;
  triggerInteractiveSearch: (item: CutoffUnmetItem) => Promise<void>;
  queueRelease: (item: CutoffUnmetItem, release: Release) => Promise<void>;
  triggerBulkSearch: () => void;
  cancelBulkSearch: () => void;
};

function cutoffItemKey(item: CutoffUnmetItem) {
  return item.episodeId?.trim() || item.titleId;
}

function cutoffEpisodeCode(item: CutoffUnmetItem): string | null {
  const seasonDigits = item.seasonNumber?.match(/\d+/)?.[0] ?? null;
  const episodeDigits = item.episodeNumber?.match(/\d+/)?.[0] ?? null;
  if (!seasonDigits || !episodeDigits) {
    return null;
  }
  return `S${seasonDigits.padStart(2, "0")}E${episodeDigits.padStart(2, "0")}`;
}

function cutoffOverviewView(facet: Facet): ViewId | null {
  switch (facet) {
    case "movie":
      return "movies";
    case "series":
      return "series";
    case "anime":
      return "anime";
    default:
      return null;
  }
}

function cutoffOverviewHref(item: CutoffUnmetItem, includeEpisode: boolean): string | null {
  const targetView = cutoffOverviewView(item.titleFacet);
  if (!targetView) {
    return null;
  }

  const normalizedSlug = item.titleSlug?.trim() || null;
  const targetPath = buildOverviewDetailPath(targetView, normalizedSlug);
  const params = new URLSearchParams();
  if (!normalizedSlug) {
    params.set("id", item.titleId);
  }
  if (includeEpisode && item.episodeId) {
    params.set("episodeId", item.episodeId);
  }

  const query = params.toString();
  return `${targetPath}${query ? `?${query}` : ""}`;
}

function qualityBadge(tier: string, variant: "current" | "target") {
  const cls =
    variant === "current"
      ? "bg-amber-500/20 text-amber-400"
      : "bg-green-500/20 text-green-400";
  return (
    <span
      className={`inline-block rounded px-2 py-0.5 text-xs font-medium ${cls}`}
    >
      {tier}
    </span>
  );
}

function TitleCell({ item }: { item: CutoffUnmetItem }) {
  const titleHref = cutoffOverviewHref(item, false);
  const episodeHref = cutoffOverviewHref(item, true);
  const episodeCode = cutoffEpisodeCode(item);

  return (
    <div className="space-y-1">
      {titleHref ? (
        <Link
          to={titleHref}
          className="font-medium text-foreground underline-offset-4 hover:underline"
        >
          {item.titleName}
        </Link>
      ) : (
        <span className="font-medium text-foreground">{item.titleName}</span>
      )}
      {episodeCode ? (
        episodeHref ? (
          <Link
            to={episodeHref}
            className="block text-sm text-muted-foreground underline-offset-4 hover:text-foreground hover:underline"
          >
            {episodeCode}
          </Link>
        ) : (
          <span className="block text-sm text-muted-foreground">{episodeCode}</span>
        )
      ) : null}
    </div>
  );
}

function ActionButtons({
  item,
  autoSearchingId,
  interactiveSearchingId,
  bulkSearching,
  triggerAutoSearch,
  triggerInteractiveSearch,
}: {
  item: CutoffUnmetItem;
  autoSearchingId: string | null;
  interactiveSearchingId: string | null;
  bulkSearching: boolean;
  triggerAutoSearch: (item: CutoffUnmetItem) => Promise<void>;
  triggerInteractiveSearch: (item: CutoffUnmetItem) => Promise<void>;
}) {
  const t = useTranslate();
  const itemKey = cutoffItemKey(item);
  const autoSearching = autoSearchingId === itemKey;
  const interactiveSearching = interactiveSearchingId === itemKey;

  return (
    <div className="flex flex-wrap gap-2">
      <Button
        size="sm"
        disabled={autoSearching || interactiveSearching || bulkSearching}
        onClick={() => void triggerAutoSearch(item)}
      >
        {autoSearching ? (
          <Loader2 className="mr-1 h-4 w-4 animate-spin" />
        ) : (
          <Zap className="mr-1 h-4 w-4" />
        )}
        {t("label.autoSearch")}
      </Button>
      <Button
        size="sm"
        variant="outline"
        disabled={autoSearching || interactiveSearching || bulkSearching}
        onClick={() => void triggerInteractiveSearch(item)}
      >
        {interactiveSearching ? (
          <Loader2 className="mr-1 h-4 w-4 animate-spin" />
        ) : (
          <Search className="mr-1 h-4 w-4" />
        )}
        {t("label.interactiveSearch")}
      </Button>
    </div>
  );
}

export function CutoffUnmetView({ state }: { state: CutoffUnmetViewState }) {
  const t = useTranslate();
  const isMobile = useIsMobile();
  const {
    items,
    loading,
    facetFilter,
    setFacetFilter,
    autoSearchingId,
    interactiveSearchingId,
    activeInteractiveItemId,
    searchResultsByItemId,
    bulkSearching,
    bulkProgress,
    triggerAutoSearch,
    triggerInteractiveSearch,
    queueRelease,
    triggerBulkSearch,
    cancelBulkSearch,
  } = state;

  const filtered = facetFilter
    ? items.filter((item) => item.titleFacet === facetFilter)
    : items;

  return (
    <Card>
      <CardHeader>
        <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
          <CardTitle>{t("cutoff.title")}</CardTitle>
          <div className="flex flex-col gap-2 sm:flex-row sm:items-center">
            {bulkSearching && bulkProgress ? (
              <>
                <span className="text-sm text-muted-foreground">
                  {t("cutoff.searchProgress", {
                    current: bulkProgress.current,
                    total: bulkProgress.total,
                  })}
                </span>
                <Button
                  size="sm"
                  variant="destructive"
                  className="w-full sm:w-auto"
                  onClick={cancelBulkSearch}
                >
                  {t("label.cancel")}
                </Button>
              </>
            ) : (
              <Button
                size="sm"
                className="w-full sm:w-auto"
                onClick={triggerBulkSearch}
                disabled={filtered.length === 0 || loading}
              >
                <Search className="mr-1 h-3 w-3" />
                {t("cutoff.searchAll")}
              </Button>
            )}
          </div>
        </div>
      </CardHeader>
      <CardContent>
        <div className="mb-4 flex flex-col gap-3 sm:flex-row sm:flex-wrap">
          <Select
            value={facetFilter ?? "__all__"}
            onValueChange={(value) =>
              setFacetFilter(value === "__all__" ? undefined : value)
            }
          >
            <SelectTrigger className="w-full sm:w-[150px]">
              <SelectValue placeholder={t("cutoff.filterFacet")} />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="__all__">{t("cutoff.allFacets")}</SelectItem>
              <SelectItem value="movie">movie</SelectItem>
              <SelectItem value="series">series</SelectItem>
              <SelectItem value="anime">anime</SelectItem>
            </SelectContent>
          </Select>

          <span className="self-center text-sm text-muted-foreground sm:ml-auto">
            {t("cutoff.totalCount", { count: filtered.length })}
          </span>
        </div>

        {isMobile ? (
          filtered.length === 0 && !loading ? (
            <p className="text-center text-muted-foreground">{t("cutoff.noItems")}</p>
          ) : (
            <div className="space-y-3">
              {filtered.map((item) => {
                const itemKey = cutoffItemKey(item);
                const searchResults = searchResultsByItemId[itemKey] ?? [];
                const showResults = activeInteractiveItemId === itemKey;

                return (
                  <div
                    key={itemKey}
                    className="space-y-3 rounded-xl border border-border bg-card/30 p-3"
                  >
                    <div className="space-y-3">
                      <TitleCell item={item} />
                      <div className="flex flex-wrap gap-2">
                        {qualityBadge(item.currentTier, "current")}
                        {qualityBadge(item.targetTier, "target")}
                      </div>
                      <ActionButtons
                        item={item}
                        autoSearchingId={autoSearchingId}
                        interactiveSearchingId={interactiveSearchingId}
                        bulkSearching={bulkSearching}
                        triggerAutoSearch={triggerAutoSearch}
                        triggerInteractiveSearch={triggerInteractiveSearch}
                      />
                    </div>
                    {showResults ? (
                      <SearchResultBuckets
                        results={searchResults}
                        onQueue={(release) => queueRelease(item, release)}
                        requireCandidateToken
                      />
                    ) : null}
                  </div>
                );
              })}
            </div>
          )
        ) : (
          <div className="overflow-auto rounded-xl border border-border/60">
            <Table className="min-w-[900px]">
              <TableHeader>
                <TableRow>
                  <TableHead>{t("cutoff.colTitleEpisode")}</TableHead>
                  <TableHead>{t("cutoff.colCurrentQuality")}</TableHead>
                  <TableHead>{t("cutoff.colTargetQuality")}</TableHead>
                  <TableHead>{t("label.actions")}</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {filtered.map((item) => {
                  const itemKey = cutoffItemKey(item);
                  const searchResults = searchResultsByItemId[itemKey] ?? [];
                  const showResults = activeInteractiveItemId === itemKey;

                  return (
                    <Fragment key={itemKey}>
                      <TableRow>
                        <TableCell className="min-w-[320px] align-top">
                          <TitleCell item={item} />
                        </TableCell>
                        <TableCell className="align-top">
                          {qualityBadge(item.currentTier, "current")}
                        </TableCell>
                        <TableCell className="align-top">
                          {qualityBadge(item.targetTier, "target")}
                        </TableCell>
                        <TableCell className="align-top">
                          <ActionButtons
                            item={item}
                            autoSearchingId={autoSearchingId}
                            interactiveSearchingId={interactiveSearchingId}
                            bulkSearching={bulkSearching}
                            triggerAutoSearch={triggerAutoSearch}
                            triggerInteractiveSearch={triggerInteractiveSearch}
                          />
                        </TableCell>
                      </TableRow>
                      {showResults ? (
                        <TableRow>
                          <TableCell colSpan={4} className="bg-background/20">
                            <SearchResultBuckets
                              results={searchResults}
                              onQueue={(release) => queueRelease(item, release)}
                              requireCandidateToken
                            />
                          </TableCell>
                        </TableRow>
                      ) : null}
                    </Fragment>
                  );
                })}
                {filtered.length === 0 && !loading ? (
                  <TableRow>
                    <TableCell colSpan={4} className="text-center text-muted-foreground">
                      {t("cutoff.noItems")}
                    </TableCell>
                  </TableRow>
                ) : null}
              </TableBody>
            </Table>
          </div>
        )}
      </CardContent>
    </Card>
  );
}
