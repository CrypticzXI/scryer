import * as React from "react";
import {
  CalendarDays,
  ChevronDown,
  ChevronRight,
  Clock3,
  Eye,
  EyeOff,
  Loader2,
  Search,
  Zap,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { SearchResultBuckets } from "@/components/common/release-search-results";
import { TitleSearchDownloadClientNotice } from "@/components/common/title-search-download-client-notice";
import { EpisodeQueueIndicator } from "@/components/common/download-queue-overview";
import { useTranslate } from "@/lib/context/translate-context";
import type { Release } from "@/lib/types";
import { cn } from "@/lib/utils";
import { releaseSupportsAdditionalFileQueue } from "@/lib/utils/release-queue-scope";
import { useIsMobile } from "@/lib/hooks/use-mobile";
import {
  boxedActionButtonBaseClass,
  boxedActionButtonToneClass,
  type BoxedActionButtonTone,
} from "@/lib/utils/action-button-styles";
import {
  selectorId,
  seriesOverviewSeasonMonitorId,
  seriesOverviewSeasonSectionId,
  seriesOverviewSeasonSearchId,
  seriesOverviewEpisodeAutoSearchId,
  seriesOverviewEpisodeInteractiveSearchId,
  seriesOverviewEpisodeRowId,
  seriesOverviewSeriesMovieAutoSearchId,
  seriesOverviewSeriesMovieInteractiveSearchId,
  seriesOverviewSeriesMovieRowId,
} from "@/lib/utils/dom-ids";
import {
  EpisodeProgressBar,
  getCollectionEpisodeProgressPresentation,
} from "@/components/views/media-content/title-table-shared";
import type {
  CollectionEpisode,
  EpisodeMediaFile,
  SeriesMovieLink,
  TitleCollection,
  TitleReleaseBlocklistEntry,
} from "@/components/containers/series-overview-container";
import type { EpisodePanelTab } from "./episode-panel-reducer";
import type { ExternalSubtitleRecord } from "@/lib/types/subtitles";
import type { DownloadQueueItem } from "@/lib/types/download-queue";
import {
  blocklistEntryMatchesEpisode,
  deriveMediaFileQualityLabel,
  formatDate,
  formatFileSize,
  formatRuntimeFromSeconds,
  isEpisodeCountableForProgress,
  isSpecialsCollection,
  seasonHeading,
} from "./helpers";
import { EpisodeDetailsPanel } from "./episode-details-panel";
import { SeriesMoviePanel } from "./series-movie-panel";
import { EpisodeBlocklistPanel } from "./episode-blocklist-panel";

type TranslateFn = ReturnType<typeof useTranslate>;

const EMPTY_EPISODE_FILES: EpisodeMediaFile[] = [];
const EMPTY_RELEASES: Release[] = [];
const EMPTY_SUBTITLE_DOWNLOADS: ExternalSubtitleRecord[] = [];
const EMPTY_BLOCKLIST_ENTRIES: TitleReleaseBlocklistEntry[] = [];

function EpisodeTableActionButton({
  label,
  tone,
  showTitleAttribute = true,
  className,
  children,
  ...props
}: React.ComponentProps<typeof Button> & {
  label: string;
  tone: Extract<BoxedActionButtonTone, "auto" | "search">;
  showTitleAttribute?: boolean;
}) {
  return (
    <Button
      type="button"
      size="icon-sm"
      variant="secondary"
      title={showTitleAttribute ? label : undefined}
      aria-label={label}
      className={cn(
        boxedActionButtonBaseClass,
        boxedActionButtonToneClass[tone],
        className,
      )}
      {...props}
    >
      {children}
    </Button>
  );
}

function renderEpisodeTypeBadges(episode: CollectionEpisode, t: TranslateFn) {
  return (
    <>
      {episode.episodeType === "special" ? (
        <span className="rounded border border-indigo-500/30 bg-indigo-500/15 px-1.5 py-0.5 text-[10px] font-medium text-indigo-700 dark:text-indigo-300">
          {t("episode.special")}
        </span>
      ) : episode.episodeType === "ova" ? (
        <span className="rounded border border-violet-500/30 bg-violet-500/15 px-1.5 py-0.5 text-[10px] font-medium text-violet-700 dark:text-violet-300">
          {t("episode.ova")}
        </span>
      ) : episode.episodeType === "ona" ? (
        <span className="rounded border border-emerald-500/30 bg-emerald-500/15 px-1.5 py-0.5 text-[10px] font-medium text-emerald-700 dark:text-emerald-300">
          {t("episode.ona")}
        </span>
      ) : episode.episodeType === "alternate" ? (
        <span className="rounded border border-sky-500/30 bg-sky-500/15 px-1.5 py-0.5 text-[10px] font-medium text-sky-700 dark:text-sky-300">
          {t("episode.alternate")}
        </span>
      ) : null}
      {episode.isFiller ? (
        <span className="rounded border border-orange-500/30 bg-orange-500/15 px-1.5 py-0.5 text-[10px] font-medium text-orange-700 dark:text-orange-300">
          {t("episode.filler")}
        </span>
      ) : null}
      {episode.isRecap ? (
        <span className="rounded border border-amber-500/30 bg-amber-500/15 px-1.5 py-0.5 text-[10px] font-medium text-amber-700 dark:text-amber-300">
          {t("episode.recap")}
        </span>
      ) : null}
      {episode.hasMultiAudio ? (
        <span className="rounded border border-purple-500/30 bg-purple-500/15 px-1.5 py-0.5 text-[10px] font-medium text-purple-700 dark:text-purple-300">
          {t("episode.multiAudio")}
        </span>
      ) : null}
    </>
  );
}

function renderEpisodeQualityBadge(
  episode: CollectionEpisode,
  episodeFiles: EpisodeMediaFile[],
  t: TranslateFn,
) {
  const primaryFile = episodeFiles[0];

  if (primaryFile) {
    const qualityLabel = deriveMediaFileQualityLabel(primaryFile);
    if (qualityLabel) {
      return (
        <span className="rounded border border-emerald-500/40 bg-emerald-500/20 px-1.5 py-0.5 text-[10px] font-medium text-emerald-700 dark:border-emerald-500/30 dark:bg-emerald-500/15 dark:text-emerald-300">
          {qualityLabel}
        </span>
      );
    }

    if (primaryFile.scanStatus === "imported") {
      return (
        <span className="rounded border border-amber-500/30 bg-amber-500/15 px-1.5 py-0.5 text-[10px] font-medium text-amber-300">
          {t("mediaFile.pendingScan")}
        </span>
      );
    }

    if (primaryFile.scanStatus === "scan_failed") {
      return (
        <span className="rounded border border-red-500/30 bg-red-500/15 px-1.5 py-0.5 text-[10px] font-medium text-red-300">
          {t("mediaFile.scanFailed")}
        </span>
      );
    }

    return (
      <span className="rounded border border-emerald-500/40 bg-emerald-500/20 px-1.5 py-0.5 text-[10px] font-medium text-emerald-700 dark:border-emerald-500/30 dark:bg-emerald-500/15 dark:text-emerald-300">
        {t("episode.fileOnDisk")}
      </span>
    );
  }

  if (episode.monitored) {
    return (
      <span className="rounded border border-amber-500/30 bg-amber-500/15 px-1.5 py-0.5 text-[10px] font-medium text-amber-300">
        {t("episode.missing")}
      </span>
    );
  }

  return null;
}

function renderEpisodeQualityCell(
  episode: CollectionEpisode,
  episodeFiles: EpisodeMediaFile[],
  queueItem: DownloadQueueItem | undefined,
  t: TranslateFn,
) {
  const qualityBadge = renderEpisodeQualityBadge(episode, episodeFiles, t);

  if (!qualityBadge && !queueItem) {
    return null;
  }

  return (
    <div className="flex flex-col items-center gap-1">
      {qualityBadge}
      {queueItem ? <EpisodeQueueIndicator item={queueItem} /> : null}
    </div>
  );
}

type EpisodePanelContentProps = {
  activeTab: EpisodePanelTab;
  canClearBlocklistEntries: boolean;
  collection: TitleCollection;
  clearingReleaseBlocklistEntryId?: string | null;
  episode: CollectionEpisode;
  episodeFiles: EpisodeMediaFile[];
  episodeLoading: boolean;
  episodeResults: Release[];
  facet: string;
  onClearReleaseBlocklistEntry?: (entryId: string) => Promise<void> | void;
  onDeleteFile?: (fileId: string) => void;
  onQueueFromEpisodeSearch?: (episode: CollectionEpisode, release: Release) => Promise<void> | void;
  onQueueAdditionalFromEpisodeSearch?: (episode: CollectionEpisode, release: Release) => Promise<void> | void;
  onRefreshSubtitles?: () => Promise<void> | void;
  onRunEpisodeSearch?: (episode: CollectionEpisode) => void;
  onTabChange: (tab: EpisodePanelTab | "history") => void;
  showHistoryTab: boolean;
  showSearchTab: boolean;
  releaseBlocklistEntries: TitleReleaseBlocklistEntry[];
  searchBlocked: boolean;
  subtitleDownloads: ExternalSubtitleRecord[];
};

const EpisodePanelContent = React.memo(function EpisodePanelContent({
  activeTab,
  canClearBlocklistEntries,
  collection,
  clearingReleaseBlocklistEntryId,
  episode,
  episodeFiles,
  episodeLoading,
  episodeResults,
  facet,
  onClearReleaseBlocklistEntry,
  onDeleteFile,
  onQueueFromEpisodeSearch,
  onQueueAdditionalFromEpisodeSearch,
  onRefreshSubtitles,
  onRunEpisodeSearch,
  onTabChange,
  showHistoryTab,
  showSearchTab,
  releaseBlocklistEntries,
  searchBlocked,
  subtitleDownloads,
}: EpisodePanelContentProps) {
  const t = useTranslate();
  const filteredBlocklistEntries = React.useMemo(() => {
    if (activeTab !== "blocklist") {
      return EMPTY_BLOCKLIST_ENTRIES;
    }

    return releaseBlocklistEntries.filter((entry) =>
      blocklistEntryMatchesEpisode(entry, episode, collection),
    );
  }, [activeTab, collection, episode, releaseBlocklistEntries]);

  return (
    <Tabs
      value={activeTab}
      onValueChange={(value) => onTabChange(value as EpisodePanelTab | "history")}
    >
      <TabsList className="flex w-full flex-nowrap overflow-x-auto">
        <TabsTrigger
          id={selectorId("series-overview-episode-tab", episode.id, "details")}
          value="details"
          className="shrink-0"
        >{t("episode.details")}</TabsTrigger>
        {showSearchTab ? (
          <TabsTrigger
            id={selectorId("series-overview-episode-tab", episode.id, "search")}
            value="search"
            className="shrink-0"
          >{t("episode.search")}</TabsTrigger>
        ) : null}
        {showHistoryTab ? (
          <TabsTrigger
            id={selectorId("series-overview-episode-tab", episode.id, "history")}
            value="history"
            className="shrink-0"
          >{t("history.title")}</TabsTrigger>
        ) : null}
        <TabsTrigger
          id={selectorId("series-overview-episode-tab", episode.id, "blocklist")}
          value="blocklist"
          className="shrink-0"
        >{t("episode.blocklist")}</TabsTrigger>
      </TabsList>
      <TabsContent value="details">
        <EpisodeDetailsPanel
          episode={episode}
          mediaFiles={episodeFiles}
          subtitleDownloads={subtitleDownloads}
          onRefreshSubtitles={onRefreshSubtitles}
          onDeleteFile={onDeleteFile}
        />
      </TabsContent>
      {showSearchTab ? (
        <TabsContent value="search">
        {searchBlocked ? (
          <TitleSearchDownloadClientNotice />
        ) : (
          <div className="mb-2 flex items-center justify-end">
            <Button
              id={selectorId("series-overview-episode-search-refresh", episode.id)}
              type="button"
              variant="ghost"
              size="sm"
              onClick={() => onRunEpisodeSearch?.(episode)}
              disabled={episodeLoading}
              aria-label={t("label.search")}
            >
              <Search className="h-4 w-4" />
              <span className="ml-1">
                {episodeLoading ? t("label.searching") : t("label.refresh")}
              </span>
            </Button>
          </div>
        )}
        {searchBlocked ? null : episodeLoading ? (
          <div className="flex flex-col items-center justify-center gap-4 py-16">
            <Loader2 className="h-10 w-10 animate-spin text-emerald-500" />
            <p className="text-lg text-muted-foreground">{t("label.searching")}</p>
          </div>
        ) : episodeResults.length === 0 ? (
          <p className="text-sm text-muted-foreground">{t("nzb.noResultsYet")}</p>
        ) : (
          <SearchResultBuckets
            results={episodeResults}
            onQueue={(release) => onQueueFromEpisodeSearch?.(episode, release)}
            onQueueAdditional={(release) => onQueueAdditionalFromEpisodeSearch?.(episode, release)}
            canQueueAdditional={(release) =>
              releaseSupportsAdditionalFileQueue(release, facet)
            }
            requireCandidateToken
          />
        )}
        </TabsContent>
      ) : null}
      <TabsContent value="blocklist">
        <EpisodeBlocklistPanel
          entries={filteredBlocklistEntries}
          canClear={canClearBlocklistEntries}
          clearingEntryId={clearingReleaseBlocklistEntryId}
          onClear={onClearReleaseBlocklistEntry}
        />
      </TabsContent>
    </Tabs>
  );
});

type EpisodeRowProps = {
  autoSearching: boolean;
  collection: TitleCollection;
  clearingReleaseBlocklistEntryId?: string | null;
  episode: CollectionEpisode;
  episodeFiles: EpisodeMediaFile[];
  episodeResults: Release[];
  facet: string;
  hasSearchResults: boolean;
  initiallyOpen: boolean;
  isMobile: boolean;
  onAutoSearchEpisode?: (episode: CollectionEpisode) => void;
  onClearReleaseBlocklistEntry?: (entryId: string) => Promise<void> | void;
  onDeleteFile?: (fileId: string) => void;
  onOpenHistory?: (episode: CollectionEpisode) => void;
  onQueueFromEpisodeSearch?: (episode: CollectionEpisode, release: Release) => Promise<void> | void;
  onQueueAdditionalFromEpisodeSearch?: (episode: CollectionEpisode, release: Release) => Promise<void> | void;
  onRefreshSubtitles?: () => Promise<void> | void;
  onRunEpisodeSearch?: (episode: CollectionEpisode) => void;
  onSetEpisodeMonitored?: (episodeId: string, monitored: boolean) => Promise<void>;
  queueItem?: DownloadQueueItem;
  releaseBlocklistEntries: TitleReleaseBlocklistEntry[];
  searchBlocked: boolean;
  searchLoading: boolean;
  subtitleDownloads: ExternalSubtitleRecord[];
};

const EpisodeRow = React.memo(function EpisodeRow({
  autoSearching,
  collection,
  clearingReleaseBlocklistEntryId,
  episode,
  episodeFiles,
  episodeResults,
  facet,
  hasSearchResults,
  initiallyOpen,
  isMobile,
  onAutoSearchEpisode,
  onClearReleaseBlocklistEntry,
  onDeleteFile,
  onOpenHistory,
  onQueueFromEpisodeSearch,
  onQueueAdditionalFromEpisodeSearch,
  onRefreshSubtitles,
  onRunEpisodeSearch,
  onSetEpisodeMonitored,
  queueItem,
  releaseBlocklistEntries,
  searchBlocked,
  searchLoading,
  subtitleDownloads,
}: EpisodeRowProps) {
  const t = useTranslate();
  const [isPanelOpen, setIsPanelOpen] = React.useState(initiallyOpen);
  const [activeTab, setActiveTab] = React.useState<EpisodePanelTab>("details");
  const [episodeToggling, setEpisodeToggling] = React.useState(false);

  React.useEffect(() => {
    if (initiallyOpen) {
      setIsPanelOpen(true);
      setActiveTab("details");
    }
  }, [initiallyOpen]);

  const formattedAirDate = React.useMemo(
    () => formatDate(episode.airDate),
    [episode.airDate],
  );
  const episodeRuntime = React.useMemo(
    () => formatRuntimeFromSeconds(episode.durationSeconds),
    [episode.durationSeconds],
  );

  const openPanelTab = React.useCallback(
    (tab: EpisodePanelTab) => {
      setActiveTab(tab);
      setIsPanelOpen(true);
      if (tab === "search" && onRunEpisodeSearch && (searchBlocked || !hasSearchResults)) {
        onRunEpisodeSearch(episode);
      }
    },
    [episode, hasSearchResults, onRunEpisodeSearch, searchBlocked],
  );

  const handleToggleEpisodeDetails = React.useCallback(() => {
    if (isPanelOpen && activeTab === "details") {
      setIsPanelOpen(false);
      return;
    }

    openPanelTab("details");
  }, [activeTab, isPanelOpen, openPanelTab]);

  const handleToggleEpisodeSearch = React.useCallback(() => {
    if (!onRunEpisodeSearch || !onQueueFromEpisodeSearch) {
      return;
    }
    if (isPanelOpen && activeTab === "search") {
      setIsPanelOpen(false);
      return;
    }

    openPanelTab("search");
  }, [activeTab, isPanelOpen, onQueueFromEpisodeSearch, onRunEpisodeSearch, openPanelTab]);

  const handleEpisodeTabChange = React.useCallback(
    (tab: EpisodePanelTab | "history") => {
      if (tab === "history") {
        if (!onOpenHistory) {
          return;
        }
        setIsPanelOpen(true);
        onOpenHistory(episode);
        return;
      }

      openPanelTab(tab);
    },
    [episode, onOpenHistory, openPanelTab],
  );

  const handleAutoSearchClick = React.useCallback(() => {
    onAutoSearchEpisode?.(episode);
  }, [episode, onAutoSearchEpisode]);

  const handleToggleEpisodeMonitored = React.useCallback(() => {
    if (!onSetEpisodeMonitored) {
      return;
    }

    setEpisodeToggling(true);
    onSetEpisodeMonitored(episode.id, !episode.monitored)
      .finally(() => setEpisodeToggling(false));
  }, [episode.id, episode.monitored, onSetEpisodeMonitored]);

  const panelContent = isPanelOpen ? (
    <EpisodePanelContent
      activeTab={activeTab}
      canClearBlocklistEntries={Boolean(onClearReleaseBlocklistEntry)}
      collection={collection}
      clearingReleaseBlocklistEntryId={clearingReleaseBlocklistEntryId}
      episode={episode}
      episodeFiles={episodeFiles}
      episodeLoading={searchLoading}
      episodeResults={episodeResults}
      facet={facet}
      onClearReleaseBlocklistEntry={onClearReleaseBlocklistEntry}
      onDeleteFile={onDeleteFile}
      onQueueFromEpisodeSearch={onQueueFromEpisodeSearch}
      onQueueAdditionalFromEpisodeSearch={onQueueAdditionalFromEpisodeSearch}
      onRefreshSubtitles={onRefreshSubtitles}
      onRunEpisodeSearch={onRunEpisodeSearch}
      onTabChange={handleEpisodeTabChange}
      showHistoryTab={Boolean(onOpenHistory)}
      showSearchTab={Boolean(onRunEpisodeSearch && onQueueFromEpisodeSearch)}
      releaseBlocklistEntries={releaseBlocklistEntries}
      searchBlocked={searchBlocked}
      subtitleDownloads={subtitleDownloads}
    />
  ) : null;

  const episodeTypeBadges = renderEpisodeTypeBadges(episode, t);
  const qualityCell = renderEpisodeQualityCell(episode, episodeFiles, queueItem, t);

  if (isMobile) {
    return (
      <div
        id={selectorId("series-overview-episode", episode.id)}
        data-episode-id={episode.id}
        className={cn(
          "rounded-lg border border-border bg-card/50 p-3",
          !episode.monitored && "opacity-60",
        )}
      >
        <div className="flex items-start gap-3">
          <button
            id={selectorId("series-overview-episode-monitor", episode.id)}
            type="button"
            disabled={!onSetEpisodeMonitored || episodeToggling}
            aria-label={t("title.episodeMonitored")}
            className={cn(
              "mt-0.5 inline-flex size-6 shrink-0 items-center justify-center rounded transition-colors",
              episodeToggling && "opacity-50",
              episode.monitored
                ? "text-emerald-600 dark:text-emerald-300"
                : "text-muted-foreground/60",
            )}
            onClick={handleToggleEpisodeMonitored}
          >
            {episode.monitored ? (
              <Eye className="size-5" />
            ) : (
              <EyeOff className="size-5" />
            )}
          </button>
          <div className="min-w-0 flex-1">
            <div className="flex items-start justify-between gap-3">
              <button
                id={selectorId("series-overview-episode-details-toggle", episode.id)}
                type="button"
                className="min-w-0 flex-1 text-left"
                onClick={handleToggleEpisodeDetails}
              >
                <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
                  <span className="rounded bg-accent px-2 py-0.5 font-mono text-card-foreground">
                    {episode.episodeNumber ?? episode.episodeLabel ?? "—"}
                  </span>
                  {episode.absoluteNumber && facet === "anime" ? (
                    <span>#{episode.absoluteNumber}</span>
                  ) : null}
                </div>
                <p className="mt-1 text-sm font-medium text-card-foreground">
                  {episode.title || episode.episodeLabel || "—"}
                </p>
              </button>
              <div className="flex shrink-0 items-center gap-1">
                {qualityCell}
              </div>
            </div>
            <div className="mt-2 flex flex-wrap gap-2">
              {episodeTypeBadges}
            </div>
            <div className="mt-2 flex flex-wrap items-center gap-3 text-xs text-muted-foreground">
              <span className="inline-flex items-center gap-1">
                <CalendarDays className="h-3.5 w-3.5" />
                {formattedAirDate}
              </span>
              {episodeRuntime ? (
                <span className="inline-flex items-center gap-1">
                  <Clock3 className="h-3 w-3" />
                  {episodeRuntime}
                </span>
              ) : null}
            </div>
            <div className="mt-3 flex flex-col gap-2">
              {onAutoSearchEpisode ? (
                <Button
                  id={selectorId("series-overview-episode-auto-search", episode.id)}
                  type="button"
                  size="sm"
                  variant="secondary"
                  className={cn("w-full", boxedActionButtonToneClass.auto)}
                  onClick={handleAutoSearchClick}
                  disabled={autoSearching}
                >
                  {autoSearching ? (
                    <Loader2 className="h-4 w-4 animate-spin" />
                  ) : (
                    <Zap className="h-4 w-4" />
                  )}
                  <span>{t("label.search")}</span>
                </Button>
              ) : null}
              {onRunEpisodeSearch && onQueueFromEpisodeSearch ? (
                <Button
                  id={selectorId("series-overview-episode-interactive-search", episode.id)}
                  type="button"
                  size="sm"
                  variant="primary"
                  className="w-full border border-sky-500/70 bg-sky-600 text-white hover:bg-sky-500 focus-visible:ring-sky-300/70 dark:border-sky-400/50 dark:bg-sky-500 dark:hover:bg-sky-400"
                  onClick={handleToggleEpisodeSearch}
                >
                  <Search className="h-4 w-4" />
                  <span>{t("label.interactiveSearch")}</span>
                </Button>
              ) : null}
            </div>
            {isPanelOpen ? (
              <div className="mt-3 border-t border-border pt-3">
                {panelContent}
              </div>
            ) : null}
          </div>
        </div>
      </div>
    );
  }

  return (
    <React.Fragment>
      <TableRow
        id={seriesOverviewEpisodeRowId(
          facet,
          episode.seasonNumber,
          episode.episodeNumber,
          episode.absoluteNumber,
        )}
        data-episode-id={episode.id}
        className={`cv-auto-row-sm${episode.monitored ? "" : " opacity-50"}`}
      >
        <TableCell className="pl-2 pr-0 text-right align-middle">
          <div className="flex items-center justify-end">
            <button
              id={selectorId("series-overview-episode-monitor", episode.id)}
              type="button"
              disabled={episodeToggling}
              aria-label={t("title.episodeMonitored")}
              className={cn(
                "inline-flex size-6 items-center justify-center rounded transition-colors",
                episodeToggling && "opacity-50",
                episode.monitored
                  ? "text-emerald-600 dark:text-emerald-300"
                  : "text-muted-foreground/60",
              )}
              onClick={handleToggleEpisodeMonitored}
            >
              {episode.monitored ? (
                <Eye className="size-5" />
              ) : (
                <EyeOff className="size-5" />
              )}
            </button>
          </div>
        </TableCell>
        <TableCell className="text-center align-middle font-mono text-sm text-card-foreground">
          <div className="flex flex-col items-center gap-0.5">
            <span>{episode.episodeNumber ?? episode.episodeLabel ?? "—"}</span>
            {episode.absoluteNumber && facet === "anime" ? (
              <span className="text-[10px] text-muted-foreground">
                #{episode.absoluteNumber}
              </span>
            ) : null}
          </div>
        </TableCell>
        <TableCell
          id={selectorId("series-overview-episode-details-toggle", episode.id)}
          className="cursor-pointer align-middle text-sm text-card-foreground hover:text-foreground"
          onClick={handleToggleEpisodeDetails}
        >
          <div className="flex items-center gap-1.5">
            <span>{episode.title || episode.episodeLabel || "—"}</span>
            {episodeTypeBadges}
          </div>
          {episodeRuntime ? (
            <span className="inline-flex items-center gap-1 text-[10px] text-muted-foreground">
              <Clock3 className="h-3 w-3" />
              {episodeRuntime}
            </span>
          ) : null}
        </TableCell>
        <TableCell className="text-muted-foreground">
          <span className="inline-flex items-center gap-1">
            <CalendarDays className="h-3.5 w-3.5" />
            {formattedAirDate}
          </span>
        </TableCell>
        <TableCell className="text-center">
          <div className="inline-flex items-center justify-center gap-1">
            {qualityCell}
          </div>
        </TableCell>
        <TableCell className="text-right">
          <TooltipProvider>
            <div className="inline-flex items-center justify-end gap-2">
              {onAutoSearchEpisode ? (
                <Tooltip>
                  <TooltipTrigger asChild>
                    <span>
                      <EpisodeTableActionButton
                        id={seriesOverviewEpisodeAutoSearchId(
                          facet,
                          episode.seasonNumber,
                          episode.episodeNumber,
                          episode.absoluteNumber,
                        )}
                        tone="auto"
                        onClick={handleAutoSearchClick}
                        disabled={autoSearching}
                        label={t("label.search")}
                        showTitleAttribute={false}
                      >
                        {autoSearching ? (
                          <Loader2 className="h-4 w-4 animate-spin" />
                        ) : (
                          <Zap className="h-4 w-4" />
                        )}
                      </EpisodeTableActionButton>
                    </span>
                  </TooltipTrigger>
                  <TooltipContent side="top" sideOffset={8} className="max-w-[18rem] whitespace-normal break-words text-left text-sm leading-snug">
                    {t("help.autoSearchTooltip")}
                  </TooltipContent>
                </Tooltip>
              ) : null}
              {onRunEpisodeSearch && onQueueFromEpisodeSearch ? (
                <Tooltip>
                  <TooltipTrigger asChild>
                    <span>
                      <EpisodeTableActionButton
                        id={seriesOverviewEpisodeInteractiveSearchId(
                          facet,
                          episode.seasonNumber,
                          episode.episodeNumber,
                          episode.absoluteNumber,
                        )}
                        tone="search"
                        onClick={handleToggleEpisodeSearch}
                        label={t("label.search")}
                        showTitleAttribute={false}
                      >
                        <Search className="h-4 w-4" />
                      </EpisodeTableActionButton>
                    </span>
                  </TooltipTrigger>
                  <TooltipContent side="top" sideOffset={8} className="max-w-[18rem] whitespace-normal break-words text-left text-sm leading-snug">
                    {t("help.interactiveSearchTooltip")}
                  </TooltipContent>
                </Tooltip>
              ) : null}
            </div>
          </TooltipProvider>
        </TableCell>
      </TableRow>
      {isPanelOpen ? (
        <TableRow id={selectorId("series-overview-episode-panel", episode.id)}>
          <TableCell colSpan={6} className="border-t border-border bg-background/40 p-0">
            <div className="px-4 py-3">
              {panelContent}
            </div>
          </TableCell>
        </TableRow>
      ) : null}
    </React.Fragment>
  );
});

type SeriesMovieTimelineContentProps = {
  link: SeriesMovieLink;
  mediaFilesByEpisode: Record<string, EpisodeMediaFile[]>;
  mediaFilesBySeriesMovieLink: Record<string, EpisodeMediaFile[]>;
  seriesMovieSearchResultsByLink: Record<string, Release[]>;
  seriesMovieSearchLoadingByLink: Record<string, boolean>;
  seriesMovieSearchAttemptedByLink: Record<string, boolean>;
  searchBlockedBySeriesMovie: Record<string, boolean>;
  onRunSeriesMovieSearch?: (link: SeriesMovieLink) => void;
  onQueueFromSeriesMovieSearch?: (link: SeriesMovieLink, release: Release) => Promise<void> | void;
  onAutoSearchSeriesMovie?: (link: SeriesMovieLink) => void;
  autoSearchSeriesMovieLoadingByLink: Record<string, boolean>;
};

function SeriesMovieTimelineContent({
  link,
  mediaFilesByEpisode,
  mediaFilesBySeriesMovieLink,
  seriesMovieSearchResultsByLink,
  seriesMovieSearchLoadingByLink,
  seriesMovieSearchAttemptedByLink,
  searchBlockedBySeriesMovie,
  onRunSeriesMovieSearch,
  onQueueFromSeriesMovieSearch,
  onAutoSearchSeriesMovie,
  autoSearchSeriesMovieLoadingByLink,
}: SeriesMovieTimelineContentProps) {
  const t = useTranslate();
  const searchBlockedForMovie = searchBlockedBySeriesMovie[link.id] === true;
  const searchLoading = seriesMovieSearchLoadingByLink[link.id] === true;
  const searchAttempted = seriesMovieSearchAttemptedByLink[link.id] === true;
  const searchResults = seriesMovieSearchResultsByLink[link.id];
  const autoSearchLoading = autoSearchSeriesMovieLoadingByLink[link.id] === true;
  const linkedEpisodeFiles = link.linkedEpisodeId
    ? mediaFilesByEpisode[link.linkedEpisodeId] ?? EMPTY_EPISODE_FILES
    : EMPTY_EPISODE_FILES;
  const seriesMovieFiles =
    mediaFilesBySeriesMovieLink[link.id] ?? EMPTY_EPISODE_FILES;

  return (
    <div className="space-y-3">
      <SeriesMoviePanel
        link={link}
        hasFile={seriesMovieFiles.length > 0 || linkedEpisodeFiles.length > 0}
      />
      <div className="flex flex-wrap items-center gap-2">
        {onRunSeriesMovieSearch ? (
          <button
            id={seriesOverviewSeriesMovieInteractiveSearchId(link.id)}
            type="button"
            disabled={searchLoading}
            onClick={() => onRunSeriesMovieSearch(link)}
            className="inline-flex items-center gap-1.5 self-start rounded-md border border-border bg-card/45 px-3 py-1.5 text-xs text-card-foreground transition hover:bg-muted disabled:opacity-50"
          >
            <Search className="h-3.5 w-3.5" />
            {t("title.searchReleasesAction")}
          </button>
        ) : null}
        {onAutoSearchSeriesMovie ? (
          <button
            id={seriesOverviewSeriesMovieAutoSearchId(link.id)}
            type="button"
            disabled={autoSearchLoading}
            onClick={() => onAutoSearchSeriesMovie(link)}
            className="inline-flex items-center gap-1.5 self-start rounded-md border border-border bg-card/45 px-3 py-1.5 text-xs text-card-foreground transition hover:bg-muted disabled:opacity-50"
          >
            {autoSearchLoading ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
            ) : (
              <Zap className="h-3.5 w-3.5" />
            )}
            {t("title.queueLatest")}
          </button>
        ) : null}
      </div>
      {searchBlockedForMovie ? <TitleSearchDownloadClientNotice /> : null}
      {!searchBlockedForMovie && searchLoading ? (
        <div className="flex items-center gap-2 text-xs text-muted-foreground">
          <Loader2 className="h-4 w-4 animate-spin" />
          <span>{t("title.searchingReleases")}</span>
        </div>
      ) : null}
      {!searchBlockedForMovie
      && !searchLoading
      && searchResults
      && searchResults.length > 0
      && onQueueFromSeriesMovieSearch ? (
        <div className="space-y-2">
          <p className="text-xs font-medium text-muted-foreground">
            {t("title.searchReleasesAction")}
          </p>
          <SearchResultBuckets
            results={searchResults}
            onQueue={(release) => onQueueFromSeriesMovieSearch(link, release)}
            requireCandidateToken
          />
        </div>
      ) : null}
      {!searchBlockedForMovie
      && !searchLoading
      && searchAttempted
      && (!searchResults || searchResults.length === 0) ? (
        <p className="text-xs text-muted-foreground">
          {t("title.noReleasesFound", { name: link.movie.title })}
        </p>
      ) : null}
      {!searchBlockedForMovie
      && !searchLoading
      && !searchAttempted ? (
        <p className="text-xs text-muted-foreground">
          {t("title.interactiveSearchHint", { name: link.movie.title })}
        </p>
      ) : null}
    </div>
  );
}

export function SeriesMovieTimelineSection(props: SeriesMovieTimelineContentProps) {
  return (
    <div
      id={seriesOverviewSeriesMovieRowId(props.link.id)}
      data-timeline-kind="series-movie"
      data-series-movie-link-id={props.link.id}
      className="overflow-hidden rounded-lg border border-border bg-background/40 p-3"
    >
      <SeriesMovieTimelineContent {...props} />
    </div>
  );
}

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
      const sizeBytes = Number(file.sizeBytes);
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
                <p className="mb-2 text-xs font-medium text-muted-foreground">Season pack results</p>
                <SearchResultBuckets
                  results={seasonSearchResults}
                  onQueue={(release) => onQueueFromSeasonSearch(collection, release)}
                  requireCandidateToken
                />
              </div>
            ) : null}
            {episodes.length === 0 ? (
              <div className={cn(showCollectionHeader && "border-t border-border", "px-4 py-3 text-sm text-muted-foreground")}>
                No episode records for this season.
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
                <Table className="min-w-[760px]">
                  <TableHeader>
                    <TableRow>
                      <TableHead className="w-10 text-center" />
                      <TableHead className="w-16 text-center">{t("episode.numberLabel")}</TableHead>
                      <TableHead>{t("label.title")}</TableHead>
                      <TableHead className="w-40">{t("episode.airDate")}</TableHead>
                      <TableHead className="w-40 text-center">{t("episode.quality")}</TableHead>
                      <TableHead className="w-28 text-right">{t("label.actions")}</TableHead>
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
