
import * as React from "react";
import {
  deleteMediaFilePreviewQuery,
  deleteTitlePreviewQuery,
  seriesOverviewSettingsInitQuery,
} from "@/lib/graphql/queries";
import {
  deleteMediaFileMutation,
  deleteTitleMutation,
  scanTitleLibraryMutation,
  setCollectionMonitoredMutation,
  queueBestReleaseMutation,
  queueExistingMutation,
  setEpisodeMonitoredMutation,
  setTitleMonitoredMutation,
  triggerTitleWantedSearchMutation,
  triggerSeasonWantedSearchMutation,
  updateTitleMutation,
} from "@/lib/graphql/mutations";
import type { DownloadQueueItem } from "@/lib/types/download-queue";
import type { Release } from "@/lib/types";
import { DEFAULT_SERIES_LIBRARY_PATH } from "@/lib/constants/settings";
import { qualityProfileSettingsToEntries } from "@/lib/utils/quality-profiles";
import { releaseQueueScopeInput } from "@/lib/utils/release-queue-scope";
import { useClient } from "urql";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { handleFixTitleMatchComplete as applyFixTitleMatchCompletion } from "@/lib/fix-title-match";
import { useTitleDownloadQueue } from "@/lib/hooks/use-title-download-queue";
import { useTitleOverviewReactiveRefresh } from "@/lib/hooks/use-title-overview-reactive-refresh";
import {
  createEmptyTitleOverviewDownloadFeedbackSnapshot,
  fetchTitleOverviewDownloadFeedbackSnapshot,
  fetchTitleOverviewNativeSnapshot,
} from "@/lib/title-overview-loader";
import { SeriesOverviewView } from "@/components/views/series-overview-view";
import { ManualImportDialog } from "@/components/dialogs/manual-import-dialog";
import { FixTitleMatchDialog } from "@/components/dialogs/fix-title-match-dialog";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { useDownloadConflictConfirmation } from "@/components/common/download-conflict-confirmation";
import { DeletePreviewSummary } from "@/components/common/delete-preview-summary";
import { Checkbox } from "@/components/ui/checkbox";
import type { OverviewTitleTarget } from "@/components/root/types";
import type { TitleOptionUpdates } from "@/lib/types/title-options";
import { useDeletePreview } from "@/lib/hooks/use-delete-preview";
import {
  assertNoReplaceConflict,
  retryWithReplaceOnConflict,
} from "@/lib/utils/download-conflicts";
import type {
  TitleOverviewDownloadFeedbackSnapshot,
  TitleOverviewNativeSnapshot,
} from "@/lib/title-overview-loader";
import type { SubtitleDownloadRecord } from "@/lib/types/subtitles";

export type TitleDetail = {
  id: string;
  name: string;
  facet: string;
  monitored: boolean;
  tags: string[];
  externalIds: { source: string; value: string }[];
  year: number | null;
  overview: string | null;
  posterUrl: string | null;
  posterSourceUrl: string | null;
  bannerUrl: string | null;
  backgroundUrl: string | null;
  sortTitle: string | null;
  slug: string | null;
  imdbId: string | null;
  runtimeMinutes: number | null;
  genres: string[];
  contentStatus: string | null;
  language: string | null;
  firstAired: string | null;
  network: string | null;
  studio: string | null;
  country: string | null;
  aliases: string[];
  metadataLanguage: string | null;
  metadataFetchedAt: string | null;
  requiredAudioLanguagesOverride?: string[] | null;
  effectiveRequiredAudioLanguages?: string[];
  inheritsRequiredAudioLanguages?: boolean;
  qualityProfileId?: string | null;
  rootFolderPath?: string | null;
  monitorType?: string | null;
  useSeasonFolders?: boolean | null;
  monitorSpecials?: boolean | null;
  interSeasonMovies?: boolean | null;
  fillerPolicy?: string | null;
  recapPolicy?: string | null;
  createdAt: string;
};

export type TitleCollection = {
  id: string;
  titleId: string;
  collectionType: string;
  collectionIndex: string;
  label: string | null;
  orderedPath: string | null;
  narrativeOrder: string | null;
  fileSizeBytes: number | null;
  firstEpisodeNumber: string | null;
  lastEpisodeNumber: string | null;
  interstitialMovie: InterstitialMovieMetadata | null;
  specialsMovies: InterstitialMovieMetadata[];
  interstitialSeasonEpisode: string | null;
  monitored: boolean;
  episodes?: CollectionEpisode[];
  createdAt: string;
};

export type InterstitialMovieMetadata = {
  tvdbId: string;
  name: string;
  slug: string;
  year: number | null;
  contentStatus: string;
  overview: string;
  posterUrl: string;
  language: string;
  runtimeMinutes: number;
  sortTitle: string;
  imdbId: string;
  genres: string[];
  studio: string;
  digitalReleaseDate: string | null;
  associationConfidence: string | null;
  continuityStatus: string | null;
  movieForm: string | null;
  confidence: string | null;
  signalSummary: string | null;
  placement: string | null;
  movieTmdbId: string | null;
  movieMalId: string | null;
};

import type { TitleHistoryEvent } from "@/lib/types";
export type { TitleHistoryEvent };

export type TitleReleaseBlocklistEntry = {
  sourceHint: string | null;
  sourceTitle: string | null;
  errorMessage: string | null;
  attemptedAt: string;
};

export type CollectionEpisode = {
  id: string;
  titleId: string;
  collectionId: string | null;
  episodeType: string;
  episodeNumber: string | null;
  seasonNumber: string | null;
  episodeLabel: string | null;
  title: string | null;
  overview: string | null;
  airDate: string | null;
  durationSeconds: number | null;
  hasMultiAudio: boolean;
  hasSubtitle: boolean;
  isFiller: boolean;
  isRecap: boolean;
  absoluteNumber: string | null;
  monitored: boolean;
  createdAt: string;
};

export type EpisodeMediaFile = {
  id: string;
  titleId: string;
  episodeId: string | null;
  filePath: string;
  sizeBytes: string;
  qualityLabel: string | null;
  scanStatus: string;
  createdAt: string;
  videoCodec: string | null;
  videoWidth: number | null;
  videoHeight: number | null;
  videoBitrateKbps: number | null;
  videoBitDepth: number | null;
  videoHdrFormat: string | null;
  videoFrameRate: string | null;
  videoProfile: string | null;
  audioCodec: string | null;
  audioChannels: number | null;
  audioBitrateKbps: number | null;
  audioLanguages: string[];
  audioStreams: { codec: string | null; channels: number | null; language: string | null; bitrateKbps: number | null }[];
  subtitleLanguages: string[];
  subtitleCodecs: string[];
  subtitleStreams: { codec: string | null; language: string | null; name: string | null; forced: boolean; default: boolean }[];
  hasMultiaudio: boolean;
  durationSeconds: number | null;
  numChapters: number | null;
  containerFormat: string | null;
  sceneName: string | null;
  releaseGroup: string | null;
  sourceType: string | null;
  resolution: string | null;
  videoCodecParsed: string | null;
  audioCodecParsed: string | null;
  acquisitionScore: number | null;
  scoringLog: string | null;
  indexerSource: string | null;
  grabbedReleaseTitle: string | null;
  grabbedAt: string | null;
  edition: string | null;
  originalFilePath: string | null;
  releaseHash: string | null;
};

type SeriesOverviewSnapshotTitle = TitleDetail & {
  collections?: TitleCollection[];
  mediaFiles?: EpisodeMediaFile[];
};

type SeriesOverviewContainerProps = {
  titleId: string;
  onTitleNotFound?: () => void;
  onBackToList?: () => void;
  onTitleResolved?: (title: OverviewTitleTarget) => void;
  initialEpisodeId?: string | null;
};

function groupMediaFilesByEpisode(
  files: EpisodeMediaFile[],
): Record<string, EpisodeMediaFile[]> {
  const grouped: Record<string, EpisodeMediaFile[]> = {};
  for (const file of files) {
    const key = file.episodeId ?? "__unlinked__";
    (grouped[key] ??= []).push(file);
  }
  return grouped;
}

export const SeriesOverviewContainer = React.memo(function SeriesOverviewContainer({
  titleId,
  onTitleNotFound,
  onBackToList,
  onTitleResolved,
  initialEpisodeId,
}: SeriesOverviewContainerProps) {
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const client = useClient();
  const { confirmReplaceConflict, replaceConflictDialog } =
    useDownloadConflictConfirmation();
  const [title, setTitle] = React.useState<TitleDetail | null>(null);
  const [collections, setCollections] = React.useState<TitleCollection[]>([]);
  const [events, setEvents] = React.useState<TitleHistoryEvent[]>([]);
  const [releaseBlocklistEntries, setReleaseBlocklistEntries] = React.useState<
    TitleReleaseBlocklistEntry[]
  >([]);
  const [loading, setLoading] = React.useState(true);
  const [episodesByCollection, setEpisodesByCollection] = React.useState<
    Record<string, CollectionEpisode[]>
  >({});
  const [qualityProfiles, setQualityProfiles] = React.useState<{ id: string; name: string }[]>([]);
  const [defaultRootFolder, setDefaultRootFolder] = React.useState(DEFAULT_SERIES_LIBRARY_PATH);
  const [rootFolders, setRootFolders] = React.useState<{ path: string; isDefault: boolean }[]>([]);
  const [mediaFilesByEpisode, setMediaFilesByEpisode] = React.useState<
    Record<string, EpisodeMediaFile[]>
  >({});
  const [downloadQueueSeed, setDownloadQueueSeed] = React.useState<DownloadQueueItem[]>([]);
  const [downloadFeedbackSettled, setDownloadFeedbackSettled] = React.useState(false);
  const [subtitleDownloads, setSubtitleDownloads] = React.useState<
    SubtitleDownloadRecord[]
  >([]);
  const [completedDownloads, setCompletedDownloads] = React.useState<DownloadQueueItem[]>([]);
  const [manualImportItem, setManualImportItem] = React.useState<DownloadQueueItem | null>(null);
  const [hydratingFromActivity, setHydratingFromActivity] = React.useState(false);
  const [hasDownloadClients, setHasDownloadClients] = React.useState(true);
  const [downloadFeedbackWarning, setDownloadFeedbackWarning] = React.useState<string | null>(null);
  const [showSearchPrerequisiteNotice, setShowSearchPrerequisiteNotice] =
    React.useState(false);
  const [monitoredUpdating, setMonitoredUpdating] = React.useState(false);
  const [searchMonitoredLoading, setSearchMonitoredLoading] = React.useState(false);
  const [refreshAndScanLoading, setRefreshAndScanLoading] = React.useState(false);
  const [deleteDialogOpen, setDeleteDialogOpen] = React.useState(false);
  const [deleteFilesOnDisk, setDeleteFilesOnDisk] = React.useState(false);
  const [deleteLoading, setDeleteLoading] = React.useState(false);
  const [titleDeleteTypedConfirmation, setTitleDeleteTypedConfirmation] =
    React.useState("");
  const [mediaFileToDelete, setMediaFileToDelete] =
    React.useState<EpisodeMediaFile | null>(null);
  const [mediaFileDeleteLoading, setMediaFileDeleteLoading] = React.useState(false);
  const [mediaFileDeleteTypedConfirmation, setMediaFileDeleteTypedConfirmation] =
    React.useState("");
  const [fixMatchOpen, setFixMatchOpen] = React.useState(false);
  const [titleLookupAttempted, setTitleLookupAttempted] = React.useState(false);
  const [titleLookupFailed, setTitleLookupFailed] = React.useState(false);
  const currentTitleIdRef = React.useRef<string | null>(titleId ?? null);
  React.useEffect(() => {
    currentTitleIdRef.current = titleId ?? null;
  }, [titleId]);
  const lastShownDownloadFeedbackWarningRef = React.useRef<string | null>(null);
  const downloadQueueItems = useTitleDownloadQueue({
    enabled: Boolean(titleId) && hasDownloadClients && downloadFeedbackSettled,
    titleId,
    initialItems: downloadQueueSeed,
  });

  const titleDeletePreviewVariables = React.useMemo(
    () =>
      title && deleteDialogOpen && deleteFilesOnDisk
        ? { input: { titleId: title.id } }
        : null,
    [deleteDialogOpen, deleteFilesOnDisk, title],
  );
  const {
    preview: titleDeletePreview,
    loading: titleDeletePreviewLoading,
    error: titleDeletePreviewError,
  } = useDeletePreview(
    deleteTitlePreviewQuery,
    "deleteTitlePreview",
    titleDeletePreviewVariables,
    deleteDialogOpen && title !== null && deleteFilesOnDisk,
  );
  const mediaFileDeletePreviewVariables = React.useMemo(
    () =>
      mediaFileToDelete ? { input: { fileId: mediaFileToDelete.id } } : null,
    [mediaFileToDelete],
  );
  const {
    preview: mediaFileDeletePreview,
    loading: mediaFileDeletePreviewLoading,
    error: mediaFileDeletePreviewError,
  } = useDeletePreview(
    deleteMediaFilePreviewQuery,
    "deleteMediaFilePreview",
    mediaFileDeletePreviewVariables,
    mediaFileToDelete !== null,
  );

  const applyDownloadFeedbackSnapshot = React.useCallback(
    (snapshot: TitleOverviewDownloadFeedbackSnapshot) => {
      setDownloadQueueSeed(snapshot.downloadQueueItems);
      setCompletedDownloads(snapshot.completedDownloadQueueItems);
      setDownloadFeedbackWarning(snapshot.downloadFeedbackWarning);
    },
    [],
  );

  const applyNativeTitleDetailSnapshot = React.useCallback(
    (
      snapshot: TitleOverviewNativeSnapshot<
        SeriesOverviewSnapshotTitle,
        unknown,
        TitleHistoryEvent,
        TitleReleaseBlocklistEntry,
        SubtitleDownloadRecord
      >,
    ) => {
      const nextTitle = snapshot.title;
      const nextCollections = nextTitle?.collections ?? [];
      const nextMediaFiles = nextTitle?.mediaFiles ?? [];
      setTitle(nextTitle);
      if (nextTitle) {
        onTitleResolved?.({ id: nextTitle.id, slug: nextTitle.slug });
      }
      setCollections(nextCollections);
      setEpisodesByCollection(
        Object.fromEntries(
          nextCollections.map((collection: TitleCollection) => [
            collection.id,
            collection.episodes ?? [],
          ]),
        ),
      );
      setMediaFilesByEpisode(groupMediaFilesByEpisode(nextMediaFiles));
      setEvents(snapshot.titleEvents);
      setReleaseBlocklistEntries(snapshot.titleReleaseBlocklist);
      setSubtitleDownloads(snapshot.subtitleDownloads);
      setHasDownloadClients(snapshot.hasDownloadClients);
      if (!nextTitle || !snapshot.hasDownloadClients) {
        applyDownloadFeedbackSnapshot(createEmptyTitleOverviewDownloadFeedbackSnapshot());
        setDownloadFeedbackSettled(true);
      }
    },
    [applyDownloadFeedbackSnapshot, onTitleResolved],
  );

  React.useEffect(() => {
    if (hasDownloadClients) {
      setShowSearchPrerequisiteNotice(false);
    }
  }, [hasDownloadClients]);

  React.useEffect(() => {
    if (downloadFeedbackWarning === null) {
      lastShownDownloadFeedbackWarningRef.current = null;
      return;
    }

    if (lastShownDownloadFeedbackWarningRef.current === downloadFeedbackWarning) {
      return;
    }

    lastShownDownloadFeedbackWarningRef.current = downloadFeedbackWarning;
    setGlobalStatus(downloadFeedbackWarning);
  }, [downloadFeedbackWarning, setGlobalStatus]);

  const refreshDownloadFeedback = React.useCallback(async () => {
    if (!titleId) {
      return;
    }

    const requestedTitleId = titleId;
    try {
      const snapshot = await fetchTitleOverviewDownloadFeedbackSnapshot(
        client,
        requestedTitleId,
      );
      if (currentTitleIdRef.current !== requestedTitleId) {
        return;
      }
      applyDownloadFeedbackSnapshot(snapshot);
    } catch (error: unknown) {
      if (currentTitleIdRef.current !== requestedTitleId) {
        return;
      }
      setGlobalStatus(
        error instanceof Error ? error.message : t("status.apiError"),
      );
    } finally {
      if (currentTitleIdRef.current === requestedTitleId) {
        setDownloadFeedbackSettled(true);
      }
    }
  }, [applyDownloadFeedbackSnapshot, client, setGlobalStatus, t, titleId]);

  const refreshTitleDetail = React.useCallback(async () => {
    if (!titleId) {
      return;
    }

    const requestedTitleId = titleId;
    const snapshot = await fetchTitleOverviewNativeSnapshot<
      SeriesOverviewSnapshotTitle,
      unknown,
      TitleHistoryEvent,
      TitleReleaseBlocklistEntry,
      SubtitleDownloadRecord
    >(client, requestedTitleId, 300);
    if (currentTitleIdRef.current !== requestedTitleId) {
      return;
    }
    applyNativeTitleDetailSnapshot(snapshot);
    if (!snapshot.title || !snapshot.hasDownloadClients) {
      return;
    }
    void refreshDownloadFeedback();
  }, [applyNativeTitleDetailSnapshot, client, refreshDownloadFeedback, titleId]);

  React.useEffect(() => {
    let cancelled = false;

    if (!titleId) {
      setTitle(null);
      setCollections([]);
      setEvents([]);
      setReleaseBlocklistEntries([]);
      setEpisodesByCollection({});
      setMediaFilesByEpisode({});
      setDownloadQueueSeed([]);
      setDownloadFeedbackSettled(false);
      setSubtitleDownloads([]);
      setCompletedDownloads([]);
      setManualImportItem(null);
      setHydratingFromActivity(false);
      setHasDownloadClients(true);
      setDownloadFeedbackWarning(null);
      setShowSearchPrerequisiteNotice(false);
      setTitleLookupAttempted(false);
      setTitleLookupFailed(false);
      setLoading(false);
      return () => {
        cancelled = true;
      };
    }

    setTitleLookupAttempted(false);
    setTitleLookupFailed(false);
    setDownloadQueueSeed([]);
    setCompletedDownloads([]);
    setDownloadFeedbackWarning(null);
    setDownloadFeedbackSettled(false);
    setShowSearchPrerequisiteNotice(false);
    setLoading(true);
    refreshTitleDetail()
      .catch((error: unknown) => {
        if (!cancelled) {
          setTitleLookupFailed(true);
          setGlobalStatus(
            error instanceof Error ? error.message : t("status.apiError"),
          );
        }
      })
      .finally(() => {
        if (!cancelled) {
          setTitleLookupAttempted(true);
          setLoading(false);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [refreshTitleDetail, setGlobalStatus, t, titleId]);

  React.useEffect(() => {
    if (titleId && titleLookupAttempted && !loading && !titleLookupFailed && !title) {
      onTitleNotFound?.();
    }
  }, [loading, onTitleNotFound, title, titleId, titleLookupAttempted, titleLookupFailed]);

  React.useEffect(() => {
    setHydratingFromActivity(false);
  }, [titleId]);

  const inferredHydrating = React.useMemo(() => {
    if (!title) {
      return false;
    }

    const metadataFetchedAt = title.metadataFetchedAt ? Date.parse(title.metadataFetchedAt) : NaN;
    const metadataJustHydrated =
      Number.isFinite(metadataFetchedAt) &&
      Date.now() - metadataFetchedAt < 30_000;

    return title.metadataFetchedAt === null || (collections.length === 0 && metadataJustHydrated);
  }, [title, collections.length]);

  const hydrating = inferredHydrating || hydratingFromActivity;

  React.useEffect(() => {
    if (!inferredHydrating && hydratingFromActivity) {
      setHydratingFromActivity(false);
    }
  }, [inferredHydrating, hydratingFromActivity]);

  // Fetch quality profile catalog and default root folder
  React.useEffect(() => {
    let cancelled = false;
    const load = async () => {
      try {
        const { data, error } = await client.query(seriesOverviewSettingsInitQuery, {
          scope: title?.facet === "anime" ? "anime" : "series",
        }).toPromise();
        if (error) throw error;
        if (cancelled) return;
        setQualityProfiles(
          qualityProfileSettingsToEntries(data.qualityProfileSettings).map((profile) => ({
            id: profile.id,
            name: profile.name,
          })),
        );
        const folder = (data.mediaSettings?.libraryPath ?? "").trim();
        if (folder) setDefaultRootFolder(folder);
        if (Array.isArray(data.mediaSettings?.rootFolders)) {
          setRootFolders(data.mediaSettings.rootFolders);
        }
      } catch {
        // Settings fetch is best-effort
      }
    };
    void load();
    return () => { cancelled = true; };
  }, [client, title?.facet]);

  const handleUpdateTitleOptions = React.useCallback(
    async (options: TitleOptionUpdates) => {
      const { error } = await client.mutation(updateTitleMutation, {
        input: { titleId, options },
      }).toPromise();
      if (error) throw error;
      await refreshTitleDetail();
    },
    [titleId, client, refreshTitleDetail],
  );

  const handleSetCollectionMonitored = React.useCallback(
    async (collectionId: string, monitored: boolean) => {
      const { error } = await client.mutation(
        setCollectionMonitoredMutation,
        { input: { collectionId, monitored } },
      ).toPromise();
      if (error) throw error;
      await refreshTitleDetail();
    },
    [client, refreshTitleDetail],
  );

  const handleSetEpisodeMonitored = React.useCallback(
    async (episodeId: string, monitored: boolean) => {
      const { error } = await client.mutation(
        setEpisodeMonitoredMutation,
        { input: { episodeId, monitored } },
      ).toPromise();
      if (error) throw error;
      await refreshTitleDetail();
    },
    [client, refreshTitleDetail],
  );

  const handleSetTitleMonitored = React.useCallback(
    async (monitored: boolean) => {
      if (!title) return;
      setMonitoredUpdating(true);
      try {
        const { error } = await client.mutation(setTitleMonitoredMutation, {
          input: { titleId: title.id, monitored },
        }).toPromise();
        if (error) throw error;
        setGlobalStatus(
          monitored
            ? t("status.titleMonitoringEnabled")
            : t("status.titleMonitoringDisabled"),
        );
        await refreshTitleDetail();
      } catch (error: unknown) {
        setGlobalStatus(error instanceof Error ? error.message : t("status.apiError"));
      } finally {
        setMonitoredUpdating(false);
      }
    },
    [client, refreshTitleDetail, setGlobalStatus, t, title],
  );

  const handleSearchMonitored = React.useCallback(async () => {
    if (!title) return;
    if (!hasDownloadClients) {
      setShowSearchPrerequisiteNotice(true);
      return;
    }

    setSearchMonitoredLoading(true);
    try {
      const { data, error } = await client.mutation(triggerTitleWantedSearchMutation, {
        input: { titleId: title.id },
      }).toPromise();
      if (error) throw error;

      const queued = data?.triggerTitleWantedSearch?.queuedCount ?? 0;
      const skipped = data?.triggerTitleWantedSearch?.skippedInProgressCount ?? 0;
      const baseStatus =
        queued > 0
          ? t("status.searchMonitoredQueued", { count: queued })
          : t("status.searchMonitoredEmpty");
      setGlobalStatus(
        skipped > 0
          ? `${baseStatus} ${t("status.searchSkippedInProgress", { count: skipped })}`
          : baseStatus,
      );
    } catch (error: unknown) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.apiError"));
    } finally {
      setSearchMonitoredLoading(false);
    }
  }, [client, hasDownloadClients, setGlobalStatus, t, title]);

  const handleDeleteMediaFile = React.useCallback((fileId: string) => {
    const nextFile =
      Object.values(mediaFilesByEpisode)
        .flat()
        .find((candidate) => candidate.id === fileId) ?? null;
    setMediaFileToDelete(nextFile);
    setMediaFileDeleteTypedConfirmation("");
  }, [mediaFilesByEpisode]);

  const handleRefreshAndScan = React.useCallback(async () => {
    if (!title) return;

    setRefreshAndScanLoading(true);
    try {
      const { data, error } = await client.mutation(scanTitleLibraryMutation, {
        input: { titleId: title.id },
      }).toPromise();
      if (error) throw error;

      const summary = data?.scanTitleLibrary;
      setGlobalStatus(
        t("status.titleScanSuccess", {
          imported: summary?.imported ?? 0,
          skipped: summary?.skipped ?? 0,
          unmatched: summary?.unmatched ?? 0,
        }),
      );
      await refreshTitleDetail();
    } catch (error: unknown) {
      setGlobalStatus(error instanceof Error ? error.message : t("settings.libraryScanFailed"));
    } finally {
      setRefreshAndScanLoading(false);
    }
  }, [client, refreshTitleDetail, setGlobalStatus, t, title]);

  const handleRequestDeleteTitle = React.useCallback(() => {
    setDeleteFilesOnDisk(false);
    setTitleDeleteTypedConfirmation("");
    setDeleteDialogOpen(true);
  }, []);

  const handleFixMatchComplete = React.useCallback(
    async (warnings: string[]) => {
      await applyFixTitleMatchCompletion({
        warnings,
        refreshTitleDetail,
        setGlobalStatus,
        t,
        titleName: title?.name,
      });
    },
    [refreshTitleDetail, setGlobalStatus, t, title?.name],
  );

  const handleCancelDeleteTitle = React.useCallback(() => {
    if (deleteLoading) return;
    setDeleteDialogOpen(false);
    setDeleteFilesOnDisk(false);
    setTitleDeleteTypedConfirmation("");
  }, [deleteLoading]);

  React.useEffect(() => {
    if (!deleteFilesOnDisk) {
      setTitleDeleteTypedConfirmation("");
    }
  }, [deleteFilesOnDisk]);

  const handleConfirmDeleteTitle = React.useCallback(async () => {
    if (!title) return;
    setDeleteLoading(true);
    try {
      const payload: {
        titleId: string;
        deleteFilesOnDisk?: boolean;
        previewFingerprint?: string;
        typedConfirmation?: string;
      } = {
        titleId: title.id,
      };
      if (deleteFilesOnDisk) {
        if (!titleDeletePreview) {
          throw new Error("Delete preview is not ready yet.");
        }
        payload.deleteFilesOnDisk = true;
        payload.previewFingerprint = titleDeletePreview.fingerprint;
        if (titleDeleteTypedConfirmation.trim()) {
          payload.typedConfirmation = titleDeleteTypedConfirmation.trim();
        }
      }

      const { error } = await client.mutation(deleteTitleMutation, {
        input: payload,
      }).toPromise();
      if (error) throw error;

      setGlobalStatus(t("status.titleDeleted", { name: title.name }));
      setDeleteDialogOpen(false);
      setDeleteFilesOnDisk(false);

      if (onBackToList) {
        onBackToList();
        return;
      }
      onTitleNotFound?.();
    } catch (error: unknown) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToDelete"));
    } finally {
      setDeleteLoading(false);
    }
  }, [
    client,
    deleteFilesOnDisk,
    onBackToList,
    onTitleNotFound,
    titleDeletePreview,
    titleDeleteTypedConfirmation,
    setGlobalStatus,
    t,
    title,
  ]);

  const handleCancelDeleteMediaFile = React.useCallback(() => {
    if (mediaFileDeleteLoading) return;
    setMediaFileToDelete(null);
    setMediaFileDeleteTypedConfirmation("");
  }, [mediaFileDeleteLoading]);

  const handleConfirmDeleteMediaFile = React.useCallback(async () => {
    if (!mediaFileToDelete || !mediaFileDeletePreview) return;
    setMediaFileDeleteLoading(true);
    try {
      const { error } = await client.mutation(deleteMediaFileMutation, {
        input: {
          fileId: mediaFileToDelete.id,
          deleteFromDisk: true,
          previewFingerprint: mediaFileDeletePreview.fingerprint,
          typedConfirmation: mediaFileDeleteTypedConfirmation.trim() || undefined,
        },
      }).toPromise();
      if (error) throw error;
      await refreshTitleDetail();
      setMediaFileToDelete(null);
      setMediaFileDeleteTypedConfirmation("");
    } catch (error: unknown) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.apiError"));
    } finally {
      setMediaFileDeleteLoading(false);
    }
  }, [
    client,
    mediaFileDeletePreview,
    mediaFileDeleteTypedConfirmation,
    mediaFileToDelete,
    refreshTitleDetail,
    setGlobalStatus,
    t,
  ]);

  const deleteTitleConfirmDisabled =
    deleteFilesOnDisk &&
    (titleDeletePreviewLoading ||
      !!titleDeletePreviewError ||
      !titleDeletePreview ||
      (titleDeletePreview.requiresTypedConfirmation &&
        titleDeleteTypedConfirmation.trim() !== "DELETE"));
  const deleteMediaFileConfirmDisabled =
    mediaFileDeletePreviewLoading ||
    !!mediaFileDeletePreviewError ||
    !mediaFileDeletePreview ||
    (mediaFileDeletePreview.requiresTypedConfirmation &&
      mediaFileDeleteTypedConfirmation.trim() !== "DELETE");

  const handleAutoSearchEpisode = React.useCallback(
    async (episode: CollectionEpisode) => {
      if (!title) return;

      try {
        const input = {
          titleId: title.id,
          scope: { episode: episode.id },
        };
        const payload = await retryWithReplaceOnConflict(
          input,
          async (nextInput) => {
            const { data, error } = await client.mutation(queueBestReleaseMutation, {
              input: nextInput,
            }).toPromise();
            if (error) throw error;
            return data?.queueBestRelease;
          },
          "A download is already in progress for this episode.",
          confirmReplaceConflict,
        );
        assertNoReplaceConflict(payload, "A download is already in progress for this episode.");
        setGlobalStatus(t("status.queuedLatest", { name: title.name }));
        await refreshTitleDetail();
      } catch (error: unknown) {
        setGlobalStatus(error instanceof Error ? error.message : t("status.queueFailed"));
      }
    },
    [refreshTitleDetail, client, confirmReplaceConflict, title, t, setGlobalStatus],
  );

  const handleAutoSearchInterstitialMovie = React.useCallback(
    async (collection: TitleCollection) => {
      if (!title || !collection.interstitialMovie) return;
      const input = {
        titleId: title.id,
        scope: { collection: collection.id },
      };
      const payload = await retryWithReplaceOnConflict(
        input,
        async (nextInput) => {
          const { data, error } = await client.mutation(queueBestReleaseMutation, {
            input: nextInput,
          }).toPromise();
          if (error) throw error;
          return data?.queueBestRelease;
        },
        "A download is already in progress for this collection.",
        confirmReplaceConflict,
      );
      assertNoReplaceConflict(payload, "A download is already in progress for this collection.");
      setGlobalStatus(t("status.queuedLatest", { name: collection.interstitialMovie.name }));
      await refreshTitleDetail();
    },
    [refreshTitleDetail, client, confirmReplaceConflict, title, t, setGlobalStatus],
  );

  const [seasonSearchResultsByCollection] = React.useState<
    Record<string, Release[]>
  >({});
  const [seasonSearchLoadingByCollection, setSeasonSearchLoadingByCollection] = React.useState<
    Record<string, boolean>
  >({});

  const handleRunSeasonSearch = React.useCallback(
    async (collection: TitleCollection) => {
      if (!title) return;
      const seasonNum = parseInt(collection.collectionIndex?.trim().replace(/\D+/g, "") || "0", 10);
      if (!seasonNum) return;

      setSeasonSearchLoadingByCollection((prev) => ({ ...prev, [collection.id]: true }));
      try {
        const { data, error } = await client
          .mutation(triggerSeasonWantedSearchMutation, {
            input: { titleId: title.id, seasonNumber: seasonNum },
          })
          .toPromise();
        if (error) throw error;
        const queued = data?.triggerSeasonWantedSearch?.queuedCount ?? 0;
        const skipped = data?.triggerSeasonWantedSearch?.skippedInProgressCount ?? 0;
        const baseStatus =
          queued > 0
            ? t("status.searchMonitoredQueued", { count: queued })
            : t("status.searchMonitoredEmpty");
        setGlobalStatus(
          skipped > 0
            ? `${baseStatus} ${t("status.searchSkippedInProgress", { count: skipped })}`
            : baseStatus,
        );
      } catch (error: unknown) {
        setGlobalStatus(error instanceof Error ? error.message : t("status.apiError"));
      } finally {
        setSeasonSearchLoadingByCollection((prev) => ({ ...prev, [collection.id]: false }));
      }
    },
    [client, setGlobalStatus, t, title],
  );

  const handleQueueFromSeasonSearch = React.useCallback(
    async (collection: TitleCollection, release: Release) => {
      if (!title) return;
      if (!release.candidateToken) {
        setGlobalStatus(t("status.releaseMissingCandidateToken"));
        return;
      }
      try {
        const input = {
          titleId: title.id,
          scope: releaseQueueScopeInput(release, { collection: collection.id }),
          candidateToken: release.candidateToken,
        };
        const payload = await retryWithReplaceOnConflict(
          input,
          async (nextInput) => {
            const { data, error } = await client
              .mutation(queueExistingMutation, { input: nextInput })
              .toPromise();
            if (error) throw error;
            return data?.queueExistingTitleDownload;
          },
          "A download is already in progress for this collection.",
          confirmReplaceConflict,
        );
        assertNoReplaceConflict(payload, "A download is already in progress for this collection.");
        setGlobalStatus(t("status.queuedLatest", { name: title.name }));
        await refreshTitleDetail();
      } catch (error: unknown) {
        setGlobalStatus(error instanceof Error ? error.message : t("status.queueFailed"));
      }
    },
    [client, confirmReplaceConflict, title, refreshTitleDetail, setGlobalStatus, t],
  );

  const handleOpenManualImport = React.useCallback(
    (item: DownloadQueueItem) => {
      setManualImportItem(item);
    },
    [],
  );

  const handleManualImportComplete = React.useCallback(async () => {
    await refreshTitleDetail();
  }, [refreshTitleDetail]);

  const IMPORT_KINDS = React.useMemo(
    () =>
      new Set([
        "movie_downloaded",
        "series_episode_imported",
        "file_analyzed",
        "file_upgraded",
        "subtitle_downloaded",
      ]),
    [],
  );

  useTitleOverviewReactiveRefresh({
    titleId,
    blocklistLimit: 300,
    applyNativeSnapshot: applyNativeTitleDetailSnapshot,
    applyDownloadFeedbackSnapshot,
    importKinds: IMPORT_KINDS,
    downloadFeedbackEnabled: hasDownloadClients,
    pause: !titleId,
    onHydrationStarted() {
      setHydratingFromActivity(true);
    },
    onHydrationCompleted() {
      setHydratingFromActivity(false);
    },
    onHydrationFailed() {
      setHydratingFromActivity(false);
    },
  });

  return (
    <>
      <SeriesOverviewView
        loading={loading}
        hydrating={hydrating}
        title={title}
        collections={collections}
        events={events}
        episodesByCollection={episodesByCollection}
        mediaFilesByEpisode={mediaFilesByEpisode}
        subtitleDownloads={subtitleDownloads}
        onRefreshSubtitles={refreshTitleDetail}
        releaseBlocklistEntries={releaseBlocklistEntries}
        onTitleChanged={refreshTitleDetail}
        onBackToList={onBackToList}
        onSetCollectionMonitored={handleSetCollectionMonitored}
        onSetEpisodeMonitored={handleSetEpisodeMonitored}
        onSetTitleMonitored={handleSetTitleMonitored}
        onSearchMonitored={handleSearchMonitored}
        onAutoSearchEpisode={handleAutoSearchEpisode}
        onAutoSearchInterstitialMovie={handleAutoSearchInterstitialMovie}
        downloadQueueItems={downloadQueueItems}
        hasDownloadClients={hasDownloadClients}
        showSearchPrerequisiteNotice={showSearchPrerequisiteNotice}
        qualityProfiles={qualityProfiles}
        defaultRootFolder={defaultRootFolder}
        rootFolders={rootFolders}
        onUpdateTitleOptions={handleUpdateTitleOptions}
        completedDownloads={completedDownloads}
        onOpenManualImport={handleOpenManualImport}
        initialEpisodeId={initialEpisodeId}
        seasonSearchResultsByCollection={seasonSearchResultsByCollection}
        seasonSearchLoadingByCollection={seasonSearchLoadingByCollection}
        onRunSeasonSearch={handleRunSeasonSearch}
        onQueueFromSeasonSearch={handleQueueFromSeasonSearch}
        monitoredUpdating={monitoredUpdating}
        searchMonitoredLoading={searchMonitoredLoading}
        onRefreshAndScan={handleRefreshAndScan}
        refreshAndScanLoading={refreshAndScanLoading}
        onRequestDeleteTitle={handleRequestDeleteTitle}
        deleteLoading={deleteLoading}
        onDeleteFile={handleDeleteMediaFile}
        onOpenFixMatch={() => setFixMatchOpen(true)}
      />
      <FixTitleMatchDialog
        open={fixMatchOpen}
        onOpenChange={setFixMatchOpen}
        title={title}
        onFixed={handleFixMatchComplete}
      />
      <ConfirmDialog
        open={deleteDialogOpen && title !== null}
        title={t("label.delete")}
        description={
          title
            ? t("status.deleteCatalogConfirm", { name: title.name })
            : t("label.delete")
        }
        confirmLabel={t("label.delete")}
        cancelLabel={t("label.cancel")}
        isBusy={deleteLoading}
        confirmDisabled={deleteTitleConfirmDisabled}
        onConfirm={handleConfirmDeleteTitle}
        onCancel={handleCancelDeleteTitle}
      >
        <div className="space-y-3">
          <label className="flex items-center gap-2">
            <Checkbox
              checked={deleteFilesOnDisk}
              onCheckedChange={(checked) => setDeleteFilesOnDisk(checked === true)}
              disabled={deleteLoading}
            />
            <span className="text-sm text-muted-foreground">{t("title.deleteFilesOnDisk")}</span>
          </label>
          {deleteFilesOnDisk ? (
            <DeletePreviewSummary
              preview={titleDeletePreview}
              loading={titleDeletePreviewLoading}
              error={titleDeletePreviewError}
              typedConfirmation={titleDeleteTypedConfirmation}
              onTypedConfirmationChange={setTitleDeleteTypedConfirmation}
            />
          ) : null}
        </div>
      </ConfirmDialog>
      <ConfirmDialog
        open={mediaFileToDelete !== null}
        title={t("mediaFile.delete")}
        description={mediaFileToDelete?.filePath ?? t("mediaFile.delete")}
        confirmLabel={t("label.delete")}
        cancelLabel={t("label.cancel")}
        isBusy={mediaFileDeleteLoading}
        confirmDisabled={deleteMediaFileConfirmDisabled}
        onConfirm={handleConfirmDeleteMediaFile}
        onCancel={handleCancelDeleteMediaFile}
      >
        <DeletePreviewSummary
          preview={mediaFileDeletePreview}
          loading={mediaFileDeletePreviewLoading}
          error={mediaFileDeletePreviewError}
          typedConfirmation={mediaFileDeleteTypedConfirmation}
          onTypedConfirmationChange={setMediaFileDeleteTypedConfirmation}
        />
      </ConfirmDialog>
      {manualImportItem && title && (
        <ManualImportDialog
          open={true}
          onOpenChange={(open) => { if (!open) setManualImportItem(null); }}
          titleId={title.id}
          titleName={title.name}
          clientId={manualImportItem.clientId}
          clientType={manualImportItem.clientType}
          downloadClientItemId={manualImportItem.downloadClientItemId}
          onImportComplete={() => void handleManualImportComplete()}
        />
      )}
      {replaceConflictDialog}
    </>
  );
});
