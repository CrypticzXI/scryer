import * as React from "react";
import { FileInput, FolderOpen, Loader2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Clapperboard } from "lucide-react";
import { useClient } from "urql";
import type { Release, TitleAcquisitionDiagnostics } from "@/lib/types";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { TitlePosterSlot } from "@/components/title-poster-slot";
import { searchForEpisodeQuery } from "@/lib/graphql/queries";
import { queueExistingMutation } from "@/lib/graphql/mutations";
import type {
  CollectionEpisode,
  EpisodeMediaFile,
  TitleCollection,
  TitleDetail,
  TitleHistoryEvent,
  TitleReleaseBlocklistEntry,
} from "@/components/containers/series-overview-container";
import type { DownloadQueueItem } from "@/lib/types/download-queue";
import { TitleHistoryModal } from "@/components/common/title-history-modal";
import { TitleSearchDownloadClientNotice } from "@/components/common/title-search-download-client-notice";
import {
  episodePanelReducer,
  initialEpisodePanelState,
  type EpisodePanelTab,
} from "./episode-panel-reducer";
import {
  sortDbCollections,
  findLatestSeasonKey,
  episodeSortValue,
  isSpecialsCollection,
  formatDate,
} from "./helpers";
import { OverviewControlPanel } from "../overview-control-panel";
import { OverviewBackLink } from "../overview-back-link";
import { TitleSettingsPanel } from "./title-settings-panel";
import { SeasonSection } from "./season-section";
import type { TitleOptionUpdates } from "@/lib/types/title-options";
import { localizedTitleStatus } from "../overview-localization";
import type { SubtitleDownloadRecord } from "@/lib/types/subtitles";

const imdbLogoUrl = `${import.meta.env.BASE_URL}media-sites/imdb.svg`;
const tvdbLogoUrl = `${import.meta.env.BASE_URL}media-sites/tvdb.svg`;
const tmdbLogoUrl = `${import.meta.env.BASE_URL}media-sites/tmdb.svg`;
const malLogoUrl = `${import.meta.env.BASE_URL}media-sites/mal.svg`;
const anilistLogoUrl = `${import.meta.env.BASE_URL}media-sites/anilist.svg`;
const anidbLogoUrl = `${import.meta.env.BASE_URL}media-sites/anidb.png`;

type Props = {
  loading: boolean;
  hydrating: boolean;
  title: TitleDetail | null;
  collections: TitleCollection[];
  events: TitleHistoryEvent[];
  episodesByCollection: Record<string, CollectionEpisode[]>;
  mediaFilesByEpisode: Record<string, EpisodeMediaFile[]>;
  subtitleDownloads?: SubtitleDownloadRecord[];
  onRefreshSubtitles?: () => Promise<void> | void;
  releaseBlocklistEntries: TitleReleaseBlocklistEntry[];
  onTitleChanged?: () => Promise<void>;
  onBackToList?: () => void;
  onSetCollectionMonitored?: (collectionId: string, monitored: boolean) => Promise<void>;
  onSetEpisodeMonitored?: (episodeId: string, monitored: boolean) => Promise<void>;
  onSetTitleMonitored?: (monitored: boolean) => Promise<void>;
  onSearchMonitored?: () => Promise<void> | void;
  onRefreshAndScan?: () => Promise<void> | void;
  onAutoSearchEpisode?: (episode: CollectionEpisode) => Promise<void> | void;
  onAutoSearchInterstitialMovie?: (collection: TitleCollection) => Promise<void> | void;
  acquisitionDiagnostics?: TitleAcquisitionDiagnostics | null;
  qualityProfiles?: { id: string; name: string }[];
  defaultRootFolder?: string;
  rootFolders?: { path: string; isDefault: boolean }[];
  onUpdateTitleOptions?: (options: TitleOptionUpdates) => Promise<void>;
  completedDownloads?: DownloadQueueItem[];
  onOpenManualImport?: (item: DownloadQueueItem) => void;
  initialEpisodeId?: string | null;
  seasonSearchResultsByCollection?: Record<string, Release[]>;
  seasonSearchLoadingByCollection?: Record<string, boolean>;
  onRunSeasonSearch?: (collection: TitleCollection) => Promise<void> | void;
  onQueueFromSeasonSearch?: (collection: TitleCollection, release: Release) => Promise<void> | void;
  onTriggerMismatchRecovery?: () => Promise<void> | void;
  monitoredUpdating?: boolean;
  searchMonitoredLoading?: boolean;
  hasDownloadClients: boolean;
  showSearchPrerequisiteNotice: boolean;
  refreshAndScanLoading?: boolean;
  onRequestDeleteTitle?: () => void;
  deleteLoading?: boolean;
  onDeleteFile?: (fileId: string) => void;
  onOpenFixMatch?: () => void;
};

export function SeriesOverviewView({
  loading,
  hydrating,
  title,
  collections,
  events: _events,
  episodesByCollection,
  mediaFilesByEpisode,
  subtitleDownloads,
  onRefreshSubtitles,
  releaseBlocklistEntries,
  onTitleChanged,
  onBackToList,
  onSetCollectionMonitored,
  onSetEpisodeMonitored,
  onSetTitleMonitored,
  onSearchMonitored,
  onRefreshAndScan,
  onAutoSearchEpisode,
  onAutoSearchInterstitialMovie,
  acquisitionDiagnostics,
  qualityProfiles,
  defaultRootFolder,
  rootFolders,
  onUpdateTitleOptions,
  completedDownloads,
  onOpenManualImport,
  initialEpisodeId,
  seasonSearchResultsByCollection,
  seasonSearchLoadingByCollection,
  onRunSeasonSearch,
  onQueueFromSeasonSearch,
  onTriggerMismatchRecovery,
  monitoredUpdating = false,
  searchMonitoredLoading = false,
  hasDownloadClients,
  showSearchPrerequisiteNotice,
  refreshAndScanLoading = false,
  onRequestDeleteTitle,
  deleteLoading = false,
  onDeleteFile,
  onOpenFixMatch,
}: Props) {
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const client = useClient();
  const backLabel = title?.facet === "anime" ? t("nav.anime") : t("nav.series");
  const sortedCollections = React.useMemo(
    () => sortDbCollections(collections),
    [collections],
  );

  const latestKey = React.useMemo(
    () => findLatestSeasonKey(sortedCollections),
    [sortedCollections],
  );

  const [expandedKeys, setExpandedKeys] = React.useState<Set<string>>(new Set());
  const [historyOpen, setHistoryOpen] = React.useState(false);
  const [episodePanel, dispatchEpisodePanel] = React.useReducer(episodePanelReducer, initialEpisodePanelState);
  const [searchBlockedByEpisode, setSearchBlockedByEpisode] = React.useState<Record<string, boolean>>({});
  const [searchBlockedByCollection, setSearchBlockedByCollection] = React.useState<Record<string, boolean>>({});
  const searchPrerequisiteNotice = !hasDownloadClients && showSearchPrerequisiteNotice
    ? <TitleSearchDownloadClientNotice />
    : null;

  React.useEffect(() => {
    setSearchBlockedByEpisode({});
    setSearchBlockedByCollection({});
  }, [title?.id]);

  React.useEffect(() => {
    if (hasDownloadClients) {
      setSearchBlockedByEpisode({});
      setSearchBlockedByCollection({});
    }
  }, [hasDownloadClients]);

  // Initialize expanded state when data arrives
  const initializedRef = React.useRef(false);
  React.useEffect(() => {
    if (initializedRef.current) return;

    // If we have an initialEpisodeId, find which collection it belongs to and expand that
    if (initialEpisodeId && Object.keys(episodesByCollection).length > 0) {
      for (const [collectionId, episodes] of Object.entries(episodesByCollection)) {
        const match = episodes.find((ep) => ep.id === initialEpisodeId);
        if (match) {
          initializedRef.current = true;
          setExpandedKeys(new Set([`s-${collectionId}`]));
          dispatchEpisodePanel({ type: "TOGGLE_EPISODE_ROW", episodeId: initialEpisodeId });
          // Scroll to the episode row after DOM updates
          requestAnimationFrame(() => {
            const el = document.querySelector(`[data-episode-id="${initialEpisodeId}"]`);
            el?.scrollIntoView({ behavior: "smooth", block: "center" });
          });
          return;
        }
      }
    }

    if (latestKey) {
      initializedRef.current = true;
      setExpandedKeys(new Set([latestKey]));
    }
  }, [latestKey, initialEpisodeId, episodesByCollection]);

  const toggleKey = React.useCallback((key: string) => {
    setExpandedKeys((prev) => {
      const next = new Set(prev);
      if (next.has(key)) {
        next.delete(key);
      } else {
        next.add(key);
      }
      return next;
    });
  }, []);

  const handleRunEpisodeSearch = React.useCallback(
    (episode: CollectionEpisode) => {
      if (!title) return;
      const episodeId = episode.id;

      if (!hasDownloadClients) {
        setSearchBlockedByEpisode((prev) => ({ ...prev, [episodeId]: true }));
        dispatchEpisodePanel({ type: "SET_SEARCH_RESULTS", episodeId, results: [] });
        dispatchEpisodePanel({ type: "SET_SEARCH_LOADING", episodeId, loading: false });
        return;
      }

      setSearchBlockedByEpisode((prev) => {
        if (!prev[episodeId]) return prev;
        const next = { ...prev };
        delete next[episodeId];
        return next;
      });
      dispatchEpisodePanel({ type: "SET_SEARCH_LOADING", episodeId, loading: true });

      const collection = collections.find((c) => c.id === episode.collectionId);
      const seasonNum = episode.seasonNumber?.trim().replace(/\D+/g, "")
        || collection?.collectionIndex?.trim().replace(/\D+/g, "")
        || "1";
      const episodeNum = episode.episodeNumber?.trim().replace(/\D+/g, "") || "1";

      client.query(searchForEpisodeQuery, {
        titleId: title.id,
        season: seasonNum,
        episode: episodeNum,
        }).toPromise()
        .then(({ data, error: queryError }) => {
          if (queryError) throw queryError;
          dispatchEpisodePanel({
            type: "SET_SEARCH_RESULTS",
            episodeId,
            results: data.searchReleases ?? [],
          });
        })
        .catch(() => {
          dispatchEpisodePanel({ type: "SET_SEARCH_RESULTS", episodeId, results: [] });
        })
        .finally(() => {
          dispatchEpisodePanel({ type: "SET_SEARCH_LOADING", episodeId, loading: false });
        });
    },
    [client, hasDownloadClients, title, collections],
  );

  const handleToggleEpisodeSearch = React.useCallback(
    (episode: CollectionEpisode) => {
      const episodeId = episode.id;
      const isOpen = episodePanel.expandedEpisodeRows.has(episodeId);
      const currentTab = episodePanel.episodeActiveTab[episodeId] ?? "details";

      if (isOpen && currentTab === "search") {
        dispatchEpisodePanel({ type: "TOGGLE_EPISODE_ROW", episodeId });
      } else {
        if (!isOpen) {
          dispatchEpisodePanel({ type: "TOGGLE_EPISODE_ROW", episodeId });
        }
        dispatchEpisodePanel({ type: "SET_EPISODE_TAB", episodeId, tab: "search" });
        if (
          searchBlockedByEpisode[episodeId]
          || !Object.prototype.hasOwnProperty.call(episodePanel.searchResultsByEpisode, episodeId)
        ) {
          handleRunEpisodeSearch(episode);
        }
      }
    },
    [episodePanel.expandedEpisodeRows, episodePanel.episodeActiveTab, handleRunEpisodeSearch, episodePanel.searchResultsByEpisode, searchBlockedByEpisode],
  );

  const handleToggleEpisodeDetails = React.useCallback(
    (episode: CollectionEpisode) => {
      const episodeId = episode.id;
      const isOpen = episodePanel.expandedEpisodeRows.has(episodeId);
      const currentTab = episodePanel.episodeActiveTab[episodeId] ?? "details";

      if (isOpen && currentTab === "details") {
        dispatchEpisodePanel({ type: "TOGGLE_EPISODE_ROW", episodeId });
      } else {
        if (!isOpen) {
          dispatchEpisodePanel({ type: "TOGGLE_EPISODE_ROW", episodeId });
        }
        dispatchEpisodePanel({ type: "SET_EPISODE_TAB", episodeId, tab: "details" });
      }
    },
    [episodePanel.expandedEpisodeRows, episodePanel.episodeActiveTab],
  );

  const handleEpisodeTabChange = React.useCallback(
    (episodeId: string, tab: EpisodePanelTab, episode: CollectionEpisode) => {
      dispatchEpisodePanel({ type: "SET_EPISODE_TAB", episodeId, tab });
      if (
        tab === "search"
        && (
          searchBlockedByEpisode[episodeId]
          || !Object.prototype.hasOwnProperty.call(episodePanel.searchResultsByEpisode, episodeId)
        )
      ) {
        handleRunEpisodeSearch(episode);
      }
    },
    [handleRunEpisodeSearch, episodePanel.searchResultsByEpisode, searchBlockedByEpisode],
  );

  const handleQueueFromEpisodeSearch = React.useCallback(
    (episode: CollectionEpisode, release: Release) => {
      if (!title) return Promise.resolve();

      if (!release.candidateToken) {
        setGlobalStatus(t("status.releaseMissingCandidateToken"));
        return Promise.resolve();
      }

      return client.mutation(queueExistingMutation, {
        input: {
          titleId: title.id,
          scope: { episode: episode.id },
          candidateToken: release.candidateToken,
        },
      }).toPromise()
        .then(async ({ error: mutationError }) => {
          if (mutationError) throw mutationError;
          const queuedMessage = t("status.queuedLatest", { name: title.name });
          setGlobalStatus(queuedMessage);
          await onTitleChanged?.();
        })
        .catch((error: unknown) => {
          setGlobalStatus(error instanceof Error ? error.message : t("status.queueFailed"));
        });
    },
    [onTitleChanged, client, setGlobalStatus, t, title],
  );

  const handleAutoSearchEpisode = React.useCallback(
    (episode: CollectionEpisode) => {
      if (!hasDownloadClients) {
        const episodeId = episode.id;
        if (!episodePanel.expandedEpisodeRows.has(episodeId)) {
          dispatchEpisodePanel({ type: "TOGGLE_EPISODE_ROW", episodeId });
        }
        dispatchEpisodePanel({ type: "SET_EPISODE_TAB", episodeId, tab: "search" });
        dispatchEpisodePanel({ type: "SET_SEARCH_RESULTS", episodeId, results: [] });
        setSearchBlockedByEpisode((prev) => ({ ...prev, [episodeId]: true }));
        return;
      }
      if (!onAutoSearchEpisode) return;
      const episodeId = episode.id;
      setSearchBlockedByEpisode((prev) => {
        if (!prev[episodeId]) return prev;
        const next = { ...prev };
        delete next[episodeId];
        return next;
      });
      dispatchEpisodePanel({ type: "SET_AUTO_SEARCH_LOADING", episodeId, loading: true });
      Promise.resolve(onAutoSearchEpisode(episode))
        .catch((error: unknown) => {
          setGlobalStatus(error instanceof Error ? error.message : t("status.queueFailed"));
        })
        .finally(() => {
          dispatchEpisodePanel({ type: "SET_AUTO_SEARCH_LOADING", episodeId, loading: false });
        });
    },
    [episodePanel.expandedEpisodeRows, hasDownloadClients, onAutoSearchEpisode, setGlobalStatus, t],
  );

  const [interstitialSearchLoading, setInterstitialSearchLoading] = React.useState(false);
  const handleAutoSearchInterstitialMovie = React.useCallback(
    (collection: TitleCollection) => {
      if (!hasDownloadClients) {
        setSearchBlockedByCollection((prev) => ({ ...prev, [collection.id]: true }));
        return;
      }
      if (!onAutoSearchInterstitialMovie) return;
      setSearchBlockedByCollection((prev) => {
        if (!prev[collection.id]) return prev;
        const next = { ...prev };
        delete next[collection.id];
        return next;
      });
      setInterstitialSearchLoading(true);
      Promise.resolve(onAutoSearchInterstitialMovie(collection))
        .catch((error: unknown) => {
          setGlobalStatus(error instanceof Error ? error.message : t("status.queueFailed"));
        })
        .finally(() => setInterstitialSearchLoading(false));
    },
    [hasDownloadClients, onAutoSearchInterstitialMovie, setGlobalStatus, t],
  );

  if (loading) {
    return (
      <div className="space-y-4">
        <div className="h-8 w-48 animate-pulse rounded bg-muted" />
        <div className="h-32 animate-pulse rounded-lg bg-muted" />
        <div className="h-48 animate-pulse rounded-lg bg-muted" />
      </div>
    );
  }

  if (!title) {
    return (
      <div className="space-y-4">
        <OverviewBackLink
          label={t("title.backToFacet", { facet: backLabel })}
          onClick={() => onBackToList?.()}
        />
        <Card>
          <CardContent className="pt-6">
            <p className="text-muted-foreground">{t("title.notFound")}</p>
          </CardContent>
        </Card>
      </div>
    );
  }

  return (
    <div className="space-y-4">
      <OverviewBackLink
        label={t("title.backToFacet", { facet: backLabel })}
        onClick={() => onBackToList?.()}
      />

      <Card
        className="relative overflow-hidden p-0"
        style={(title.backgroundUrl ?? title.bannerUrl) ? {
          backgroundImage: `linear-gradient(to top, var(--color-card) 0%, var(--color-card) 5%, color-mix(in srgb, var(--color-card) 80%, transparent), color-mix(in srgb, var(--color-card) 50%, transparent)), url(${title.backgroundUrl ?? title.bannerUrl})`,
          backgroundSize: "cover",
          backgroundPosition: "center top",
          backgroundClip: "padding-box",
        } : undefined}
      >
        <CardContent className="relative p-4">
          <div className="flex flex-col gap-4 sm:flex-row sm:gap-5">
            <div className="mx-auto shrink-0 sm:mx-0">
              <TitlePosterSlot
                src={title.posterUrl}
                sourceSrc={title.posterSourceUrl}
                metadataFetchedAt={title.metadataFetchedAt}
                createdAt={title.createdAt}
                alt={title.name}
                className="block h-auto w-32 rounded-lg object-cover shadow-lg sm:w-[180px]"
                placeholderClassName="flex h-48 w-32 items-center justify-center rounded-lg bg-muted text-sm text-muted-foreground/60 sm:h-[270px] sm:w-[180px]"
                emptyLabel={t("title.noPoster")}
              />
            </div>

            <div className="min-w-0 flex-1 flex flex-col">
              <h1 className="text-xl font-bold text-foreground sm:text-2xl">
                {title.name}
                {title.year ? (
                  <span className="block text-base font-normal text-muted-foreground sm:ml-2 sm:inline sm:text-lg">
                    ({title.year})
                  </span>
                ) : null}
              </h1>

              <div className="mt-2 flex flex-wrap items-center gap-2">
                <span
                  className={`inline-flex items-center rounded-full px-2.5 py-0.5 text-xs font-medium ${
                    title.monitored
                      ? "bg-emerald-500/20 text-emerald-700 dark:text-emerald-300"
                      : "bg-accent text-muted-foreground"
                  }`}
                >
                  {title.monitored
                    ? t("title.monitored")
                    : t("search.monitorType.unmonitored")}
                </span>
                {localizedTitleStatus(t, title.contentStatus) ? (
                  <span className="inline-flex items-center rounded-full border border-border px-2.5 py-0.5 text-xs font-medium capitalize text-muted-foreground">
                    {localizedTitleStatus(t, title.contentStatus)}
                  </span>
                ) : null}
                {title.network ? (
                  <span className="inline-flex items-center gap-1 text-xs text-muted-foreground">
                    <Clapperboard className="h-3.5 w-3.5" />
                    {title.network}
                  </span>
                ) : null}
              </div>

              {title.genres.length > 0 ? (
                <div className="mt-2 flex flex-wrap gap-1.5">
                  {title.genres.map((genre) => (
                    <span
                      key={genre}
                      className="rounded bg-muted px-2 py-0.5 text-xs text-muted-foreground"
                    >
                      {genre}
                    </span>
                  ))}
                </div>
              ) : null}

              {title.overview ? (
                <p className="mt-4 text-sm leading-relaxed text-foreground/70">
                  {title.overview}
                </p>
              ) : null}

              <div className="mt-auto flex flex-wrap items-center gap-3 pt-3">
                {(() => { const e = title.externalIds.find((e) => e.source === "imdb"); return e ? (
                  <a
                    href={e.value.startsWith("tt") ? `https://www.imdb.com/title/${e.value}` : `https://www.imdb.com/find?q=${encodeURIComponent(e.value)}&s=tt`}
                    target="_blank"
                    rel="noreferrer"
                    className="inline-flex h-12 items-center gap-2 rounded-md border border-border bg-card/45 px-3 py-2 text-base hover:bg-muted"
                    aria-label={t("external.openOn", { site: "IMDb" })}
                  >
                    <img src={imdbLogoUrl} alt="IMDb" className="h-8 w-8" />
                    <span className="text-muted-foreground">IMDb</span>
                  </a>
                ) : null; })()}
                {(() => { const e = title.externalIds.find((e) => e.source === "tvdb"); return e && title.slug ? (
                  <a
                    href={`https://thetvdb.com/series/${title.slug}`}
                    target="_blank"
                    rel="noreferrer"
                    className="inline-flex h-12 items-center gap-2 rounded-md border border-border bg-card/45 px-3 py-2 text-base hover:bg-muted"
                    aria-label={t("external.openOn", { site: "TVDB" })}
                  >
                    <img src={tvdbLogoUrl} alt="TVDB" className="h-8 w-8" />
                    <span className="text-muted-foreground">TVDB</span>
                  </a>
                ) : null; })()}
                {(() => { const e = title.externalIds.find((e) => e.source === "tmdb"); return e ? (
                  <a
                    href={`https://www.themoviedb.org/tv/${e.value}`}
                    target="_blank"
                    rel="noreferrer"
                    className="inline-flex h-12 items-center gap-2 rounded-md border border-border bg-card/45 px-3 py-2 text-base hover:bg-muted"
                    aria-label={t("external.openOn", { site: "TMDB" })}
                  >
                    <img src={tmdbLogoUrl} alt="TMDB" className="h-8 w-8" />
                    <span className="text-muted-foreground">TMDB</span>
                  </a>
                ) : null; })()}
                {title.facet === "anime" ? (
                  <>
                    {(() => { const e = title.externalIds.find((e) => e.source === "mal"); return e ? (
                      <a
                        href={`https://myanimelist.net/anime/${e.value}`}
                        target="_blank"
                        rel="noreferrer"
                        className="inline-flex h-12 items-center gap-2 rounded-md border border-border bg-card/45 px-3 py-2 text-base hover:bg-muted"
                        aria-label={t("external.openOn", { site: "MyAnimeList" })}
                      >
                        <img src={malLogoUrl} alt="MyAnimeList" className="h-8 w-8" />
                        <span className="text-muted-foreground">MAL</span>
                      </a>
                    ) : null; })()}
                    {(() => { const e = title.externalIds.find((e) => e.source === "anilist"); return e ? (
                      <a
                        href={`https://anilist.co/anime/${e.value}`}
                        target="_blank"
                        rel="noreferrer"
                        className="inline-flex h-12 items-center gap-2 rounded-md border border-border bg-card/45 px-3 py-2 text-base hover:bg-muted"
                        aria-label={t("external.openOn", { site: "AniList" })}
                      >
                        <img src={anilistLogoUrl} alt="AniList" className="h-8 w-8" />
                        <span className="text-muted-foreground">AniList</span>
                      </a>
                    ) : null; })()}
                    {(() => { const e = title.externalIds.find((e) => e.source === "anidb"); return e ? (
                      <a
                        href={`https://anidb.net/anime/${e.value}`}
                        target="_blank"
                        rel="noreferrer"
                        className="inline-flex h-12 items-center gap-2 rounded-md border border-border bg-card/45 px-3 py-2 text-base hover:bg-muted"
                        aria-label={t("external.openOn", { site: "AniDB" })}
                      >
                        <img src={anidbLogoUrl} alt="AniDB" className="h-8 w-8" />
                        <span className="text-muted-foreground">AniDB</span>
                      </a>
                    ) : null; })()}
                  </>
                ) : null}
                <span className="ml-auto text-xs text-muted-foreground/60">
                  {t("title.addedAt", { date: formatDate(title.createdAt) })}
                </span>
              </div>
            </div>
          </div>
        </CardContent>
      </Card>

      <OverviewControlPanel
        monitored={title.monitored}
        monitoredUpdating={monitoredUpdating}
        searchMonitoredLoading={searchMonitoredLoading}
        refreshAndScanLoading={refreshAndScanLoading}
        deleteLoading={deleteLoading}
        onToggleMonitoring={onSetTitleMonitored ? () => void onSetTitleMonitored(!title.monitored) : undefined}
        onSearchMonitored={onSearchMonitored ? () => void onSearchMonitored() : undefined}
        onRefreshAndScan={onRefreshAndScan ? () => void onRefreshAndScan() : undefined}
        onRequestDelete={onRequestDeleteTitle}
        onHistory={() => setHistoryOpen(true)}
        searchNotice={searchPrerequisiteNotice}
        settingsPanel={
          onUpdateTitleOptions && qualityProfiles && defaultRootFolder ? (
            <TitleSettingsPanel
              title={title}
              qualityProfiles={qualityProfiles}
              defaultRootFolder={defaultRootFolder}
              rootFolders={rootFolders ?? []}
              onUpdateTitleOptions={onUpdateTitleOptions}
              onTitleChanged={onTitleChanged}
              onOpenFixMatch={onOpenFixMatch}
            />
          ) : undefined
        }
      />

      {acquisitionDiagnostics ? (
        <Card>
          <CardHeader>
            <div className="flex items-center justify-between gap-3">
              <CardTitle className="text-base">Acquisition diagnostics</CardTitle>
              {acquisitionDiagnostics.mismatchRecoveryEligibleCount > 0 && onTriggerMismatchRecovery ? (
                <Button size="sm" variant="secondary" onClick={() => void onTriggerMismatchRecovery()}>
                  Recover mismatches
                </Button>
              ) : null}
            </div>
          </CardHeader>
          <CardContent className="space-y-4 text-sm">
            <div className="flex flex-wrap gap-4 text-muted-foreground">
              <span>Latest decision: {formatDate(acquisitionDiagnostics.latestDecisionAt)}</span>
              <span>Latest wanted search: {formatDate(acquisitionDiagnostics.latestWantedSearchAt)}</span>
              <span>Mismatch recovery eligible: {acquisitionDiagnostics.mismatchRecoveryEligibleCount}</span>
            </div>
            <div className="grid gap-3 md:grid-cols-3">
              <div>
                <div className="mb-2 text-xs font-medium uppercase tracking-wide text-muted-foreground">
                  Recent decision counts
                </div>
                <div className="space-y-1">
                  {acquisitionDiagnostics.decisionCounts.map((item) => (
                    <div key={item.code} className="flex items-center justify-between gap-3">
                      <span>{item.code}</span>
                      <span className="text-muted-foreground">{item.count}</span>
                    </div>
                  ))}
                </div>
              </div>
              <div>
                <div className="mb-2 text-xs font-medium uppercase tracking-wide text-muted-foreground">
                  Wanted status counts
                </div>
                <div className="space-y-1">
                  {acquisitionDiagnostics.wantedStatusCounts.map((item) => (
                    <div key={item.status} className="flex items-center justify-between gap-3">
                      <span>{item.status}</span>
                      <span className="text-muted-foreground">{item.count}</span>
                    </div>
                  ))}
                </div>
              </div>
              <div>
                <div className="mb-2 text-xs font-medium uppercase tracking-wide text-muted-foreground">
                  Pending release counts
                </div>
                <div className="space-y-1">
                  {acquisitionDiagnostics.pendingReleaseCounts.map((item) => (
                    <div key={item.status} className="flex items-center justify-between gap-3">
                      <span>{item.status}</span>
                      <span className="text-muted-foreground">{item.count}</span>
                    </div>
                  ))}
                </div>
              </div>
            </div>
          </CardContent>
        </Card>
      ) : null}

      <div>
        <Card className="relative overflow-hidden">
          <CardHeader>
            <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
              <CardTitle className="flex items-center gap-2 text-base">
                <FolderOpen className="h-4 w-4" />
                {t("title.seasonsAndEpisodes")}
              </CardTitle>
              {onOpenManualImport && completedDownloads && completedDownloads.length > 0 ? (
                <Button
                  className="w-full sm:w-auto"
                  variant="outline"
                  size="sm"
                  onClick={() => onOpenManualImport(completedDownloads[0])}
                >
                  <FileInput className="mr-1.5 h-4 w-4" />
                  {t("queue.manualImport")}
                </Button>
              ) : null}
            </div>
          </CardHeader>
          <CardContent className="space-y-4">
            {sortedCollections.length > 0 ? (
              sortedCollections.map((collection) => {
                const key = `s-${collection.id}`;
                const sortedEpisodes = [
                  ...(episodesByCollection[collection.id] ?? []),
                ].sort((left, right) => episodeSortValue(right) - episodeSortValue(left));

                // Hide specials section when it has no episodes and no movies
                if (isSpecialsCollection(collection) && sortedEpisodes.length === 0 && collection.specialsMovies.length === 0) {
                  return null;
                }

                return (
                  <SeasonSection
                    key={key}
                    collection={collection}
                    episodes={sortedEpisodes}
                    facet={title.facet}
                    expanded={expandedKeys.has(key)}
                    onToggle={() => toggleKey(key)}
                    expandedEpisodeRows={episodePanel.expandedEpisodeRows}
                    episodeActiveTab={episodePanel.episodeActiveTab}
                    mediaFilesByEpisode={mediaFilesByEpisode}
                    subtitleDownloads={subtitleDownloads}
                    onRefreshSubtitles={onRefreshSubtitles}
                    releaseBlocklistEntries={releaseBlocklistEntries}
                    searchResultsByEpisode={episodePanel.searchResultsByEpisode}
                    searchLoadingByEpisode={episodePanel.searchLoadingByEpisode}
                    searchBlockedByEpisode={searchBlockedByEpisode}
                    autoSearchLoadingByEpisode={episodePanel.autoSearchLoadingByEpisode}
                    onToggleEpisodeSearch={handleToggleEpisodeSearch}
                    onToggleEpisodeDetails={handleToggleEpisodeDetails}
                    onEpisodeTabChange={handleEpisodeTabChange}
                    onRunEpisodeSearch={handleRunEpisodeSearch}
                    onQueueFromEpisodeSearch={handleQueueFromEpisodeSearch}
                    onAutoSearchEpisode={handleAutoSearchEpisode}
                    onSetCollectionMonitored={onSetCollectionMonitored}
                    onSetEpisodeMonitored={onSetEpisodeMonitored}
                    seasonSearchResults={seasonSearchResultsByCollection?.[collection.id]}
                    seasonSearchLoading={seasonSearchLoadingByCollection?.[collection.id] === true}
                    onRunSeasonSearch={onRunSeasonSearch ? () => {
                      if (!hasDownloadClients) {
                        setSearchBlockedByCollection((prev) => ({ ...prev, [collection.id]: true }));
                        return;
                      }
                      setSearchBlockedByCollection((prev) => {
                        if (!prev[collection.id]) return prev;
                        const next = { ...prev };
                        delete next[collection.id];
                        return next;
                      });
                      return onRunSeasonSearch(collection);
                    } : undefined}
                    searchBlocked={searchBlockedByCollection[collection.id] === true}
                    onQueueFromSeasonSearch={onQueueFromSeasonSearch}
                    onDeleteFile={onDeleteFile}
                    onAutoSearchInterstitialMovie={onAutoSearchInterstitialMovie ? handleAutoSearchInterstitialMovie : undefined}
                    autoSearchInterstitialMovieLoading={interstitialSearchLoading}
                  />
                );
              })
            ) : (
              <p className="text-sm text-muted-foreground">
                {t("title.noTrackedSeasons")}
              </p>
            )}
          </CardContent>
          {hydrating ? (
            <div className="absolute inset-0 z-10 flex items-center justify-center bg-background/75 backdrop-blur-sm">
              <div className="flex items-center gap-3 rounded-full border border-border bg-card/95 px-4 py-2 text-sm font-medium text-foreground shadow-lg">
                <Loader2 className="h-4 w-4 animate-spin" />
                <span>{t("title.fetchingData")}</span>
              </div>
            </div>
          ) : null}
        </Card>
      </div>

      {title ? (
        <TitleHistoryModal
          open={historyOpen}
          onOpenChange={setHistoryOpen}
          titleId={title.id}
          titleName={title.name}
        />
      ) : null}
    </div>
  );
}
