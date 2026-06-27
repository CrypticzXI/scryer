import * as React from "react";
import {
  ChevronDown,
  ChevronRight,
  Eye,
  EyeOff,
  Loader2,
  Zap,
} from "lucide-react";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import {
  Table,
  TableBody,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { SearchResultBuckets } from "@/components/common/release-search-results";
import { TitleSearchDownloadClientNotice } from "@/components/common/title-search-download-client-notice";
import { useTranslate } from "@/lib/context/translate-context";
import type { Release } from "@/lib/types";
import { cn } from "@/lib/utils";
import { useIsMobile } from "@/lib/hooks/use-mobile";
import {
  seriesOverviewSeasonMonitorId,
  seriesOverviewSeasonSectionId,
  seriesOverviewSeasonSearchId,
} from "@/lib/utils/dom-ids";
import {
  EpisodeProgressBar,
  getCollectionEpisodeProgressPresentation,
} from "@/components/views/media-content/title-table-shared";
import type {
  CollectionEpisode,
  EpisodeMediaFile,
  TitleCollection,
  TitleReleaseBlocklistEntry,
} from "@/components/containers/series-overview-container";
import type { ExternalSubtitleRecord } from "@/lib/types/subtitles";
import type { DownloadQueueItem } from "@/lib/types/download-queue";
import {
  formatFileSize,
  isEpisodeCountableForProgress,
  isSpecialsCollection,
  seasonHeading,
} from "./helpers";
import { EpisodeRow } from "./episode-row";
import {
  EpisodeTableActionButton,
  EMPTY_EPISODE_FILES,
  EMPTY_RELEASES,
  EMPTY_SUBTITLE_DOWNLOADS,
} from "./season-section-utils";

export { SeriesMovieTimelineSection } from "./series-movie-row";

export function SeasonSection({
  collection,
  episodes,
  expanded,
  facet,
  onToggle,
  initiallyOpenEpisodeId,
  mediaFilesByEpisode,
  downloadQueueItemByEpisodeId,
  releaseBlocklistEntries,
  clearingReleaseBlocklistEntryId,
  subtitleDownloads,
  onRefreshSubtitles,
  searchResultsByEpisode,
  searchLoadingByEpisode,
  searchBlockedByEpisode,
  onRunEpisodeSearch,
  onOpenEpisodeHistory,
  onQueueFromEpisodeSearch,
  onQueueAdditionalFromEpisodeSearch,
  autoSearchLoadingByEpisode,
  onAutoSearchEpisode,
  onClearReleaseBlocklistEntry,
  onSetCollectionMonitored,
  onSetEpisodeMonitored,
  seasonSearchResults,
  seasonSearchLoading,
  searchBlocked = false,
  onRunSeasonSearch,
  onQueueFromSeasonSearch,
  onDeleteFile,
}: {
  collection: TitleCollection;
  facet: string;
  episodes: CollectionEpisode[];
  expanded: boolean;
  onToggle: () => void;
  initiallyOpenEpisodeId?: string | null;
  mediaFilesByEpisode: Record<string, EpisodeMediaFile[]>;
  downloadQueueItemByEpisodeId?: Record<string, DownloadQueueItem | undefined>;
  subtitleDownloads?: ExternalSubtitleRecord[];
  onRefreshSubtitles?: () => Promise<void> | void;
  releaseBlocklistEntries: TitleReleaseBlocklistEntry[];
  clearingReleaseBlocklistEntryId?: string | null;
  searchResultsByEpisode: Record<string, Release[]>;
  searchLoadingByEpisode: Record<string, boolean>;
  searchBlockedByEpisode: Record<string, boolean>;
  autoSearchLoadingByEpisode: Record<string, boolean>;
  onClearReleaseBlocklistEntry?: (entryId: string) => Promise<void> | void;
  onRunEpisodeSearch?: (episode: CollectionEpisode) => void;
  onOpenEpisodeHistory?: (episode: CollectionEpisode) => void;
  onQueueFromEpisodeSearch?: (episode: CollectionEpisode, release: Release) => Promise<void> | void;
  onQueueAdditionalFromEpisodeSearch?: (episode: CollectionEpisode, release: Release) => Promise<void> | void;
  onAutoSearchEpisode?: (episode: CollectionEpisode) => void;
  onSetCollectionMonitored?: (collectionId: string, monitored: boolean) => Promise<void>;
  onSetEpisodeMonitored?: (episodeId: string, monitored: boolean) => Promise<void>;
  seasonSearchResults?: Release[];
  seasonSearchLoading?: boolean;
  searchBlocked?: boolean;
  onRunSeasonSearch?: () => void;
  onQueueFromSeasonSearch?: (collection: TitleCollection, release: Release) => Promise<void> | void;
  onDeleteFile?: (fileId: string) => void;
}) {
  const t = useTranslate();
  const isMobile = useIsMobile();
  const Chevron = expanded ? ChevronDown : ChevronRight;
  const [seasonToggling, setSeasonToggling] = React.useState(false);
  const stableSubtitleDownloads = subtitleDownloads ?? EMPTY_SUBTITLE_DOWNLOADS;

  const seasonCheckedState: boolean | "indeterminate" = React.useMemo(() => {
    if (episodes.length === 0) {
      return collection.monitored;
    }

    const monitoredCount = episodes.filter((episode) => episode.monitored).length;
    if (monitoredCount === 0) {
      return false;
    }
    if (monitoredCount === episodes.length) {
      return true;
    }
    return "indeterminate";
  }, [collection.monitored, episodes]);

  const episodeRangeLabel = React.useMemo(() => {
    if (!collection.firstEpisodeNumber && !collection.lastEpisodeNumber) {
      return null;
    }

    return t("title.episodeRange", {
      start: collection.firstEpisodeNumber ?? "?",
      end: collection.lastEpisodeNumber ?? "?",
    });
  }, [collection.firstEpisodeNumber, collection.lastEpisodeNumber, t]);

  const collectionMetrics = React.useMemo(() => {
    const uniqueFiles = new Map<string, EpisodeMediaFile>();
    let totalEpisodes = 0;
    let monitoredEpisodes = 0;
    let ownedEpisodes = 0;

    for (const episode of episodes) {
      if (!isEpisodeCountableForProgress(episode)) {
        continue;
      }

      totalEpisodes += 1;

      if (episode.monitored) {
        monitoredEpisodes += 1;
      }

      const episodeFiles = mediaFilesByEpisode[episode.id] ?? EMPTY_EPISODE_FILES;
      if (episodeFiles.length > 0) {
        ownedEpisodes += 1;
      }

      for (const file of episodeFiles) {
        if (!uniqueFiles.has(file.id)) {
          uniqueFiles.set(file.id, file);
        }
      }
    }

    let matchedSizeBytes = 0;
    for (const file of uniqueFiles.values()) {
      const sizeBytes = file.sizeBytes;
      if (Number.isFinite(sizeBytes) && sizeBytes > 0) {
        matchedSizeBytes += sizeBytes;
      }
    }

    return {
      totalEpisodes,
      monitoredEpisodes,
      ownedEpisodes,
      matchedSizeBytes,
    };
  }, [episodes, mediaFilesByEpisode]);

  const collectionEpisodeProgress = React.useMemo(
    () => {
      if (!collectionMetrics || collectionMetrics.totalEpisodes <= 0) {
        return null;
      }

      return getCollectionEpisodeProgressPresentation({
        ownedEpisodes: collectionMetrics.ownedEpisodes,
        totalEpisodes: collectionMetrics.totalEpisodes,
        monitoredEpisodes: collectionMetrics.monitoredEpisodes,
        monitored: seasonCheckedState !== false,
        t,
      });
    },
    [collectionMetrics, seasonCheckedState, t],
  );

  const collectionSizeLabel = React.useMemo(() => {
    const derivedSizeBytes = collectionMetrics?.matchedSizeBytes ?? 0;
    if (derivedSizeBytes > 0) {
      return formatFileSize(derivedSizeBytes);
    }

    return null;
  }, [collectionMetrics]);

  const isSpecials = isSpecialsCollection(collection);
  const showCollectionHeader = true;
  const showSectionContent = expanded;

  return (
    <div
      id={seriesOverviewSeasonSectionId(collection.id)}
      data-timeline-kind="collection"
      data-collection-id={collection.id}
      className="overflow-hidden rounded-lg border border-border bg-background/40"
    >
      {showCollectionHeader ? (
        <div
          role="button"
          tabIndex={0}
          aria-expanded={expanded}
          onClick={onToggle}
          onKeyDown={(event) => {
            if (event.key === "Enter" || event.key === " ") {
              event.preventDefault();
              onToggle();
            }
          }}
          className="flex w-full cursor-pointer flex-wrap items-center justify-between gap-3 bg-card/60 px-4 py-2 text-left transition hover:bg-accent/80"
        >
          <div className="flex items-center gap-2">
            <button
              id={seriesOverviewSeasonMonitorId(collection.id)}
              type="button"
              disabled={!onSetCollectionMonitored || seasonToggling}
              aria-label={t("title.seasonMonitored")}
              className={cn(
                "inline-flex size-6 shrink-0 items-center justify-center rounded transition-colors",
                seasonToggling && "opacity-50",
                seasonCheckedState === true
                  ? "text-emerald-600 dark:text-emerald-300"
                  : seasonCheckedState === "indeterminate"
                    ? "text-amber-500 dark:text-amber-400"
                    : "text-muted-foreground/60",
              )}
              onClick={(event) => {
                event.stopPropagation();
                if (!onSetCollectionMonitored) {
                  return;
                }

                setSeasonToggling(true);
                const nextMonitored = seasonCheckedState !== true;
                onSetCollectionMonitored(collection.id, nextMonitored)
                  .finally(() => setSeasonToggling(false));
              }}
            >
              {seasonCheckedState === false ? (
                <EyeOff className="size-5" />
              ) : (
                <Eye className="size-5" />
              )}
            </button>
            <Chevron className="h-4 w-4 shrink-0 text-muted-foreground" />
            <div className="min-w-0">
              <p className="text-sm font-semibold text-foreground">
                {seasonHeading(collection, t)}
              </p>
              {episodeRangeLabel ? <p className="text-xs text-muted-foreground">{episodeRangeLabel}</p> : null}
            </div>
          </div>
          <div className="flex items-center gap-2">
            {!isSpecials && collectionSizeLabel ? (
              <span className="text-xs tabular-nums text-muted-foreground">
                {collectionSizeLabel}
              </span>
            ) : null}
            {!isSpecials && collectionEpisodeProgress ? (
              <EpisodeProgressBar
                progress={collectionEpisodeProgress}
                compact
                className="w-[6.75rem]"
              />
            ) : null}
            {onRunSeasonSearch ? (
              <TooltipProvider>
                <Tooltip>
                  <TooltipTrigger asChild>
                    <span>
                      <EpisodeTableActionButton
                        id={seriesOverviewSeasonSearchId(collection.id)}
                        tone="auto"
                        aria-label={t("series.searchSeason")}
                        showTitleAttribute={false}
                        disabled={seasonSearchLoading === true}
                        onClick={(event) => {
                          event.stopPropagation();
                          onRunSeasonSearch();
                        }}
                        label={t("series.searchSeason")}
                      >
                        {seasonSearchLoading === true ? (
                          <Loader2 className="h-4 w-4 animate-spin" />
                        ) : (
                          <Zap className="h-4 w-4" />
                        )}
                      </EpisodeTableActionButton>
                    </span>
                  </TooltipTrigger>
                  <TooltipContent side="left" sideOffset={8} className="w-auto text-left">
                    {t("help.seasonSearchTooltip")}
                  </TooltipContent>
                </Tooltip>
              </TooltipProvider>
            ) : null}
          </div>
        </div>
      ) : null}

      {searchBlocked && onRunSeasonSearch && showCollectionHeader ? (
        <div className="border-t border-border bg-card/40 p-4">
          <TitleSearchDownloadClientNotice />
        </div>
      ) : null}

      {showSectionContent ? (
        <>
            {seasonSearchResults && seasonSearchResults.length > 0 && onQueueFromSeasonSearch ? (
              <div className={cn(showCollectionHeader && "border-t border-border", "px-4 py-3")}>
                <p className="mb-2 text-xs font-medium text-muted-foreground">{t("seasonSection.seasonPackResults")}</p>
                <SearchResultBuckets
                  results={seasonSearchResults}
                  onQueue={(release) => onQueueFromSeasonSearch(collection, release)}
                  requireCandidateToken
                />
              </div>
            ) : null}
            {episodes.length === 0 ? (
              <div className={cn(showCollectionHeader && "border-t border-border", "px-4 py-3 text-sm text-muted-foreground")}>
                {t("seasonSection.noEpisodeRecords")}
              </div>
            ) : isMobile ? (
              <div className={cn(showCollectionHeader && "border-t border-border", "px-3 py-3")}>
                <div className="space-y-3">
                  {episodes.map((episode) => (
                    <EpisodeRow
                      key={episode.id}
                      autoSearching={autoSearchLoadingByEpisode[episode.id] === true}
                      collection={collection}
                      clearingReleaseBlocklistEntryId={clearingReleaseBlocklistEntryId}
                      episode={episode}
                      episodeFiles={mediaFilesByEpisode[episode.id] ?? EMPTY_EPISODE_FILES}
                      episodeResults={searchResultsByEpisode[episode.id] ?? EMPTY_RELEASES}
                      facet={facet}
                      hasSearchResults={Object.prototype.hasOwnProperty.call(searchResultsByEpisode, episode.id)}
                      initiallyOpen={episode.id === initiallyOpenEpisodeId}
                      isMobile={true}
                      onAutoSearchEpisode={onAutoSearchEpisode}
                      onClearReleaseBlocklistEntry={onClearReleaseBlocklistEntry}
                      onDeleteFile={onDeleteFile}
                      onOpenHistory={onOpenEpisodeHistory}
                      onQueueFromEpisodeSearch={onQueueFromEpisodeSearch}
                      onQueueAdditionalFromEpisodeSearch={onQueueAdditionalFromEpisodeSearch}
                      onRefreshSubtitles={onRefreshSubtitles}
                      onRunEpisodeSearch={onRunEpisodeSearch}
                      onSetEpisodeMonitored={onSetEpisodeMonitored}
                      queueItem={downloadQueueItemByEpisodeId?.[episode.id]}
                      releaseBlocklistEntries={releaseBlocklistEntries}
                      searchBlocked={searchBlockedByEpisode[episode.id] === true}
                      searchLoading={searchLoadingByEpisode[episode.id] === true}
                      subtitleDownloads={stableSubtitleDownloads}
                    />
                  ))}
                </div>
              </div>
            ) : (
              <div className={cn(showCollectionHeader && "border-t border-border", "overflow-x-auto")}>
                <Table className="min-w-[640px]">
                  <TableHeader>
                    <TableRow>
                      <TableHead className="w-10 text-center" />
                      <TableHead className="w-12 text-center">{t("episode.numberLabel")}</TableHead>
                      <TableHead>{t("label.title")}</TableHead>
                      <TableHead className="w-32">{t("episode.airDate")}</TableHead>
                      <TableHead className="w-32 text-center">{t("episode.quality")}</TableHead>
                      <TableHead className="w-24 text-right">{t("label.actions")}</TableHead>
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {episodes.map((episode) => (
                      <EpisodeRow
                        key={episode.id}
                        autoSearching={autoSearchLoadingByEpisode[episode.id] === true}
                        collection={collection}
                        clearingReleaseBlocklistEntryId={clearingReleaseBlocklistEntryId}
                        episode={episode}
                        episodeFiles={mediaFilesByEpisode[episode.id] ?? EMPTY_EPISODE_FILES}
                        episodeResults={searchResultsByEpisode[episode.id] ?? EMPTY_RELEASES}
                        facet={facet}
                        hasSearchResults={Object.prototype.hasOwnProperty.call(searchResultsByEpisode, episode.id)}
                        initiallyOpen={episode.id === initiallyOpenEpisodeId}
                        isMobile={false}
                        onAutoSearchEpisode={onAutoSearchEpisode}
                        onClearReleaseBlocklistEntry={onClearReleaseBlocklistEntry}
                        onDeleteFile={onDeleteFile}
                        onOpenHistory={onOpenEpisodeHistory}
                        onQueueFromEpisodeSearch={onQueueFromEpisodeSearch}
                        onQueueAdditionalFromEpisodeSearch={onQueueAdditionalFromEpisodeSearch}
                        onRefreshSubtitles={onRefreshSubtitles}
                        onRunEpisodeSearch={onRunEpisodeSearch}
                        onSetEpisodeMonitored={onSetEpisodeMonitored}
                        queueItem={downloadQueueItemByEpisodeId?.[episode.id]}
                        releaseBlocklistEntries={releaseBlocklistEntries}
                        searchBlocked={searchBlockedByEpisode[episode.id] === true}
                        searchLoading={searchLoadingByEpisode[episode.id] === true}
                        subtitleDownloads={stableSubtitleDownloads}
                      />
                    ))}
                  </TableBody>
                </Table>
              </div>
            )}
          </>
      ) : null}
    </div>
  );
}
