import * as React from "react";
import { FileInput, FolderOpen, Loader2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Clapperboard } from "lucide-react";
import { useClient } from "urql";
import type { Release } from "@/lib/types";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useDownloadConflictConfirmation } from "@/components/common/download-conflict-confirmation";
import { userFacingGraphQlErrorMessage } from "@/lib/graphql/error-message";
import { releaseQueueScopeInput } from "@/lib/utils/release-queue-scope";
import { TitlePosterSlot } from "@/components/title-poster-slot";
import {
  searchForEpisodeQuery,
  searchForInterstitialMovieQuery,
} from "@/lib/graphql/queries";
import { queueExistingMutation } from "@/lib/graphql/mutations";
import {
  assertNoReplaceConflict,
  retryWithReplaceOnConflict,
} from "@/lib/utils/download-conflicts";
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
import type { ExternalSubtitleRecord } from "@/lib/types/subtitles";

const EPISODE_QUEUE_PRECEDENCE: Record<string, number> = {
  downloading: 0,
  post_processing: 1,
  queued: 2,
  paused: 3,
  import_pending: 4,
  importing: 5,
};

function compareEpisodeQueueItems(
  left: DownloadQueueItem,
  right: DownloadQueueItem,
): number {
  const leftRank = EPISODE_QUEUE_PRECEDENCE[left.displayState] ?? Number.MAX_SAFE_INTEGER;
  const rightRank = EPISODE_QUEUE_PRECEDENCE[right.displayState] ?? Number.MAX_SAFE_INTEGER;
  if (leftRank !== rightRank) {
    return leftRank - rightRank;
  }

  const leftUpdatedAt = Date.parse(left.lastUpdatedAt ?? "");
  const rightUpdatedAt = Date.parse(right.lastUpdatedAt ?? "");
  if (Number.isFinite(leftUpdatedAt) && Number.isFinite(rightUpdatedAt) && leftUpdatedAt !== rightUpdatedAt) {
    return rightUpdatedAt - leftUpdatedAt;
  }

  return right.progressPercent - left.progressPercent;
}

function coveredEpisodeIdsForQueueItem(
  item: DownloadQueueItem,
  episodesByCollection: Record<string, CollectionEpisode[]>,
): string[] {
  const episodeIds = new Set<string>();
  if (item.episodeId) {
    episodeIds.add(item.episodeId);
  }

  const scope = item.queueScope;
  if (!scope) {
    return Array.from(episodeIds);
  }

  if (scope.kind === "episode" && scope.episodeId) {
    episodeIds.add(scope.episodeId);
  }

  if (scope.kind === "episode_set") {
    for (const episodeId of scope.episodeIds) {
      episodeIds.add(episodeId);
    }
  }

  if (scope.kind === "collection" && scope.collectionId) {
    for (const episode of episodesByCollection[scope.collectionId] ?? []) {
      episodeIds.add(episode.id);
    }
  }

  return Array.from(episodeIds);
}

const imdbLogoUrl = `${import.meta.env.BASE_URL}media-sites/imdb.svg`;
const tvdbLogoUrl = `${import.meta.env.BASE_URL}media-sites/tvdb.svg`;
const tmdbLogoUrl = `${import.meta.env.BASE_URL}media-sites/tmdb.svg`;
const malLogoUrl = `${import.meta.env.BASE_URL}media-sites/mal.svg`;
const anilistLogoUrl = `${import.meta.env.BASE_URL}media-sites/anilist.svg`;
const anidbLogoUrl = `${import.meta.env.BASE_URL}media-sites/anidb.png`;

type Props = {
  canManageTitle: boolean;
  loading: boolean;
  hydrating: boolean;
  title: TitleDetail | null;
  collections: TitleCollection[];
  events: TitleHistoryEvent[];
  episodesByCollection: Record<string, CollectionEpisode[]>;
  mediaFilesByEpisode: Record<string, EpisodeMediaFile[]>;
  downloadQueueItems?: DownloadQueueItem[];
  subtitleDownloads?: ExternalSubtitleRecord[];
  onRefreshSubtitles?: () => Promise<void> | void;
  releaseBlocklistEntries: TitleReleaseBlocklistEntry[];
  clearingReleaseBlocklistEntryId?: string | null;
  onClearReleaseBlocklistEntry?: (entryId: string) => Promise<void> | void;
  onTitleChanged?: () => Promise<void>;
  onBackToList?: () => void;
  onSetCollectionMonitored?: (collectionId: string, monitored: boolean) => Promise<void>;
  onSetEpisodeMonitored?: (episodeId: string, monitored: boolean) => Promise<void>;
  onSetTitleMonitored?: (monitored: boolean) => Promise<void>;
  onSearchMonitored?: () => Promise<void> | void;
  onRefreshAndScan?: () => Promise<void> | void;
  onAutoSearchEpisode?: (episode: CollectionEpisode) => Promise<void> | void;
  onAutoSearchInterstitialMovie?: (collection: TitleCollection) => Promise<void> | void;
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
  canManageTitle,
  loading,
  hydrating,
  title,
  collections,
  events: _events,
  episodesByCollection,
  mediaFilesByEpisode,
  downloadQueueItems = [],
  subtitleDownloads,
  onRefreshSubtitles,
  releaseBlocklistEntries,
  clearingReleaseBlocklistEntryId,
  onClearReleaseBlocklistEntry,
  onTitleChanged,
  onBackToList,
  onSetCollectionMonitored,
  onSetEpisodeMonitored,
  onSetTitleMonitored,
  onSearchMonitored,
  onRefreshAndScan,
  onAutoSearchEpisode,
  onAutoSearchInterstitialMovie,
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
  const emptyEpisodes = React.useRef<CollectionEpisode[]>([]).current;
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const client = useClient();
  const { confirmReplaceConflict, replaceConflictDialog } =
    useDownloadConflictConfirmation();
  const backLabel = title?.facet === "anime" ? t("nav.anime") : t("nav.series");
  const sortedCollections = React.useMemo(
    () => sortDbCollections(collections),
    [collections],
  );

  const latestKey = React.useMemo(
    () => findLatestSeasonKey(sortedCollections),
    [sortedCollections],
  );

  const sortedEpisodesByCollection = React.useMemo(
    () => Object.fromEntries(
      sortedCollections.map((collection) => [
        collection.id,
        [...(episodesByCollection[collection.id] ?? emptyEpisodes)].sort(
          (left, right) => episodeSortValue(right) - episodeSortValue(left),
        ),
      ]),
    ) as Record<string, CollectionEpisode[]>,
    [emptyEpisodes, episodesByCollection, sortedCollections],
  );

  const [expandedKeys, setExpandedKeys] = React.useState<Set<string>>(new Set());
  const [historyOpen, setHistoryOpen] = React.useState(false);
  const [historyEpisodeScope, setHistoryEpisodeScope] = React.useState<{
    episodeId: string;
    episodeLabel: string;
  } | null>(null);
  const [episodePanel, dispatchEpisodePanel] = React.useReducer(episodePanelReducer, initialEpisodePanelState);
  const [searchBlockedByEpisode, setSearchBlockedByEpisode] = React.useState<Record<string, boolean>>({});
  const [searchBlockedByCollection, setSearchBlockedByCollection] = React.useState<Record<string, boolean>>({});
  const [interstitialSearchResultsByCollection, setInterstitialSearchResultsByCollection] =
    React.useState<Record<string, Release[]>>({});
  const [interstitialSearchLoadingByCollection, setInterstitialSearchLoadingByCollection] =
    React.useState<Record<string, boolean>>({});
  const [interstitialSearchAttemptedByCollection, setInterstitialSearchAttemptedByCollection] =
    React.useState<Record<string, boolean>>({});
  const [autoSearchInterstitialMovieLoadingByCollection, setAutoSearchInterstitialMovieLoadingByCollection] =
    React.useState<Record<string, boolean>>({});
  const searchPrerequisiteNotice = canManageTitle && !hasDownloadClients && showSearchPrerequisiteNotice
    ? <TitleSearchDownloadClientNotice />
    : null;
  const { primaryQueueItemByEpisodeId } = React.useMemo(() => {
    const queueItemsByEpisodeId: Record<string, DownloadQueueItem[]> = {};

    for (const item of downloadQueueItems) {
      for (const episodeId of coveredEpisodeIdsForQueueItem(item, sortedEpisodesByCollection)) {
        (queueItemsByEpisodeId[episodeId] ??= []).push(item);
      }
    }

    const primaryByEpisodeId = Object.fromEntries(
      Object.entries(queueItemsByEpisodeId).map(([episodeId, items]) => [
        episodeId,
        [...items].sort(compareEpisodeQueueItems)[0],
      ]),
    ) as Record<string, DownloadQueueItem | undefined>;

    return {
      primaryQueueItemByEpisodeId: primaryByEpisodeId,
    };
  }, [downloadQueueItems, sortedEpisodesByCollection]);

  React.useEffect(() => {
    setSearchBlockedByEpisode({});
    setSearchBlockedByCollection({});
    setInterstitialSearchResultsByCollection({});
    setInterstitialSearchLoadingByCollection({});
    setInterstitialSearchAttemptedByCollection({});
    setAutoSearchInterstitialMovieLoadingByCollection({});
  }, [title?.id]);

  React.useEffect(() => {
    if (hasDownloadClients) {
      setSearchBlockedByEpisode({});
      setSearchBlockedByCollection({});
    }
  }, [hasDownloadClients]);

  const handleOpenTitleHistory = React.useCallback(() => {
    setHistoryEpisodeScope(null);
    setHistoryOpen(true);
  }, []);

  const handleOpenEpisodeHistory = React.useCallback((episode: CollectionEpisode) => {
    setHistoryEpisodeScope({
      episodeId: episode.id,
      episodeLabel:
        episode.title ?? episode.episodeLabel ?? episode.episodeNumber ?? episode.id,
    });
    setHistoryOpen(true);
  }, []);

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

  const handleQueueFromEpisodeSearch = React.useCallback(
    (episode: CollectionEpisode, release: Release) => {
      if (!title) return Promise.resolve();

      if (!release.candidateToken) {
        setGlobalStatus(t("status.releaseMissingCandidateToken"));
        return Promise.resolve();
      }

      const input = {
        titleId: title.id,
        scope: { episode: episode.id },
        candidateToken: release.candidateToken,
      };
      return retryWithReplaceOnConflict(
        input,
        async (nextInput) => {
          const { data, error: mutationError } = await client.mutation(queueExistingMutation, {
            input: nextInput,
          }).toPromise();
          if (mutationError) throw mutationError;
          return data?.queueExistingTitleDownload;
        },
        "A download is already in progress for this episode.",
        confirmReplaceConflict,
      )
        .then(async (payload) => {
          assertNoReplaceConflict(payload, "A download is already in progress for this episode.");
          const queuedMessage = t("status.queuedLatest", { name: title.name });
          setGlobalStatus(queuedMessage);
          await onTitleChanged?.();
        })
        .catch((error: unknown) => {
          setGlobalStatus(userFacingGraphQlErrorMessage(error, t("status.queueFailed")));
        });
    },
    [onTitleChanged, client, confirmReplaceConflict, setGlobalStatus, t, title],
  );

  const handleAutoSearchEpisode = React.useCallback(
    (episode: CollectionEpisode) => {
      if (!hasDownloadClients) {
        const episodeId = episode.id;
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
          setGlobalStatus(userFacingGraphQlErrorMessage(error, t("status.queueFailed")));
        })
        .finally(() => {
          dispatchEpisodePanel({ type: "SET_AUTO_SEARCH_LOADING", episodeId, loading: false });
        });
    },
    [hasDownloadClients, onAutoSearchEpisode, setGlobalStatus, t],
  );

  const handleRunInterstitialMovieSearch = React.useCallback(
    (collection: TitleCollection) => {
      if (!title || !collection.interstitialMovie) return;

      if (!hasDownloadClients) {
        setSearchBlockedByCollection((prev) => ({ ...prev, [collection.id]: true }));
        setInterstitialSearchLoadingByCollection((prev) => ({
          ...prev,
          [collection.id]: false,
        }));
        return;
      }

      setSearchBlockedByCollection((prev) => {
        if (!prev[collection.id]) return prev;
        const next = { ...prev };
        delete next[collection.id];
        return next;
      });
      setInterstitialSearchLoadingByCollection((prev) => ({
        ...prev,
        [collection.id]: true,
      }));
      setInterstitialSearchAttemptedByCollection((prev) => ({
        ...prev,
        [collection.id]: true,
      }));

      client
        .query(searchForInterstitialMovieQuery, {
          titleId: title.id,
          collectionId: collection.id,
        })
        .toPromise()
        .then(({ data, error: queryError }) => {
          if (queryError) throw queryError;
          setInterstitialSearchResultsByCollection((prev) => ({
            ...prev,
            [collection.id]: data?.searchReleases ?? [],
          }));
        })
        .catch(() => {
          setInterstitialSearchResultsByCollection((prev) => ({
            ...prev,
            [collection.id]: [],
          }));
        })
        .finally(() => {
          setInterstitialSearchLoadingByCollection((prev) => ({
            ...prev,
            [collection.id]: false,
          }));
        });
    },
    [client, hasDownloadClients, title],
  );

  const handleQueueFromInterstitialMovieSearch = React.useCallback(
    (collection: TitleCollection, release: Release) => {
      if (!title || !collection.interstitialMovie) return Promise.resolve();

      if (!release.candidateToken) {
        setGlobalStatus(t("status.releaseMissingCandidateToken"));
        return Promise.resolve();
      }

      const input = {
        titleId: title.id,
        scope: releaseQueueScopeInput(release, { collection: collection.id }),
        candidateToken: release.candidateToken,
      };
      return retryWithReplaceOnConflict(
        input,
        async (nextInput) => {
          const { data, error: mutationError } = await client
            .mutation(queueExistingMutation, {
              input: nextInput,
            })
            .toPromise();
          if (mutationError) throw mutationError;
          return data?.queueExistingTitleDownload;
        },
        "A download is already in progress for this collection.",
        confirmReplaceConflict,
      )
        .then(async (payload) => {
          assertNoReplaceConflict(
            payload,
            "A download is already in progress for this collection.",
          );
          setGlobalStatus(
            t("status.queuedLatest", { name: collection.interstitialMovie?.name ?? title.name }),
          );
          await onTitleChanged?.();
        })
        .catch((error: unknown) => {
          setGlobalStatus(userFacingGraphQlErrorMessage(error, t("status.queueFailed")));
        });
    },
    [client, confirmReplaceConflict, onTitleChanged, setGlobalStatus, t, title],
  );

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
      setAutoSearchInterstitialMovieLoadingByCollection((prev) => ({
        ...prev,
        [collection.id]: true,
      }));
      Promise.resolve(onAutoSearchInterstitialMovie(collection))
        .catch((error: unknown) => {
          setGlobalStatus(userFacingGraphQlErrorMessage(error, t("status.queueFailed")));
        })
        .finally(() => {
          setAutoSearchInterstitialMovieLoadingByCollection((prev) => ({
            ...prev,
            [collection.id]: false,
          }));
        });
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

  const overviewBackdropUrl = title.backgroundUrl ?? title.bannerUrl;

  return (
    <>
      <div className="space-y-4">
      <OverviewBackLink
        label={t("title.backToFacet", { facet: backLabel })}
        onClick={() => onBackToList?.()}
      />

      <Card
        className="relative overflow-hidden p-0"
        style={overviewBackdropUrl ? { backdropFilter: "none", WebkitBackdropFilter: "none" } : undefined}
      >
        {overviewBackdropUrl ? (
          <div
            aria-hidden="true"
            className="pointer-events-none absolute inset-0"
            style={{ position: "absolute", inset: 0, zIndex: 0 }}
          >
            <div
              className="absolute -inset-1 scale-[1.03] bg-cover bg-no-repeat blur-[2px] brightness-[0.82] saturate-[0.9]"
              style={{
                backgroundImage: `url(${overviewBackdropUrl})`,
                backgroundPosition: "center top",
              }}
            />
            <div
              className="absolute inset-0"
              style={{
                background:
                  "linear-gradient(to top, var(--color-card) 0%, var(--color-card) 5%, color-mix(in srgb, var(--color-card) 82%, transparent), color-mix(in srgb, var(--color-card) 52%, transparent)), linear-gradient(135deg, rgba(255, 255, 255, 0.03), rgba(255, 255, 255, 0.012) 40%, transparent 100%)",
              }}
            />
          </div>
        ) : null}
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
                {(() => { const externalIds = title.externalIds ?? []; const e = externalIds.find((e) => e.source === "imdb"); return e ? (
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
                {(() => { const externalIds = title.externalIds ?? []; const e = externalIds.find((e) => e.source === "tvdb"); return e && title.slug ? (
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
                {(() => { const externalIds = title.externalIds ?? []; const e = externalIds.find((e) => e.source === "tmdb"); return e ? (
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
                    {(() => { const externalIds = title.externalIds ?? []; const e = externalIds.find((e) => e.source === "mal"); return e ? (
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
                    {(() => { const externalIds = title.externalIds ?? []; const e = externalIds.find((e) => e.source === "anilist"); return e ? (
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
                    {(() => { const externalIds = title.externalIds ?? []; const e = externalIds.find((e) => e.source === "anidb"); return e ? (
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

      {canManageTitle ? (
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
          onHistory={handleOpenTitleHistory}
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
      ) : null}

      <div>
        <Card className="relative overflow-hidden">
          <CardHeader>
            <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
              <CardTitle className="flex items-center gap-2 text-base">
                <FolderOpen className="h-4 w-4" />
                {t("title.seasonsAndEpisodes")}
              </CardTitle>
              {canManageTitle && onOpenManualImport && completedDownloads && completedDownloads.length > 0 ? (
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
                const sortedEpisodes = sortedEpisodesByCollection[collection.id] ?? emptyEpisodes;

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
                    initiallyOpenEpisodeId={initialEpisodeId}
                    mediaFilesByEpisode={mediaFilesByEpisode}
                    downloadQueueItemByEpisodeId={primaryQueueItemByEpisodeId}
                    subtitleDownloads={subtitleDownloads}
                    onRefreshSubtitles={canManageTitle ? onRefreshSubtitles : undefined}
                    releaseBlocklistEntries={releaseBlocklistEntries}
                    clearingReleaseBlocklistEntryId={clearingReleaseBlocklistEntryId}
                    onClearReleaseBlocklistEntry={
                      canManageTitle ? onClearReleaseBlocklistEntry : undefined
                    }
                    searchResultsByEpisode={episodePanel.searchResultsByEpisode}
                    searchLoadingByEpisode={episodePanel.searchLoadingByEpisode}
                    searchBlockedByEpisode={searchBlockedByEpisode}
                    autoSearchLoadingByEpisode={episodePanel.autoSearchLoadingByEpisode}
                    onRunEpisodeSearch={canManageTitle ? handleRunEpisodeSearch : undefined}
                    onOpenEpisodeHistory={canManageTitle ? handleOpenEpisodeHistory : undefined}
                    onQueueFromEpisodeSearch={canManageTitle ? handleQueueFromEpisodeSearch : undefined}
                    onAutoSearchEpisode={canManageTitle ? handleAutoSearchEpisode : undefined}
                    onSetCollectionMonitored={canManageTitle ? onSetCollectionMonitored : undefined}
                    onSetEpisodeMonitored={canManageTitle ? onSetEpisodeMonitored : undefined}
                    seasonSearchResults={seasonSearchResultsByCollection?.[collection.id]}
                    seasonSearchLoading={seasonSearchLoadingByCollection?.[collection.id] === true}
                    onRunSeasonSearch={canManageTitle && onRunSeasonSearch ? () => {
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
                    onQueueFromSeasonSearch={canManageTitle ? onQueueFromSeasonSearch : undefined}
                    onDeleteFile={canManageTitle ? onDeleteFile : undefined}
                    interstitialSearchResults={interstitialSearchResultsByCollection[collection.id]}
                    interstitialSearchLoading={interstitialSearchLoadingByCollection[collection.id] === true}
                    interstitialSearchAttempted={interstitialSearchAttemptedByCollection[collection.id] === true}
                    onRunInterstitialMovieSearch={canManageTitle ? handleRunInterstitialMovieSearch : undefined}
                    onQueueFromInterstitialMovieSearch={canManageTitle ? handleQueueFromInterstitialMovieSearch : undefined}
                    onAutoSearchInterstitialMovie={canManageTitle && onAutoSearchInterstitialMovie ? handleAutoSearchInterstitialMovie : undefined}
                    autoSearchInterstitialMovieLoading={autoSearchInterstitialMovieLoadingByCollection[collection.id] === true}
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

      <details className="rounded-xl border border-border bg-card text-card-foreground overflow-hidden">
        <summary className="cursor-pointer select-none px-4 py-3 text-sm font-medium text-card-foreground">
          <span className="inline-flex items-center gap-2">
            {t("title.blockedReleases")}
            <span className="rounded-full bg-muted px-2 py-0.5 text-xs text-muted-foreground">
              {releaseBlocklistEntries.length}
            </span>
          </span>
        </summary>
        <div className="border-t border-border p-4">
          {releaseBlocklistEntries.length === 0 ? (
            <p className="text-sm text-muted-foreground">
              {t("title.noBlockedReleases")}
            </p>
          ) : (
            <div className="space-y-2">
              {releaseBlocklistEntries.map((entry) => (
                <div
                  key={entry.id}
                  className="rounded-lg border border-border bg-background/35 p-3"
                >
                  <div className="flex items-center justify-between gap-3">
                    <div className="min-w-0 flex-1">
                      <p className="break-words text-sm text-card-foreground">
                        {entry.sourceTitle || t("episode.untitledRelease")}
                      </p>
                      <div className="mt-2 flex flex-wrap items-center gap-2 text-xs">
                        <span className="text-muted-foreground/60">{formatDate(entry.attemptedAt)}</span>
                        {entry.errorMessage ? (
                          <span className="rounded bg-red-950/40 px-2 py-0.5 text-red-200">
                            {entry.errorMessage}
                          </span>
                        ) : null}
                      </div>
                    </div>
                    {canManageTitle && onClearReleaseBlocklistEntry ? (
                      <Button
                        type="button"
                        variant="destructive"
                        size="sm"
                        className="h-8 shrink-0 px-3"
                        disabled={clearingReleaseBlocklistEntryId === entry.id}
                        onClick={() => onClearReleaseBlocklistEntry(entry.id)}
                      >
                        {clearingReleaseBlocklistEntryId === entry.id ? (
                          <Loader2 className="size-3.5 animate-spin" />
                        ) : null}
                        <span>{t("label.clear")}</span>
                      </Button>
                    ) : null}
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>
      </details>

      {title ? (
        <TitleHistoryModal
          open={historyOpen}
          onOpenChange={setHistoryOpen}
          titleId={title.id}
          titleName={title.name}
          scopedEpisode={historyEpisodeScope}
        />
      ) : null}
      {replaceConflictDialog}
      </div>
    </>
  );
}
