
import * as React from "react";
import {
  deleteMediaFilePreviewQuery,
  deleteTitlePreviewQuery,
  librariesQuery,
  mediaRenamePreviewQuery,
  movieOverviewSettingsInitQuery,
  searchForTitleQuery,
} from "@/lib/graphql/queries";
import {
  applyMediaRenameMutation,
  clearTitleReleaseBlocklistEntryMutation,
  deleteMediaFileMutation,
  deleteTitleMutation,
  queueExistingMutation,
  scanTitleLibraryMutation,
  setPrimaryMovieFileMutation,
  setTitleMonitoredMutation,
  triggerTitleWantedSearchMutation,
  triggerTitleMismatchRecoverySearchMutation,
  pauseWantedItemMutation,
  resumeWantedItemMutation,
  resetWantedItemMutation,
  updateTitleMutation,
} from "@/lib/graphql/mutations";
import { DEFAULT_MOVIE_LIBRARY_PATH } from "@/lib/constants/settings";
import { userFacingGraphQlErrorMessage } from "@/lib/graphql/error-message";
import { qualityProfileSettingsToEntries } from "@/lib/utils/quality-profiles";
import { releaseQueueScopeInput } from "@/lib/utils/release-queue-scope";
import { useClient } from "urql";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTitleDownloadQueue } from "@/lib/hooks/use-title-download-queue";
import { handleFixTitleMatchComplete as applyFixTitleMatchCompletion } from "@/lib/fix-title-match";
import type { Release, TitleAcquisitionDiagnostics, WantedItem } from "@/lib/types";
import type { CatalogDiscoveryItem } from "@/lib/types/discovery";
import type { TitleRatings } from "@/components/views/title-ratings-strip";
import type { CanonicalMediaTag, LibraryRootRecord } from "@/lib/types/titles";
import type { DownloadQueueItem } from "@/lib/types/download-queue";
import {
  createEmptyTitleOverviewDownloadFeedbackSnapshot,
  fetchTitleOverviewDownloadFeedbackSnapshot,
  fetchTitleOverviewNativeSnapshot,
} from "@/lib/title-overview-loader";
import { MovieOverviewView } from "@/components/views/movie-overview-view";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { useDownloadConflictConfirmation } from "@/components/common/download-conflict-confirmation";
import { DeletePreviewSummary } from "@/components/common/delete-preview-summary";
import { Checkbox } from "@/components/ui/checkbox";
import type { OverviewTitleTarget } from "@/components/root/types";
import type { TitleOptionUpdates } from "@/lib/types/title-options";
import { FixTitleMatchDialog } from "@/components/dialogs/fix-title-match-dialog";
import { useDeletePreview } from "@/lib/hooks/use-delete-preview";
import {
  assertNoReplaceConflict,
  retryWithReplaceOnConflict,
} from "@/lib/utils/download-conflicts";
import { isAbortError, makeAbortableFetch } from "@/lib/graphql/urql-client";
import type {
  TitleOverviewDownloadFeedbackSnapshot,
  TitleOverviewNativeSnapshot,
} from "@/lib/title-overview-loader";
import type { ExternalSubtitleRecord } from "@/lib/types/subtitles";
import { useAuth } from "@/lib/hooks/use-auth";
import {
  LIBRARY_PERMISSIONS,
  hasAnyLibraryPermission,
  hasLibraryPermission,
} from "@/lib/utils/permissions";
import { useTitleMoreLikeThisActions } from "@/lib/hooks/use-title-more-like-this-actions";

export type TitleDetail = {
  id: string;
  name: string;
  facet: string;
  libraryId: string;
  libraryName?: string | null;
  librarySlug?: string | null;
  monitored: boolean;
  tags: string[];
  externalIds: { source: string; value: string }[];
  year: number | null;
  overview: string | null;
  posterUrl: string | null;
  posterSourceUrl: string | null;
  backgroundUrl: string | null;
  sortTitle: string | null;
  slug: string | null;
  imdbId: string | null;
  runtimeMinutes: number | null;
  canonicalTags?: CanonicalMediaTag[];
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
  rootFolderId?: string;
  rootFolderPath?: string;
  monitorType?: string | null;
  useSeasonFolders?: boolean | null;
  monitorSpecials?: boolean | null;
  interSeasonMovies?: boolean | null;
  fillerPolicy?: string | null;
  recapPolicy?: string | null;
  ratings?: TitleRatings | null;
  moreLikeThis?: CatalogDiscoveryItem[];
  createdAt: string;
};

export type TitleCollection = {
  id: string;
  titleId: string;
  collectionType: string;
  collectionIndex: string;
  label: string | null;
  orderedPath: string | null;
  createdAt: string;
};

import type { TitleHistoryEvent } from "@/lib/types";
export type { TitleHistoryEvent };

export type TitleReleaseBlocklistEntry = {
  id: string;
  sourceHint: string | null;
  sourceTitle: string | null;
  errorMessage: string | null;
  attemptedAt: string;
  episodeIds: string[];
};

export type TitleMediaFile = {
  id: string;
  titleId: string;
  episodeId: string | null;
  role: string;
  filePath: string;
  sizeBytes: number;
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

export type MediaRenamePlanItem = {
  collectionId: string | null;
  seriesMovieLinkIds: string[];
  currentPath: string;
  proposedPath: string | null;
  normalizedFilename: string | null;
  collision: boolean;
  reasonCode: string;
  writeAction: string;
  sourceSizeBytes: number | null;
  sourceMtimeUnixMs: string | null;
};

export type MediaRenamePlan = {
  facet: string;
  titleId: string | null;
  template: string;
  collisionPolicy: string;
  missingMetadataPolicy: string;
  fingerprint: string;
  total: number;
  renamable: number;
  noop: number;
  conflicts: number;
  errors: number;
  items: MediaRenamePlanItem[];
};

export type MediaRenameApplyResult = {
  planFingerprint: string;
  total: number;
  applied: number;
  skipped: number;
  failed: number;
};

type MovieOverviewSnapshotTitle = TitleDetail & {
  collections?: TitleCollection[];
  mediaFiles?: TitleMediaFile[];
  wantedItems?: {
    items?: WantedItem[];
  } | null;
};

type MovieOverviewContainerProps = {
  titleId: string;
  onTitleNotFound?: () => void;
  onBackToList?: () => void;
  onTitleResolved?: (title: OverviewTitleTarget) => void;
};

export const MovieOverviewContainer = React.memo(function MovieOverviewContainer({
  titleId,
  onTitleNotFound,
  onBackToList,
  onTitleResolved,
}: MovieOverviewContainerProps) {
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const client = useClient();
  const auth = useAuth();
  const { confirmReplaceConflict, replaceConflictDialog } =
    useDownloadConflictConfirmation();
  const [title, setTitle] = React.useState<TitleDetail | null>(null);
  const canManageTitle = hasLibraryPermission(
    auth.user,
    title?.libraryId,
    LIBRARY_PERMISSIONS.manageTitles,
  );
  const canAddDiscoveryItems = hasAnyLibraryPermission(
    auth.user,
    LIBRARY_PERMISSIONS.manageTitles,
  );
  const canRequestDiscoveryItems = hasAnyLibraryPermission(
    auth.user,
    LIBRARY_PERMISSIONS.request,
  );
  const [collections, setCollections] = React.useState<TitleCollection[]>([]);
  const [events, setEvents] = React.useState<TitleHistoryEvent[]>([]);
  const [blocklistEntries, setBlocklistEntries] = React.useState<
    TitleReleaseBlocklistEntry[]
  >([]);
  const [clearingReleaseBlocklistEntryId, setClearingReleaseBlocklistEntryId] =
    React.useState<string | null>(null);
  const [loading, setLoading] = React.useState(true);

  const [searchResults, setSearchResults] = React.useState<Release[]>([]);
  const [interactiveSearchAttempted, setInteractiveSearchAttempted] = React.useState(false);
  const [searching, setSearching] = React.useState(false);
  const indexerSearchAbortRef = React.useRef<AbortController | null>(null);
  const [renamePlan, setRenamePlan] = React.useState<MediaRenamePlan | null>(null);
  const [renameEnabled, setRenameEnabled] = React.useState(true);
  const [renamePreviewing, setRenamePreviewing] = React.useState(false);
  const [renameApplying, setRenameApplying] = React.useState(false);
  const renamePlanTitleIdRef = React.useRef<string | null>(null);
  const [titleLookupAttempted, setTitleLookupAttempted] = React.useState(false);
  const [titleLookupFailed, setTitleLookupFailed] = React.useState(false);
  const [qualityProfiles, setQualityProfiles] = React.useState<{ id: string; name: string }[]>([]);
  const [defaultRootFolder, setDefaultRootFolder] = React.useState(DEFAULT_MOVIE_LIBRARY_PATH);
  const [rootFolders, setRootFolders] = React.useState<LibraryRootRecord[]>([]);
  const [mediaFiles, setMediaFiles] = React.useState<TitleMediaFile[]>([]);
  const [downloadQueueSeed, setDownloadQueueSeed] = React.useState<DownloadQueueItem[]>([]);
  const [downloadFeedbackSettled, setDownloadFeedbackSettled] = React.useState(false);
  const [subtitleDownloads, setSubtitleDownloads] = React.useState<ExternalSubtitleRecord[]>([]);
  const [wantedItem, setWantedItem] = React.useState<WantedItem | null>(null);
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
    React.useState<TitleMediaFile | null>(null);
  const downloadQueueItems = useTitleDownloadQueue({
    enabled: Boolean(titleId) && hasDownloadClients && downloadFeedbackSettled,
    titleId,
    initialItems: downloadQueueSeed,
  });
  const [mediaFileDeleteLoading, setMediaFileDeleteLoading] = React.useState(false);
  const [primaryMovieFileUpdatingId, setPrimaryMovieFileUpdatingId] =
    React.useState<string | null>(null);
  const [mediaFileDeleteTypedConfirmation, setMediaFileDeleteTypedConfirmation] =
    React.useState("");
  const [fixMatchOpen, setFixMatchOpen] = React.useState(false);
  const currentTitleIdRef = React.useRef<string | null>(titleId ?? null);
  React.useEffect(() => {
    currentTitleIdRef.current = titleId ?? null;
    indexerSearchAbortRef.current?.abort();
    indexerSearchAbortRef.current = null;
    setSearching(false);
  }, [titleId]);
  React.useEffect(() => {
    return () => {
      indexerSearchAbortRef.current?.abort();
      indexerSearchAbortRef.current = null;
    };
  }, []);
  const lastShownDownloadFeedbackWarningRef = React.useRef<string | null>(null);
  const [wantedActionLoading, setWantedActionLoading] = React.useState<
    "pause" | "resume" | "reset" | null
  >(null);
  const titleDeletePreviewVariables = React.useMemo(
    () =>
      title && deleteDialogOpen && deleteFilesOnDisk
        ? { titleId: title.id }
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
      mediaFileToDelete ? { fileId: mediaFileToDelete.id } : null,
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
      setDownloadFeedbackWarning(snapshot.downloadFeedbackWarning);
    },
    [],
  );

  const applyNativeTitleDetailSnapshot = React.useCallback(
    (
      snapshot: TitleOverviewNativeSnapshot<
        MovieOverviewSnapshotTitle,
        TitleAcquisitionDiagnostics,
        TitleHistoryEvent,
        TitleReleaseBlocklistEntry,
        ExternalSubtitleRecord
      >,
    ) => {
      const nextTitle = snapshot.title;
      setTitle(nextTitle);
      if (nextTitle) {
        onTitleResolved?.({
          id: nextTitle.id,
          slug: nextTitle.slug,
          libraryId: nextTitle.libraryId,
          librarySlug: nextTitle.librarySlug,
        });
      }
      setCollections(nextTitle?.collections ?? []);
      setEvents(snapshot.titleHistory);
      setBlocklistEntries(snapshot.titleReleaseBlocklist);
      setMediaFiles(nextTitle?.mediaFiles ?? []);
      setSubtitleDownloads(snapshot.externalSubtitles);
      setWantedItem(nextTitle?.wantedItems?.items?.[0] ?? null);
      setHasDownloadClients(snapshot.hasDownloadClients);
      if (!nextTitle || !snapshot.hasDownloadClients) {
        applyDownloadFeedbackSnapshot(createEmptyTitleOverviewDownloadFeedbackSnapshot());
        setDownloadFeedbackSettled(true);
      }
      const nextTitleId = nextTitle?.id ?? null;
      if (
        renamePlanTitleIdRef.current !== nextTitleId ||
        !nextTitle ||
        !snapshot.hasDownloadClients
      ) {
        setRenamePlan(null);
      }
      renamePlanTitleIdRef.current = nextTitleId;
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
      setGlobalStatus(error instanceof Error ? error.message : t("status.apiError"));
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
      MovieOverviewSnapshotTitle,
      TitleAcquisitionDiagnostics,
      TitleHistoryEvent,
      TitleReleaseBlocklistEntry,
      ExternalSubtitleRecord
    >(client, requestedTitleId, 200);
    if (currentTitleIdRef.current !== requestedTitleId) {
      return;
    }
    applyNativeTitleDetailSnapshot(snapshot);
    if (!snapshot.title || !snapshot.hasDownloadClients) {
      return;
    }
    void refreshDownloadFeedback();
  }, [applyNativeTitleDetailSnapshot, client, refreshDownloadFeedback, titleId]);
  const moreLikeThisActions = useTitleMoreLikeThisActions({
    canAddItems: canAddDiscoveryItems,
    canRequestItems: canRequestDiscoveryItems,
    onCatalogChanged: refreshTitleDetail,
  });

  // Load title detail on mount
  React.useEffect(() => {
    let cancelled = false;

    if (!titleId) {
      setTitle(null);
      setCollections([]);
      setEvents([]);
      setBlocklistEntries([]);
      setSearchResults([]);
      setInteractiveSearchAttempted(false);
      setMediaFiles([]);
      setDownloadQueueSeed([]);
      setDownloadFeedbackSettled(false);
      setSubtitleDownloads([]);
      setRenamePlan(null);
      renamePlanTitleIdRef.current = null;
      setRenamePreviewing(false);
      setRenameApplying(false);
      setHasDownloadClients(true);
      setDownloadFeedbackWarning(null);
      setShowSearchPrerequisiteNotice(false);
      setTitleLookupAttempted(false);
      setTitleLookupFailed(false);
      setLoading(false);
      setWantedItem(null);
      return () => {
        cancelled = true;
      };
    }

    setTitleLookupAttempted(false);
    setTitleLookupFailed(false);
    setSearchResults([]);
    setInteractiveSearchAttempted(false);
    setDownloadQueueSeed([]);
    setDownloadFeedbackWarning(null);
    setDownloadFeedbackSettled(false);
    setShowSearchPrerequisiteNotice(false);
    setLoading(true);
    refreshTitleDetail()
      .catch((err: unknown) => {
        if (!cancelled) {
          setTitleLookupFailed(true);
          setGlobalStatus(err instanceof Error ? err.message : t("status.apiError"));
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
  }, [loading, titleId, titleLookupAttempted, titleLookupFailed, title, onTitleNotFound]);

  // Fetch quality profile catalog and default root folder
  React.useEffect(() => {
    let cancelled = false;
    const load = async () => {
      try {
        const { data, error } = await client.query(movieOverviewSettingsInitQuery, {}).toPromise();
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
        setRenameEnabled(data.mediaSettings?.renameEnabled !== false);
      } catch {
        // Settings fetch is best-effort
      }
    };
    void load();
    return () => { cancelled = true; };
  }, [client]);

  React.useEffect(() => {
    let cancelled = false;
    const load = async () => {
      const libraryId = title?.libraryId;
      if (!libraryId) {
        setRootFolders([]);
        return;
      }
      try {
        const { data, error } = await client
          .query(
            librariesQuery,
            { facet: "movie", permission: "manageTitles" },
            { requestPolicy: "network-only" },
          )
          .toPromise();
        if (error) throw error;
        if (cancelled) return;
        const library = (data.libraries ?? []).find(
          (candidate: { id: string }) => candidate.id === libraryId,
        );
        setRootFolders(Array.isArray(library?.roots) ? library.roots : []);
      } catch {
        if (!cancelled) setRootFolders([]);
      }
    };
    void load();
    return () => {
      cancelled = true;
    };
  }, [client, title?.libraryId]);

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

  const handleClearReleaseBlocklistEntry = React.useCallback(
    async (entryId: string) => {
      setClearingReleaseBlocklistEntryId(entryId);
      try {
        const { error } = await client
          .mutation(clearTitleReleaseBlocklistEntryMutation, {
            id: entryId,
          })
          .toPromise();
        if (error) {
          throw error;
        }
        await refreshTitleDetail();
      } catch (error: unknown) {
        setGlobalStatus(error instanceof Error ? error.message : t("status.apiError"));
      } finally {
        setClearingReleaseBlocklistEntryId((current) =>
          current === entryId ? null : current,
        );
      }
    },
    [client, refreshTitleDetail, setGlobalStatus, t],
  );

  const handleDeleteMediaFile = React.useCallback((fileId: string) => {
    const file = mediaFiles.find((candidate) => candidate.id === fileId) ?? null;
    setMediaFileToDelete(file);
    setMediaFileDeleteTypedConfirmation("");
  }, [mediaFiles]);

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
      } catch (err) {
        setGlobalStatus(err instanceof Error ? err.message : t("status.apiError"));
      } finally {
        setMonitoredUpdating(false);
      }
    },
    [title, client, refreshTitleDetail, setGlobalStatus, t],
  );

  const runWantedAction = React.useCallback(
    async (
      action: "pause" | "resume" | "reset",
      mutation: string,
      successMessage?: string,
    ) => {
      if (!wantedItem) return;
      setWantedActionLoading(action);
      try {
        const { error } = await client.mutation(mutation, {
          id: wantedItem.id,
        }).toPromise();
        if (error) throw error;
        if (successMessage) {
          setGlobalStatus(successMessage);
        }
        await refreshTitleDetail();
      } catch (err) {
        setGlobalStatus(err instanceof Error ? err.message : t("status.apiError"));
      } finally {
        setWantedActionLoading(null);
      }
    },
    [wantedItem, client, refreshTitleDetail, setGlobalStatus, t],
  );

  const handleSearchMonitored = React.useCallback(
    async () => {
      if (!title) return;
      if (!hasDownloadClients) {
        setShowSearchPrerequisiteNotice(true);
        return;
      }
      setSearchMonitoredLoading(true);
      try {
        const payload = await retryWithReplaceOnConflict(
          { titleId: title.id },
          async (input) => {
            const { data, error } = await client.mutation(triggerTitleWantedSearchMutation, {
              input,
            }).toPromise();
            if (error) throw error;
            return data?.triggerTitleWantedSearch;
          },
          "A download is already in progress for this title.",
          confirmReplaceConflict,
        );
        assertNoReplaceConflict(payload, "A download is already in progress for this title.");

        const queued = payload?.queuedCount ?? 0;
        setGlobalStatus(
          queued > 0
            ? t("status.searchMonitoredQueued", { count: queued })
            : t("status.searchMonitoredEmpty"),
        );
        await refreshTitleDetail();
      } catch (err) {
        setGlobalStatus(err instanceof Error ? err.message : t("status.apiError"));
      } finally {
        setSearchMonitoredLoading(false);
      }
    },
    [
      title,
      hasDownloadClients,
      client,
      confirmReplaceConflict,
      refreshTitleDetail,
      setGlobalStatus,
      t,
    ],
  );

  const handlePauseWanted = React.useCallback(
    async () => {
      await runWantedAction("pause", pauseWantedItemMutation);
    },
    [runWantedAction],
  );

  const handleResumeWanted = React.useCallback(
    async () => {
      await runWantedAction("resume", resumeWantedItemMutation);
    },
    [runWantedAction],
  );

  const handleResetWanted = React.useCallback(
    async () => {
      await runWantedAction("reset", resetWantedItemMutation);
    },
    [runWantedAction],
  );

  const runIndexerSearch = React.useCallback(async () => {
    if (!title) return;
    indexerSearchAbortRef.current?.abort();
    const abortController = new AbortController();
    indexerSearchAbortRef.current = abortController;
    if (!hasDownloadClients) {
      setShowSearchPrerequisiteNotice(true);
      setSearchResults([]);
      setInteractiveSearchAttempted(false);
      indexerSearchAbortRef.current = null;
      return;
    }
    setInteractiveSearchAttempted(true);
    setShowSearchPrerequisiteNotice(false);
    setSearching(true);
    setGlobalStatus(t("status.searchingNzb", { query: title.name, category: "" }));
    try {
      const { data, error } = await client.query(searchForTitleQuery, {
        titleId: title.id,
      }, {
        fetch: makeAbortableFetch(abortController.signal),
      }).toPromise();
      if (error) throw error;
      if (abortController.signal.aborted) return;
      const results = data.searchReleases ?? [];
      setSearchResults(results);
      setGlobalStatus(t("status.foundNzb", { count: results.length }));
    } catch (err) {
      if (isAbortError(err) || abortController.signal.aborted) {
        return;
      }
      setGlobalStatus(err instanceof Error ? err.message : t("status.apiError"));
      setSearchResults([]);
    } finally {
      if (indexerSearchAbortRef.current === abortController) {
        indexerSearchAbortRef.current = null;
        setSearching(false);
      }
    }
  }, [title, hasDownloadClients, client, t, setGlobalStatus]);

  const queueRelease = React.useCallback(
    async (release: Release) => {
      if (!title) return;
      if (!release.candidateToken) {
        setGlobalStatus(t("status.releaseMissingCandidateToken"));
        return;
      }
      try {
        const input = {
          titleId: title.id,
          scope: releaseQueueScopeInput(release, { title: true }),
          candidateToken: release.candidateToken,
        };
        const payload = await retryWithReplaceOnConflict(
          input,
          async (nextInput) => {
            const { data, error } = await client.mutation(queueExistingMutation, {
              input: nextInput,
            }).toPromise();
            if (error) throw error;
            return data?.queueExistingTitleDownload;
          },
          "A download is already in progress for this title.",
          confirmReplaceConflict,
        );
        assertNoReplaceConflict(payload, "A download is already in progress for this title.");
        const queuedMessage = t("status.queueSuccess", { name: release.title });
        setGlobalStatus(queuedMessage);
        await refreshTitleDetail();
      } catch (err) {
        setGlobalStatus(userFacingGraphQlErrorMessage(err, t("status.apiError")));
      }
    },
    [title, client, confirmReplaceConflict, t, setGlobalStatus, refreshTitleDetail],
  );

  const queueAdditionalRelease = React.useCallback(
    async (release: Release) => {
      if (!title) return;
      if (!release.candidateToken) {
        setGlobalStatus(t("status.releaseMissingCandidateToken"));
        return;
      }
      try {
        const { data, error } = await client.mutation(queueExistingMutation, {
          input: {
            titleId: title.id,
            scope: releaseQueueScopeInput(release, { title: true }),
            candidateToken: release.candidateToken,
            purpose: "ADDITIONAL_FILE",
          },
        }).toPromise();
        if (error) throw error;
        assertNoReplaceConflict(
          data?.queueExistingTitleDownload,
          "A download is already in progress for this title.",
        );
        setGlobalStatus(t("status.queueSuccess", { name: release.title }));
        await refreshTitleDetail();
      } catch (err) {
        setGlobalStatus(userFacingGraphQlErrorMessage(err, t("status.apiError")));
      }
    },
    [title, client, t, setGlobalStatus, refreshTitleDetail],
  );

  const handleMakePrimaryMovieFile = React.useCallback(
    async (fileId: string) => {
      if (!title) return;
      setPrimaryMovieFileUpdatingId(fileId);
      try {
        const { error } = await client.mutation(setPrimaryMovieFileMutation, {
          input: {
            titleId: title.id,
            fileId,
          },
        }).toPromise();
        if (error) throw error;
        setGlobalStatus(t("status.primaryMovieFileUpdated"));
        await refreshTitleDetail();
      } catch (err) {
        setGlobalStatus(userFacingGraphQlErrorMessage(err, t("status.apiError")));
      } finally {
        setPrimaryMovieFileUpdatingId(null);
      }
    },
    [title, client, t, setGlobalStatus, refreshTitleDetail],
  );

  const handleRefreshAndScan = React.useCallback(async () => {
    if (!title) return;
    setRefreshAndScanLoading(true);
    try {
      const { data, error } = await client.mutation(scanTitleLibraryMutation, {
        titleId: title.id,
      }).toPromise();
      if (error) throw error;
      setGlobalStatus(
        t("status.titleScanSuccess", {
          imported: data.scanTitleLibrary.imported,
          skipped: data.scanTitleLibrary.skipped,
          unmatched: data.scanTitleLibrary.unmatched,
        }),
      );
      await refreshTitleDetail();
    } catch (err) {
      setGlobalStatus(err instanceof Error ? err.message : t("settings.libraryScanFailed"));
    } finally {
      setRefreshAndScanLoading(false);
    }
  }, [title, refreshTitleDetail, client, setGlobalStatus, t]);

  const triggerMismatchRecovery = React.useCallback(async () => {
    if (!title) return;
    const { data, error } = await client.mutation(
      triggerTitleMismatchRecoverySearchMutation,
      { titleId: title.id },
    ).toPromise();
    if (error) {
      setGlobalStatus(error.message);
      return;
    }
    setGlobalStatus(
      t("status.mismatchRecoveryQueued", {
        count: data?.triggerTitleMismatchRecoverySearch?.queuedCount ?? 0,
      }),
    );
    await refreshTitleDetail();
  }, [client, refreshTitleDetail, setGlobalStatus, t, title]);

  const previewRename = React.useCallback(async () => {
    if (!title) return;
    setRenamePreviewing(true);
    try {
      const { data, error } = await client.query(mediaRenamePreviewQuery, {
        input: {
          facet: "movie",
          titleId: title.id,
          dryRun: true,
        },
      }).toPromise();
      if (error) throw error;
      const plan = data.mediaRenamePreview;
      setRenamePlan(plan);
      setGlobalStatus(
        t("status.renamePreviewGenerated", {
          total: plan.total,
          renamable: plan.renamable,
        }),
      );
    } catch (err) {
      setGlobalStatus(err instanceof Error ? err.message : t("status.apiError"));
      setRenamePlan(null);
    } finally {
      setRenamePreviewing(false);
    }
  }, [title, client, setGlobalStatus, t]);

  const applyRename = React.useCallback(async () => {
    if (!title || !renamePlan) return;
    setRenameApplying(true);
    try {
      const { data, error } = await client.mutation(
        applyMediaRenameMutation,
        {
          input: {
            facet: "movie",
            titleId: title.id,
            fingerprint: renamePlan.fingerprint,
          },
        },
      ).toPromise();
      if (error) throw error;
      const result = data.applyMediaRename;
      setGlobalStatus(
        t("status.renameApplied", {
          applied: result.applied,
          skipped: result.skipped,
          failed: result.failed,
        }),
      );
      await refreshTitleDetail();
    } catch (err) {
      setGlobalStatus(err instanceof Error ? err.message : t("status.apiError"));
    } finally {
      setRenameApplying(false);
    }
  }, [title, renamePlan, refreshTitleDetail, client, setGlobalStatus, t]);

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
    } catch (err) {
      setGlobalStatus(err instanceof Error ? err.message : t("status.failedToDelete"));
    } finally {
      setDeleteLoading(false);
    }
  }, [
    client,
    deleteFilesOnDisk,
    onBackToList,
    onTitleNotFound,
    setGlobalStatus,
    titleDeletePreview,
    titleDeleteTypedConfirmation,
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

  return (
    <>
      <MovieOverviewView
        canManageTitle={canManageTitle}
        loading={loading}
        title={title}
        collections={collections}
        events={events}
        searchResults={searchResults}
        searching={searching}
        hasDownloadClients={hasDownloadClients}
        showSearchPrerequisiteNotice={showSearchPrerequisiteNotice}
        renamePlan={renamePlan}
        renameEnabled={renameEnabled}
        renamePreviewing={renamePreviewing}
        renameApplying={renameApplying}
        interactiveSearchAttempted={interactiveSearchAttempted}
        searchMonitoredLoading={searchMonitoredLoading}
        refreshAndScanLoading={refreshAndScanLoading}
        deleteLoading={deleteLoading}
        onSearch={runIndexerSearch}
        onQueue={queueRelease}
        onQueueAdditional={queueAdditionalRelease}
        onSearchMonitored={handleSearchMonitored}
        onRefreshAndScan={handleRefreshAndScan}
        onTitleChanged={refreshTitleDetail}
        onPreviewRename={previewRename}
        onApplyRename={applyRename}
        onBackToList={onBackToList}
        qualityProfiles={qualityProfiles}
        defaultRootFolder={defaultRootFolder}
        rootFolders={rootFolders}
        onUpdateTitleOptions={handleUpdateTitleOptions}
        onSetTitleMonitored={handleSetTitleMonitored}
        monitoredUpdating={monitoredUpdating}
        wantedItem={wantedItem}
        downloadQueueItems={downloadQueueItems}
        wantedActionLoading={wantedActionLoading}
        onPauseWanted={handlePauseWanted}
        onResumeWanted={handleResumeWanted}
        onResetWanted={handleResetWanted}
        onTriggerMismatchRecovery={triggerMismatchRecovery}
        onRequestDeleteTitle={handleRequestDeleteTitle}
        blocklistEntries={blocklistEntries}
        clearingReleaseBlocklistEntryId={clearingReleaseBlocklistEntryId}
        onClearReleaseBlocklistEntry={
          canManageTitle ? handleClearReleaseBlocklistEntry : undefined
        }
        mediaFiles={mediaFiles}
        primaryMovieFileUpdatingId={primaryMovieFileUpdatingId}
        subtitleDownloads={subtitleDownloads}
        onDeleteFile={handleDeleteMediaFile}
        onMakePrimaryFile={canManageTitle ? handleMakePrimaryMovieFile : undefined}
        onRefreshSubtitles={() => { void refreshTitleDetail(); }}
        onOpenFixMatch={() => setFixMatchOpen(true)}
        moreLikeThisActions={moreLikeThisActions.stripProps}
      />
      {moreLikeThisActions.dialogs}
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
      {replaceConflictDialog}
    </>
  );
});
