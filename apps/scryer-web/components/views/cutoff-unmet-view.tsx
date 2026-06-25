import { Fragment } from "react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { LibraryMultiSelect } from "@/components/common/library-multi-select";
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
import type { Facet, LibraryRecord, Release } from "@/lib/types";
import type { ViewId } from "@/components/root/types";

export type CutoffUnmetItem = {
  titleId: string;
  titleName: string;
  titleSlug?: string | null;
  titleFacet: Facet;
  libraryId: string;
  libraryName?: string | null;
  librarySlug?: string | null;
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
  libraries: LibraryRecord[];
  librariesLoading: boolean;
  selectedLibraryIds: string[];
  setSelectedLibraryIds: (value: string[]) => void;
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
  const targetPath = buildOverviewDetailPath(
    targetView,
    item.librarySlug ?? null,
    normalizedSlug,
  );
  const params = new URLSearchParams();
  if (!normalizedSlug || !item.librarySlug?.trim()) {
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
    libraries,
    librariesLoading,
    selectedLibraryIds,
    setSelectedLibraryIds,
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
    <Card className="overflow-hidden rounded-none border-0 bg-transparent shadow-none">
      <CardHeader className="border-b border-[var(--scry-border3)] bg-[linear-gradient(180deg,var(--scry-surfD),transparent)] px-4 py-4 sm:px-5">
        <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
          <CardTitle className="text-[22px] font-bold tracking-normal text-[var(--scry-ink2)]">
            {t("cutoff.title")}
          </CardTitle>
          <div className="flex flex-col gap-2 sm:flex-row sm:items-center">
            {bulkSearching && bulkProgress ? (
              <>
                <span className="text-sm font-medium text-[var(--scry-muted3)]">
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
      <CardContent className="bg-[color-mix(in_srgb,var(--scry-bg)_52%,transparent)] p-4 sm:p-5">
        <div className="mb-4 flex flex-col gap-3 rounded-[14px] border border-[var(--scry-border3)] bg-[var(--scry-surfC)] p-3 sm:flex-row sm:flex-wrap sm:items-center">
          <LibraryMultiSelect
            libraries={libraries}
            selectedLibraryIds={selectedLibraryIds}
            onSelectedLibraryIdsChange={setSelectedLibraryIds}
            disabled={librariesLoading}
            triggerClassName="h-10 w-full rounded-[10px] border-[var(--scry-border2)] bg-[var(--scry-inset)] text-[13px] text-[var(--scry-body)] shadow-none sm:w-[210px]"
          />

          <Select
            value={facetFilter ?? "__all__"}
            onValueChange={(value) =>
              setFacetFilter(value === "__all__" ? undefined : value)
            }
          >
            <SelectTrigger className="h-10 w-full rounded-[10px] border-[var(--scry-border2)] bg-[var(--scry-inset)] text-[13px] text-[var(--scry-body)] shadow-none sm:w-[150px]">
              <SelectValue placeholder={t("cutoff.filterFacet")} />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="__all__">{t("cutoff.allFacets")}</SelectItem>
              <SelectItem value="movie">movie</SelectItem>
              <SelectItem value="series">series</SelectItem>
              <SelectItem value="anime">anime</SelectItem>
            </SelectContent>
          </Select>

          <span className="self-center text-sm font-medium text-[var(--scry-muted3)] sm:ml-auto">
            {t("cutoff.totalCount", { count: filtered.length })}
          </span>
        </div>

        {isMobile ? (
          filtered.length === 0 && !loading ? (
            <p className="text-center text-[var(--scry-muted3)]">{t("cutoff.noItems")}</p>
          ) : (
            <div className="space-y-3">
              {filtered.map((item) => {
                const itemKey = cutoffItemKey(item);
                const searchResults = searchResultsByItemId[itemKey] ?? [];
                const showResults = activeInteractiveItemId === itemKey;

                return (
                  <div
                    key={itemKey}
                    className="space-y-3 rounded-[14px] border border-[var(--scry-border2)] bg-[var(--scry-surfC)] p-3 shadow-[0_12px_28px_rgba(2,6,23,0.10)]"
                  >
                    <div className="space-y-3">
                      <TitleCell item={item} />
                      <span className="text-xs text-muted-foreground">
                        {item.libraryName ?? item.libraryId}
                      </span>
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
          <div className="overflow-auto rounded-[14px] border border-[var(--scry-border2)] bg-[var(--scry-surfC)]">
            <Table className="min-w-[900px]">
              <TableHeader>
                <TableRow>
                  <TableHead>{t("cutoff.colTitleEpisode")}</TableHead>
                  <TableHead>Library</TableHead>
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
                        <TableCell className="align-top text-sm text-muted-foreground">
                          {item.libraryName ?? item.libraryId}
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
                          <TableCell colSpan={5} className="bg-background/20">
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
                    <TableCell colSpan={5} className="text-center text-muted-foreground">
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
