import * as React from "react";
import { MediaContentView } from "@/components/views/media-content-view";
import {
  AddToCatalogDialog,
  EMPTY_SEARCH_RESULT,
} from "@/components/root/add-to-catalog-dialog";
import { RequestMediaDialog } from "@/components/root/request-media-dialog";
import type { MediaRenamePlan } from "@/components/common/media-rename-plan-panel";
import {
  addTitleMutation,
  applyMediaRenameMutation,
  buildSetTitleMonitoredBatchMutation,
  buildUpdateTitleBatchMutation,
  createLibraryMutation,
  deleteMediaFileMutation,
  deleteLibraryMutation,
  queueBestReleaseMutation,
  queueExistingMutation,
  scanLibraryMutation,
  deleteTitlesMutation,
  setPrimaryMovieFileMutation,
  setTitleMonitoredMutation,
  updateLibraryMutation,
  updateRuleSetMutation,
} from "@/lib/graphql/mutations";
import {
  browsePathQuery,
  catalogHasValidRootQuery,
  deleteMediaFilePreviewQuery,
  deleteTitlePreviewQuery,
  downloadClientRoutingQuery,
  jobRunEventsSubscription,
  jobRunsQuery,
  librariesQuery,
  libraryDownloadClientsQuery,
  librarySettingsQuery,
  externalSubtitlesQuery,
  mediaRenamePreviewQuery,
  catalogDiscoveryQuery,
  discoveryItemDetailQuery,
  ruleSetsQuery,
  routingPageInitQuery,
  searchForTitleQuery,
  seriesTitlePanelDetailQuery,
  titlePanelDetailQuery,
  titleReleaseBlocklistQuery,
  titlesQuery,
} from "@/lib/graphql/queries";
import {
  CATEGORY_SCOPE_MAP,
  QUALITY_PROFILE_INHERIT_VALUE,
  viewToFacet,
} from "@/lib/constants/settings";
import { isAbortError, makeAbortableFetch } from "@/lib/graphql/urql-client";
import { useClient } from "urql";
import type {
  ContentSettingsSection,
  OverviewTitleTarget,
  ViewId,
} from "@/components/root/types";
import { toProfileOptions } from "@/lib/utils/quality-profiles";
import {
  discoveryItemFacet,
  metadataResultForDiscoveryItem,
} from "@/lib/utils/discovery-actions";
import {
  normalizeLibraryFilterSelection,
  singleSelectedLibraryId,
} from "@/lib/utils/library-filter";
import {
  EMPTY_TITLE_QUICK_FILTERS,
  buildTitleCatalogQueryVariables,
  titleCatalogQueryKey,
} from "@/lib/utils/title-catalog-query";
import { releaseQueueScopeInput } from "@/lib/utils/release-queue-scope";
import { useBulkDelete } from "@/lib/hooks/use-bulk-delete";
import { useDownloadClientRouting } from "@/lib/hooks/use-download-client-routing";
import { useIndexerRouting } from "@/lib/hooks/use-indexer-routing";
import { useMediaSettings } from "@/lib/hooks/use-media-settings";
import { useIsMobile } from "@/lib/hooks/use-mobile";
import { useQueueFormState } from "@/lib/hooks/use-queue-form-state";
import { useTitleManagementState } from "@/lib/hooks/use-title-management-state";
import type {
  DownloadClientRecord,
  DownloadClientRoutingEntry,
  JobRun,
  LibraryRecord,
  LibrarySettingsDraft,
  LibrarySettingsRecord,
  Release,
  RootFolderOption,
  TitleReleaseBlocklistEntry,
  TitleRecord,
  CatalogDiscoveryGroup,
  CatalogDiscoveryInput,
  CatalogDiscoveryItem,
  CatalogDiscoveryPayload,
  Facet,
  RuleSetRecord,
} from "@/lib/types";
import type { ExternalSubtitleRecord } from "@/lib/types/subtitles";
import type { DeletePreview } from "@/lib/types/delete-preview";
import { Checkbox } from "@/components/ui/checkbox";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { useDownloadConflictConfirmation } from "@/components/common/download-conflict-confirmation";
import { DeletePreviewSummary } from "@/components/common/delete-preview-summary";
import type { MetadataTvdbSearchItem } from "@/lib/graphql/smg-queries";
import { userFacingGraphQlErrorMessage } from "@/lib/graphql/error-message";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useLibraryScanProgress } from "@/lib/context/library-scan-progress-context";
import { useSearchContext } from "@/lib/context/search-context";
import {
  reactiveRefreshEpoch,
  useReactiveRefresh,
} from "@/lib/context/reactive-refresh-context";
import { useDeletePreview } from "@/lib/hooks/use-delete-preview";
import { useDeferredWsSubscription } from "@/lib/hooks/use-deferred-ws-subscription";
import { useOverviewWindowScrollRestoration } from "@/lib/hooks/use-overview-window-scroll-restoration";
import { useJobRunToasts } from "@/components/root/job-run-provider";
import type { TitleOptionUpdates } from "@/lib/types/title-options";
import { isTerminalJobRunStatus, normalizeJobRun } from "@/lib/utils/job-runs";
import { toast } from "sonner";
import { BulkTitleEditDialog } from "@/components/views/media-content/bulk-title-edit-dialog";
import {
  readStoredContentViewMode,
  writeStoredContentViewMode,
  type ContentViewMode,
} from "@/components/views/media-content/content-view-mode";
import {
  filterTitlesByQuickFilters,
  getTitleQuickFilterCounts,
  type TitleQuickFilters,
} from "@/components/views/media-content/title-quick-filters";
import {
  defaultSortDirectionForTitleKey,
  type TitleTableSortDirection,
  type TitleTableSortKey,
} from "@/components/views/media-content/title-table-shared";
import {
  assertNoReplaceConflict,
  retryWithReplaceOnConflict,
} from "@/lib/utils/download-conflicts";

const HYDRATION_POSTER_REFRESH_WINDOW_MS = 5 * 60 * 1000;
const HYDRATION_POSTER_REFRESH_INTERVAL_MS = 2_500;
const TITLE_DELETION_JOB_FALLBACK_DELAYS_MS = [
  10_000, 60_000, 180_000,
] as const;
const TITLE_CATALOG_PAGE_SIZE = 72;
const ALL_LIBRARIES_VALUE = "__all__";

type MediaContentContainerProps = {
  view: ViewId;
  contentSettingsSection: ContentSettingsSection;
  canManageConfig: boolean;
  canManageSystemSettings: boolean;
  canManageCatalogSettings: boolean;
  canManageLibrarySettings: boolean;
  canManageTitle: boolean;
  canRequestMedia: boolean;
  onOpenOverview: (
    targetView: ViewId,
    overviewTarget: OverviewTitleTarget,
  ) => void;
  routeOverviewTitleId: string | null;
  routeOverviewPending: boolean;
  routeOverviewEpisodeId: string | null;
  onCloseOverview: () => void;
};

type SelectedOverviewMediaFile = NonNullable<TitleRecord["mediaFiles"]>[number];

type SelectedOverviewMediaFileDeleteTarget = {
  titleId: string;
  file: SelectedOverviewMediaFile;
};

type TitleCatalogState = {
  queryKey: string;
  hasMore: boolean;
  nextOffset: number;
  totalCount: number;
  loadingMore: boolean;
};

type TitleCatalogSortState = {
  key: TitleTableSortKey;
  direction: TitleTableSortDirection;
};

const emptyTitleCatalogState: TitleCatalogState = {
  queryKey: "",
  hasMore: false,
  nextOffset: 0,
  totalCount: 0,
  loadingMore: false,
};

const defaultTitleCatalogSortState: TitleCatalogSortState = {
  key: "name",
  direction: "asc",
};

const CATALOG_DISCOVERY_LIMIT_PER_GROUP = 12;
const CATALOG_DISCOVERY_MAX_GROUPS = 6;

type ActiveCatalogListFilters = {
  facet: TitleRecord["facet"];
  query: string;
  libraryIds: readonly string[];
};

function mergePreferLoadedImageFields(
  current: TitleRecord,
  incoming: TitleRecord,
): TitleRecord {
  const incomingHasPoster = Boolean(
    incoming.posterUrl || incoming.posterSourceUrl,
  );
  const incomingHasBackground = Boolean(
    incoming.backgroundUrl || incoming.backgroundSourceUrl,
  );

  return {
    ...incoming,
    posterUrl: incomingHasPoster
      ? incoming.posterUrl
      : (current.posterUrl ?? null),
    posterSourceUrl: incomingHasPoster
      ? incoming.posterSourceUrl
      : (current.posterSourceUrl ?? null),
    backgroundUrl: incomingHasBackground
      ? incoming.backgroundUrl
      : (current.backgroundUrl ?? null),
    backgroundSourceUrl: incomingHasBackground
      ? incoming.backgroundSourceUrl
      : (current.backgroundSourceUrl ?? null),
    overview:
      incoming.overview === undefined ? current.overview : incoming.overview,
    runtimeMinutes:
      incoming.runtimeMinutes === undefined
        ? current.runtimeMinutes
        : incoming.runtimeMinutes,
    genres: incoming.genres === undefined ? current.genres : incoming.genres,
    language:
      incoming.language === undefined ? current.language : incoming.language,
    firstAired:
      incoming.firstAired === undefined
        ? current.firstAired
        : incoming.firstAired,
    network:
      incoming.network === undefined ? current.network : incoming.network,
    studio: incoming.studio === undefined ? current.studio : incoming.studio,
    country:
      incoming.country === undefined ? current.country : incoming.country,
    metadataLanguage:
      incoming.metadataLanguage === undefined
        ? current.metadataLanguage
        : incoming.metadataLanguage,
    monitorType:
      incoming.monitorType === undefined
        ? current.monitorType
        : incoming.monitorType,
    useSeasonFolders:
      incoming.useSeasonFolders === undefined
        ? current.useSeasonFolders
        : incoming.useSeasonFolders,
    monitorSpecials:
      incoming.monitorSpecials === undefined
        ? current.monitorSpecials
        : incoming.monitorSpecials,
    interSeasonMovies:
      incoming.interSeasonMovies === undefined
        ? current.interSeasonMovies
        : incoming.interSeasonMovies,
    fillerPolicy:
      incoming.fillerPolicy === undefined
        ? current.fillerPolicy
        : incoming.fillerPolicy,
    recapPolicy:
      incoming.recapPolicy === undefined
        ? current.recapPolicy
        : incoming.recapPolicy,
    collections:
      incoming.collections === undefined
        ? current.collections
        : incoming.collections,
    mediaFiles:
      incoming.mediaFiles === undefined
        ? current.mediaFiles
        : incoming.mediaFiles,
    metadataFetchedAt: incoming.metadataFetchedAt ?? current.metadataFetchedAt,
  };
}

function mergeCatalogTitlesPreservingImages(
  currentTitles: TitleRecord[],
  incomingTitles: TitleRecord[],
): TitleRecord[] {
  const currentById = new Map(currentTitles.map((title) => [title.id, title]));

  return incomingTitles.map((title) => {
    const current = currentById.get(title.id);
    return current ? mergePreferLoadedImageFields(current, title) : title;
  });
}

function appendCatalogTitlesPreservingImages(
  currentTitles: TitleRecord[],
  incomingTitles: TitleRecord[],
): TitleRecord[] {
  const currentById = new Map(currentTitles.map((title) => [title.id, title]));
  const next = [...currentTitles];

  for (const title of incomingTitles) {
    const current = currentById.get(title.id);
    if (current) {
      const merged = mergePreferLoadedImageFields(current, title);
      currentById.set(title.id, merged);
      const index = next.findIndex((candidate) => candidate.id === title.id);
      if (index !== -1) {
        next[index] = merged;
      }
      continue;
    }
    currentById.set(title.id, title);
    next.push(title);
  }

  return next;
}

function buildActiveCatalogListFilters(
  facet: TitleRecord["facet"],
  query: string,
  libraryIds: readonly string[],
): ActiveCatalogListFilters {
  return {
    facet,
    query: query.trim().toLocaleLowerCase(),
    libraryIds: [...libraryIds],
  };
}

function catalogTitleMatchesActiveListFilters(
  title: TitleRecord,
  filters: ActiveCatalogListFilters,
): boolean {
  if (title.facet !== filters.facet) {
    return false;
  }

  if (
    filters.libraryIds.length > 0 &&
    !filters.libraryIds.includes(title.libraryId)
  ) {
    return false;
  }

  return (
    filters.query.length === 0 ||
    title.name.toLocaleLowerCase().includes(filters.query)
  );
}

function upsertCatalogTitleRecord(
  titles: TitleRecord[],
  title: TitleRecord,
  filters?: ActiveCatalogListFilters,
): TitleRecord[] {
  const existingIndex = titles.findIndex((item) => item.id === title.id);
  if (filters && !catalogTitleMatchesActiveListFilters(title, filters)) {
    if (existingIndex === -1) {
      return titles;
    }
    const next = [...titles];
    next.splice(existingIndex, 1);
    return next;
  }

  const next = [...titles];
  if (existingIndex === -1) {
    next.push(title);
  } else {
    next[existingIndex] = mergePreferLoadedImageFields(
      next[existingIndex],
      title,
    );
  }
  return next;
}

function isPendingHydrationPosterTitle(
  title: TitleRecord,
  nowMs: number,
): boolean {
  if (
    title.posterUrl ||
    title.posterSourceUrl ||
    title.metadataFetchedAt != null
  ) {
    return false;
  }

  const createdAtMs = title.createdAt
    ? Date.parse(title.createdAt)
    : Number.NaN;
  if (!Number.isFinite(createdAtMs)) {
    return true;
  }

  return nowMs - createdAtMs <= HYDRATION_POSTER_REFRESH_WINDOW_MS;
}

function hasSelectedTitlePanelDetails(title: TitleRecord): boolean {
  return Boolean(
    title.overview?.trim() ||
    title.backgroundUrl ||
    title.backgroundSourceUrl ||
    title.runtimeMinutes ||
    (title.genres && title.genres.length > 0) ||
    title.language ||
    title.firstAired ||
    title.network ||
    title.studio ||
    title.country ||
    title.metadataLanguage ||
    title.monitorType ||
    title.useSeasonFolders != null ||
    title.monitorSpecials != null ||
    title.interSeasonMovies != null ||
    title.fillerPolicy ||
    title.recapPolicy,
  );
}

function hasSelectedTitleEpisodeDetails(title: TitleRecord): boolean {
  return title.facet === "movie"
    ? title.mediaFiles !== undefined
    : title.collections !== undefined;
}

function titlePanelDetailQueryForTitle(title: TitleRecord | null | undefined): string {
  return title?.facet === "movie"
    ? titlePanelDetailQuery
    : seriesTitlePanelDetailQuery;
}

function sameIdSet(
  left: ReadonlySet<string>,
  right: ReadonlySet<string>,
): boolean {
  if (left.size !== right.size) {
    return false;
  }

  for (const value of left) {
    if (!right.has(value)) {
      return false;
    }
  }

  return true;
}

function sameStringArray(left: string[], right: string[]): boolean {
  return (
    left.length === right.length &&
    left.every((value, index) => value === right[index])
  );
}

function batchItemAlias(index: number): string {
  return `item${index}`;
}

function batchFailureDetail(error: unknown): string | null {
  if (error instanceof Error) {
    const message = error.message.trim();
    return message || null;
  }

  if (typeof error === "string") {
    const trimmed = error.trim();
    return trimmed || null;
  }

  return null;
}

function withFailureDetail(message: string, detail: string | null): string {
  return detail ? `${message} ${detail}` : message;
}

function libraryRootsInput(roots: RootFolderOption[]) {
  return roots
    .map((root) => ({
      path: root.path.trim(),
      isDefault: root.isDefault,
    }))
    .filter((root) => root.path.length > 0);
}

function librarySettingsInput(
  settings: LibrarySettingsDraft | undefined,
): LibrarySettingsDraft | undefined {
  if (!settings) {
    return undefined;
  }
  return {
    requiredAudioLanguages: settings.requiredAudioLanguages,
    qualityProfileId: settings.qualityProfileId,
    requestQualityProfileIds: settings.requestQualityProfileIds,
    scoringPersona: settings.scoringPersona,
    fillerPolicy: settings.fillerPolicy,
    recapPolicy: settings.recapPolicy,
    monitorSpecials: settings.monitorSpecials,
    interSeasonMovies: settings.interSeasonMovies,
    monitorFillerMovies: settings.monitorFillerMovies,
    nfoWriteOnImport: settings.nfoWriteOnImport,
    plexmatchWriteOnImport: settings.plexmatchWriteOnImport,
    importMode: settings.importMode,
    setPermissionsLinux: settings.setPermissionsLinux,
    fileChmod: settings.fileChmod,
    folderChmod: settings.folderChmod,
    chownGroup: settings.chownGroup,
    indexerRouting: settings.indexerRouting,
    downloadClientRouting: settings.downloadClientRouting,
  };
}

function splitSucceededTitleIds(
  targets: TitleRecord[],
  predicate: (title: TitleRecord) => boolean,
): { succeededIds: string[]; failedIds: string[] } {
  const succeededIds: string[] = [];
  const failedIds: string[] = [];

  targets.forEach((title) => {
    if (predicate(title)) {
      succeededIds.push(title.id);
    } else {
      failedIds.push(title.id);
    }
  });

  return { succeededIds, failedIds };
}

function inferMonitoredBatchOutcome(
  targets: TitleRecord[],
  refreshedTitles: TitleRecord[],
  monitored: boolean,
): { succeededIds: string[]; failedIds: string[] } {
  const refreshedById = new Map(
    refreshedTitles.map((title) => [title.id, title]),
  );
  return splitSucceededTitleIds(
    targets,
    (title) => refreshedById.get(title.id)?.monitored === monitored,
  );
}

function inferTitleUpdateBatchOutcome(
  targets: TitleRecord[],
  refreshedTitles: TitleRecord[],
  changes: TitleOptionUpdates,
): { succeededIds: string[]; failedIds: string[] } {
  const refreshedById = new Map(
    refreshedTitles.map((title) => [title.id, title]),
  );
  return splitSucceededTitleIds(targets, (title) => {
    const refreshed = refreshedById.get(title.id);
    if (!refreshed) {
      return false;
    }

    if (
      changes.qualityProfileId !== undefined &&
      (refreshed.qualityProfileId ?? "") !== changes.qualityProfileId
    ) {
      return false;
    }
    if (
      changes.rootFolderId !== undefined &&
      (refreshed.rootFolderId ?? null) !== changes.rootFolderId
    ) {
      return false;
    }
    if (
      changes.monitorType !== undefined &&
      (refreshed.monitorType ?? "") !== changes.monitorType
    ) {
      return false;
    }
    if (
      changes.useSeasonFolders !== undefined &&
      refreshed.useSeasonFolders !== changes.useSeasonFolders
    ) {
      return false;
    }
    if (
      changes.monitorSpecials !== undefined &&
      refreshed.monitorSpecials !== changes.monitorSpecials
    ) {
      return false;
    }
    if (
      changes.interSeasonMovies !== undefined &&
      refreshed.interSeasonMovies !== changes.interSeasonMovies
    ) {
      return false;
    }
    if (
      changes.fillerPolicy !== undefined &&
      (refreshed.fillerPolicy ?? "") !== changes.fillerPolicy
    ) {
      return false;
    }
    if (
      changes.recapPolicy !== undefined &&
      (refreshed.recapPolicy ?? "") !== changes.recapPolicy
    ) {
      return false;
    }

    return true;
  });
}

function aggregateDeletePreviews(
  previews: DeletePreview[],
): DeletePreview | null {
  if (previews.length === 0) {
    return null;
  }

  const samplePaths = Array.from(
    new Set(previews.flatMap((preview) => preview.samplePaths)),
  ).slice(0, 12);
  const typedPrompt =
    previews.find((preview) => preview.requiresTypedConfirmation)
      ?.typedConfirmationPrompt ?? null;
  const mediaCount = previews.reduce(
    (sum, preview) => sum + preview.mediaCount,
    0,
  );
  const requiresTypedConfirmation =
    mediaCount > 50 ||
    previews.some((preview) => preview.requiresTypedConfirmation);

  return {
    fingerprint: "",
    totalFileCount: previews.reduce(
      (sum, preview) => sum + preview.totalFileCount,
      0,
    ),
    mediaCount,
    subtitleCount: previews.reduce(
      (sum, preview) => sum + preview.subtitleCount,
      0,
    ),
    imageCount: previews.reduce((sum, preview) => sum + preview.imageCount, 0),
    otherCount: previews.reduce((sum, preview) => sum + preview.otherCount, 0),
    directoryCount: previews.reduce(
      (sum, preview) => sum + preview.directoryCount,
      0,
    ),
    requiresTypedConfirmation,
    typedConfirmationPrompt:
      typedPrompt ??
      (requiresTypedConfirmation
        ? "Type DELETE to confirm this large delete."
        : null),
    targetLabel: "",
    samplePaths,
  };
}

export const MediaContentContainer = React.memo(function MediaContentContainer({
  view,
  contentSettingsSection,
  canManageConfig,
  canManageSystemSettings,
  canManageCatalogSettings,
  canManageLibrarySettings,
  canManageTitle,
  canRequestMedia,
  onOpenOverview,
  routeOverviewTitleId,
  routeOverviewPending,
  routeOverviewEpisodeId,
  onCloseOverview,
}: MediaContentContainerProps) {
  const searchState = useSearchContext();
  const {
    addMetadataSearchResultToCatalog,
    catalogConfigLoading,
    catalogQualityProfileOptions,
    ensureCatalogConfigReady,
    librariesByFacet,
    queueFacet,
    requestableLibrariesByFacet,
    requestMetadataSearchResult,
    resolveDefaultQualityProfileIdForFacet,
    rootFoldersByFacet,
    runTvdbSearch,
    setQueueFacet,
    tvdbCandidates,
  } = searchState;
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const client = useClient();
  const { registerInteractiveJobRun } = useJobRunToasts();
  const { confirmReplaceConflict, replaceConflictDialog } =
    useDownloadConflictConfirmation();
  const { queueCatalogTitleRefresh } = useReactiveRefresh();
  const [titleDeleteTypedConfirmation, setTitleDeleteTypedConfirmation] =
    React.useState("");
  const [pendingDeletedTitleIds, setPendingDeletedTitleIds] = React.useState<
    Set<string>
  >(() => new Set());
  const pendingDeletedTitleIdsRef = React.useRef(pendingDeletedTitleIds);
  React.useLayoutEffect(() => {
    pendingDeletedTitleIdsRef.current = pendingDeletedTitleIds;
  }, [pendingDeletedTitleIds]);
  const deletionJobIdsRef = React.useRef(new Set<string>());
  const deletionFallbackTimersRef = React.useRef<
    ReturnType<typeof setTimeout>[]
  >([]);
  const [startedLibraryScanSessionId, setStartedLibraryScanSessionId] =
    React.useState<string | null>(null);
  const activeFacet = viewToFacet[view as keyof typeof viewToFacet] ?? "movie";
  const [selectedLibraryIds, setSelectedLibraryIds] = React.useState<string[]>(
    [],
  );
  const catalogDiscoveryRequestIdRef = React.useRef(0);
  const [catalogDiscoveryGroups, setCatalogDiscoveryGroups] =
    React.useState<CatalogDiscoveryGroup[]>([]);
  const [addDiscoveryDialogTarget, setAddDiscoveryDialogTarget] =
    React.useState<{ result: MetadataTvdbSearchItem; facet: Facet } | null>(
      null,
    );
  const [requestDiscoveryDialogTarget, setRequestDiscoveryDialogTarget] =
    React.useState<{ result: MetadataTvdbSearchItem; facet: Facet } | null>(
      null,
    );
  const {
    getActiveSession,
    getSessionById,
    refreshSessions: refreshLibraryScanSessions,
  } = useLibraryScanProgress();
  const activeLibraryScanSession = getActiveSession(activeFacet);
  const startedLibraryScanSession = startedLibraryScanSessionId
    ? getSessionById(startedLibraryScanSessionId)
    : null;
  const isMobile = useIsMobile();
  const activeQualityScopeId =
    CATEGORY_SCOPE_MAP[view as keyof typeof CATEGORY_SCOPE_MAP] ?? "movie";
  const isMediaView =
    view === "movies" || view === "series" || view === "anime";
  const routeOverviewActive = routeOverviewPending;
  const shouldLoadCatalogTitles =
    isMediaView &&
    contentSettingsSection === "overview" &&
    !routeOverviewActive;
  const shouldLoadMediaSettingsForSection =
    isMediaView &&
    (contentSettingsSection === "library" ||
      contentSettingsSection === "general" ||
      contentSettingsSection === "routing");
  const refreshCatalogDiscovery = React.useCallback(async () => {
    const requestId = catalogDiscoveryRequestIdRef.current + 1;
    catalogDiscoveryRequestIdRef.current = requestId;
    if (!shouldLoadCatalogTitles) {
      setCatalogDiscoveryGroups([]);
      return;
    }
    const libraryIds = selectedLibraryIds.filter(
      (libraryId) => libraryId !== ALL_LIBRARIES_VALUE,
    );
    const input: CatalogDiscoveryInput = {
      facet: activeFacet,
      libraryIds,
      includeUnresolved: true,
      limitPerGroup: CATALOG_DISCOVERY_LIMIT_PER_GROUP,
      maxGroups: CATALOG_DISCOVERY_MAX_GROUPS,
    };
    try {
      const { data, error } = await client
        .query<{ catalogDiscovery?: CatalogDiscoveryPayload }>(
          catalogDiscoveryQuery,
          { input },
          { requestPolicy: "network-only" },
        )
        .toPromise();
      if (error) {
        throw error;
      }
      if (catalogDiscoveryRequestIdRef.current === requestId) {
        setCatalogDiscoveryGroups(
          data?.catalogDiscovery?.groups ?? [],
        );
      }
    } catch (error) {
      console.error("[catalog-discovery] refresh failed:", error);
      if (catalogDiscoveryRequestIdRef.current === requestId) {
        setCatalogDiscoveryGroups([]);
      }
    }
  }, [activeFacet, client, selectedLibraryIds, shouldLoadCatalogTitles]);

  React.useEffect(() => {
    void refreshCatalogDiscovery();
  }, [refreshCatalogDiscovery]);

  const [desktopViewModes, setDesktopViewModes] = React.useState<
    Partial<Record<ViewId, ContentViewMode>>
  >(() => ({ [view]: readStoredContentViewMode(view) }));
  const desktopViewMode =
    desktopViewModes[view] ?? readStoredContentViewMode(view);
  const effectiveViewMode: ContentViewMode = isMobile
    ? "poster"
    : desktopViewMode;
  const [selectedTitleIds, setSelectedTitleIds] = React.useState<Set<string>>(
    () => new Set(),
  );
  const [selectedOverviewTitleId, setSelectedOverviewTitleId] = React.useState<
    string | null
  >(null);
  const [selectedOverviewDetailLoading, setSelectedOverviewDetailLoading] =
    React.useState(false);
  const [selectedOverviewBlocklistState, setSelectedOverviewBlocklistState] =
    React.useState<{
      titleId: string | null;
      entries: TitleReleaseBlocklistEntry[];
  }>({ titleId: null, entries: [] });
  const selectedOverviewBlocklistEntries =
    selectedOverviewBlocklistState.titleId === selectedOverviewTitleId
      ? selectedOverviewBlocklistState.entries
      : [];
  const [
    selectedOverviewExternalSubtitleState,
    setSelectedOverviewExternalSubtitleState,
  ] = React.useState<{
    titleId: string | null;
    entries: ExternalSubtitleRecord[];
  }>({ titleId: null, entries: [] });
  const selectedOverviewExternalSubtitles =
    selectedOverviewExternalSubtitleState.titleId === selectedOverviewTitleId
      ? selectedOverviewExternalSubtitleState.entries
      : [];
  const [titleQuickFilters, setTitleQuickFilters] =
    React.useState<TitleQuickFilters>(EMPTY_TITLE_QUICK_FILTERS);
  const [titleCatalogSort, setTitleCatalogSort] =
    React.useState<TitleCatalogSortState>(defaultTitleCatalogSortState);
  const effectiveTitleCatalogSort =
    effectiveViewMode === "poster"
      ? defaultTitleCatalogSortState
      : titleCatalogSort;
  const [bulkActionBusy, setBulkActionBusy] = React.useState(false);
  const [bulkEditDialogOpen, setBulkEditDialogOpen] = React.useState(false);
  const shouldLoadMediaSettings =
    shouldLoadMediaSettingsForSection || bulkEditDialogOpen;
  const [debouncedTitleFilter, setDebouncedTitleFilter] = React.useState("");
  const [libraries, setLibraries] = React.useState<LibraryRecord[]>([]);
  const [librariesLoading, setLibrariesLoading] = React.useState(false);
  const [libraryDownloadClients, setLibraryDownloadClients] = React.useState<
    DownloadClientRecord[]
  >([]);
  const [libraryDownloadClientsLoading, setLibraryDownloadClientsLoading] =
    React.useState(false);
  const [catalogBootstrapState, setCatalogBootstrapState] = React.useState({
    facet: activeFacet,
    loading: false,
    initialLoadComplete: false,
  });
  const [rootValidationLibraries, setRootValidationLibraries] = React.useState<
    LibraryRecord[]
  >([]);
  const [rootValidationLibrariesLoading, setRootValidationLibrariesLoading] =
    React.useState(false);
  const [catalogHasValidRoot, setCatalogHasValidRoot] = React.useState<
    boolean | null
  >(null);
  const [, setInvalidRootLibraryIds] = React.useState<string[]>([]);
  const [invalidRootPathsByLibraryId, setInvalidRootPathsByLibraryId] =
    React.useState<Record<string, string[]>>({});
  const [, setValidatedRootFolderSnapshotKey] = React.useState<string | null>(
    null,
  );
  const [librarySettingsSaving, setLibrarySettingsSaving] =
    React.useState(false);
  const rootFolderValidationSnapshot = React.useMemo(() => {
    if (
      !isMediaView ||
      librariesLoading ||
      contentSettingsSection !== "library"
    ) {
      return null;
    }

    const explicitSelectedLibraryIds = selectedLibraryIds.filter(
      (libraryId) => libraryId !== ALL_LIBRARIES_VALUE,
    );
    const selectedLibraryIdSet =
      explicitSelectedLibraryIds.length > 0
        ? new Set(explicitSelectedLibraryIds)
        : null;
    const relevantLibraries = libraries.filter((library) =>
      selectedLibraryIdSet ? selectedLibraryIdSet.has(library.id) : true,
    );
    const librariesWithConfiguredRoots = relevantLibraries.filter((library) =>
      library.roots.some((root) => root.path.trim().length > 0),
    );
    const key = librariesWithConfiguredRoots
      .map((library) => {
        const rootsKey = library.roots
          .map((root) => root.path.trim())
          .filter((path) => path.length > 0)
          .sort()
          .join("\u001f");
        return `${library.id}:${rootsKey}`;
      })
      .sort()
      .join("\u001e");

    return { key, librariesWithConfiguredRoots };
  }, [
    contentSettingsSection,
    isMediaView,
    libraries,
    librariesLoading,
    selectedLibraryIds,
  ]);
  const activeCatalogQueryRef = React.useRef("");
  const interactiveSearchAbortRef = React.useRef<AbortController | null>(null);
  const activeCatalogListFiltersRef = React.useRef<ActiveCatalogListFilters>({
    facet: activeFacet,
    query: "",
    libraryIds: [],
  });
  const catalogTitleRequestSeqRef = React.useRef(0);
  const catalogBootstrapRequestSeqRef = React.useRef(0);
  const catalogPageLoadInFlightRef = React.useRef(false);
  const catalogQueryKeyRef = React.useRef("");
  const latestCriticalMutationEpochRef = React.useRef(0);
  const selectedPanelHydrationKeyRef = React.useRef<string | null>(null);
  const skipNextCatalogOverviewReloadRef = React.useRef(false);
  const [catalogPaginationState, setCatalogPaginationState] =
    React.useState<TitleCatalogState>(emptyTitleCatalogState);

  React.useEffect(() => {
    catalogQueryKeyRef.current = catalogPaginationState.queryKey;
  }, [catalogPaginationState.queryKey]);

  React.useEffect(() => {
    return () => {
      interactiveSearchAbortRef.current?.abort();
      interactiveSearchAbortRef.current = null;
    };
  }, []);

  const {
    titleNameForQueue,
    setTitleNameForQueue,
    monitoredForQueue,
    setMonitoredForQueue,
    seasonFoldersForQueue,
    setSeasonFoldersForQueue,
    minAvailabilityForQueue,
    setMinAvailabilityForQueue,
  } = useQueueFormState();

  const {
    titleFilter,
    setTitleFilter,
    monitoredTitles,
    setMonitoredTitles,
    titleLoading,
    setTitleLoading,
    titleStatus,
    setTitleStatus,
    titleToDelete,
    setTitleToDelete,
    deleteFilesOnDisk,
    setDeleteFilesOnDisk,
    deleteTitleLoadingById,
    setDeleteTitleLoadingById,
    libraryScanLoading,
    setLibraryScanLoading,
    libraryScanSummary,
    setLibraryScanSummary,
  } = useTitleManagementState();
  const [titleContextTitles, setTitleContextTitles] = React.useState<
    TitleRecord[]
  >([]);
  const [
    selectedOverviewMediaFileToDelete,
    setSelectedOverviewMediaFileToDelete,
  ] = React.useState<SelectedOverviewMediaFileDeleteTarget | null>(null);
  const [
    selectedOverviewMediaFileDeleteLoading,
    setSelectedOverviewMediaFileDeleteLoading,
  ] = React.useState(false);
  const [
    selectedOverviewMediaFileDeleteTypedConfirmation,
    setSelectedOverviewMediaFileDeleteTypedConfirmation,
  ] = React.useState("");
  const [
    selectedOverviewPrimaryMovieFileUpdatingId,
    setSelectedOverviewPrimaryMovieFileUpdatingId,
  ] = React.useState<string | null>(null);
  const mergeTitleContextTitles = React.useCallback(
    (incomingTitles: TitleRecord[]) => {
      const activeFacetTitles = incomingTitles.filter(
        (title) =>
          title.facet === activeFacet &&
          !pendingDeletedTitleIdsRef.current.has(title.id),
      );
      if (activeFacetTitles.length === 0) {
        return;
      }
      setTitleContextTitles((current) =>
        appendCatalogTitlesPreservingImages(
          current.filter((title) => title.facet === activeFacet),
          activeFacetTitles,
        ),
      );
    },
    [activeFacet],
  );
  const libraryScanInProgress =
    libraryScanLoading ||
    Boolean(activeLibraryScanSession) ||
    Boolean(startedLibraryScanSessionId && !startedLibraryScanSession);
  const catalogInitialLoadComplete =
    shouldLoadCatalogTitles &&
    catalogBootstrapState.facet === activeFacet &&
    catalogBootstrapState.initialLoadComplete;
  const catalogBootstrapInFlight =
    catalogBootstrapState.facet === activeFacet &&
    catalogBootstrapState.loading;
  const catalogBootstrapLoading =
    shouldLoadCatalogTitles && !catalogInitialLoadComplete;
  const titleDeletePreviewVariables = React.useMemo(
    () =>
      titleToDelete && deleteFilesOnDisk ? { titleId: titleToDelete.id } : null,
    [deleteFilesOnDisk, titleToDelete],
  );
  const {
    preview: titleDeletePreview,
    loading: titleDeletePreviewLoading,
    error: titleDeletePreviewError,
  } = useDeletePreview(
    deleteTitlePreviewQuery,
    "deleteTitlePreview",
    titleDeletePreviewVariables,
    titleToDelete !== null && deleteFilesOnDisk,
  );
  const selectedOverviewMediaFileDeletePreviewVariables = React.useMemo(
    () =>
      selectedOverviewMediaFileToDelete
        ? { fileId: selectedOverviewMediaFileToDelete.file.id }
        : null,
    [selectedOverviewMediaFileToDelete],
  );
  const {
    preview: selectedOverviewMediaFileDeletePreview,
    loading: selectedOverviewMediaFileDeletePreviewLoading,
    error: selectedOverviewMediaFileDeletePreviewError,
  } = useDeletePreview(
    deleteMediaFilePreviewQuery,
    "deleteMediaFilePreview",
    selectedOverviewMediaFileDeletePreviewVariables,
    selectedOverviewMediaFileToDelete !== null,
  );
  const effectiveTitleQuickFilters = React.useMemo<TitleQuickFilters>(
    () => ({
      ...titleQuickFilters,
      continuing:
        activeFacet === "movie" ? false : titleQuickFilters.continuing,
      ended: activeFacet === "movie" ? false : titleQuickFilters.ended,
    }),
    [activeFacet, titleQuickFilters],
  );
  const libraryNameById = React.useMemo(
    () => new Map(libraries.map((library) => [library.id, library.name])),
    [libraries],
  );
  const librarySlugById = React.useMemo(
    () => new Map(libraries.map((library) => [library.id, library.slug])),
    [libraries],
  );
  const activeCatalogListFilters = React.useMemo(
    () =>
      buildActiveCatalogListFilters(
        activeFacet,
        debouncedTitleFilter,
        selectedLibraryIds,
      ),
    [activeFacet, debouncedTitleFilter, selectedLibraryIds],
  );
  const catalogSourceTitlesWithLibraries = React.useMemo(
    () =>
      appendCatalogTitlesPreservingImages(
        monitoredTitles,
        titleContextTitles.filter((title) =>
          catalogTitleMatchesActiveListFilters(title, activeCatalogListFilters),
        ),
      ).map((title) => ({
        ...title,
        libraryName:
          title.libraryName ??
          libraryNameById.get(title.libraryId) ??
          title.libraryId,
        librarySlug:
          title.librarySlug ?? librarySlugById.get(title.libraryId) ?? null,
      })),
    [
      activeCatalogListFilters,
      libraryNameById,
      librarySlugById,
      monitoredTitles,
      titleContextTitles,
    ],
  );
  const titleContextSourceTitles = React.useMemo(
    () =>
      catalogSourceTitlesWithLibraries.filter(
        (title) =>
          title.facet === activeFacet && !pendingDeletedTitleIds.has(title.id),
      ),
    [activeFacet, catalogSourceTitlesWithLibraries, pendingDeletedTitleIds],
  );
  const titleQuickFilterView =
    activeFacet === "movie"
      ? "movies"
      : activeFacet === "series"
        ? "series"
        : "anime";
  const titleQuickFilterCounts = React.useMemo(
    () =>
      getTitleQuickFilterCounts(
        titleContextSourceTitles,
        effectiveTitleQuickFilters,
        titleQuickFilterView,
      ),
    [
      effectiveTitleQuickFilters,
      titleContextSourceTitles,
      titleQuickFilterView,
    ],
  );
  React.useEffect(() => {
    if (pendingDeletedTitleIds.size === 0) {
      return;
    }
    setTitleContextTitles((current) =>
      current.filter((title) => !pendingDeletedTitleIds.has(title.id)),
    );
  }, [pendingDeletedTitleIds]);
  const visibleTitles = React.useMemo(
    () =>
      filterTitlesByQuickFilters(
        titleContextSourceTitles,
        effectiveTitleQuickFilters,
      ),
    [effectiveTitleQuickFilters, titleContextSourceTitles],
  );
  const selectedTitles = React.useMemo(
    () => visibleTitles.filter((title) => selectedTitleIds.has(title.id)),
    [selectedTitleIds, visibleTitles],
  );
  const selectedTitleLibraryIds = React.useMemo(
    () => Array.from(new Set(selectedTitles.map((title) => title.libraryId))),
    [selectedTitles],
  );
  const selectedTitleLibrary = React.useMemo(
    () =>
      selectedTitleLibraryIds.length === 1
        ? (libraries.find(
            (library) => library.id === selectedTitleLibraryIds[0],
          ) ?? null)
        : null,
    [libraries, selectedTitleLibraryIds],
  );
  const bulkRootFolders = React.useMemo(
    () => selectedTitleLibrary?.roots ?? [],
    [selectedTitleLibrary],
  );

  useOverviewWindowScrollRestoration({
    enabled: shouldLoadCatalogTitles && effectiveViewMode === "poster",
    ready: !titleLoading && visibleTitles.length > 0,
    storageKeySuffix: "window",
  });

  React.useLayoutEffect(() => {
    if (
      !shouldLoadCatalogTitles ||
      effectiveViewMode === "poster" ||
      typeof window === "undefined"
    ) {
      return;
    }

    window.scrollTo({ top: 0, left: 0, behavior: "auto" });
  }, [effectiveViewMode, shouldLoadCatalogTitles]);

  React.useEffect(() => {
    if (!shouldLoadCatalogTitles) {
      setDebouncedTitleFilter("");
      return;
    }

    const timer = window.setTimeout(() => {
      setDebouncedTitleFilter(titleFilter.trim());
    }, 250);

    return () => {
      window.clearTimeout(timer);
    };
  }, [shouldLoadCatalogTitles, titleFilter]);

  React.useEffect(() => {
    setTitleQuickFilters(EMPTY_TITLE_QUICK_FILTERS);
    setSelectedTitleIds(new Set());
    setSelectedOverviewTitleId(null);
    setSelectedLibraryIds((current) => (current.length === 0 ? current : []));
    setTitleContextTitles([]);
  }, [activeFacet]);

  // The route (slug deep link / in-app navigation) is the source of truth for
  // which title is selected in the list. Mirror it into local selection state
  // so the inline overview pane reflects the URL on load and live navigation.
  React.useEffect(() => {
    if (routeOverviewPending && !routeOverviewTitleId) {
      return;
    }
    setSelectedOverviewTitleId(routeOverviewTitleId);
  }, [routeOverviewPending, routeOverviewTitleId]);

  React.useEffect(() => {
    const visibleTitleIds = new Set(visibleTitles.map((title) => title.id));
    setSelectedTitleIds((current) => {
      let changed = false;
      const next = new Set<string>();
      current.forEach((id) => {
        if (visibleTitleIds.has(id)) {
          next.add(id);
        } else {
          changed = true;
        }
      });
      return changed ? next : current;
    });
  }, [visibleTitles]);

  React.useEffect(() => {
    if (!shouldLoadCatalogTitles || contentSettingsSection !== "overview") {
      setSelectedOverviewTitleId((current) =>
        current === null ? current : null,
      );
      return;
    }

    setSelectedOverviewTitleId((current) => {
      return current &&
        titleContextSourceTitles.some((title) => title.id === current)
        ? current
        : null;
    });
  }, [
    contentSettingsSection,
    shouldLoadCatalogTitles,
    titleContextSourceTitles,
  ]);

  React.useEffect(() => {
    activeCatalogQueryRef.current = debouncedTitleFilter;
  }, [debouncedTitleFilter]);

  React.useEffect(() => {
    setDesktopViewModes((current) => {
      if (current[view]) {
        return current;
      }
      return {
        ...current,
        [view]: readStoredContentViewMode(view),
      };
    });
  }, [view]);

  React.useEffect(() => {
    if (isMobile) {
      return;
    }
    writeStoredContentViewMode(desktopViewMode, view);
  }, [desktopViewMode, isMobile, view]);

  React.useEffect(() => {
    if (
      effectiveViewMode === "compact" &&
      shouldLoadCatalogTitles &&
      contentSettingsSection === "overview"
    ) {
      return;
    }
    setSelectedTitleIds((current) =>
      current.size === 0 ? current : new Set(),
    );
  }, [
    contentSettingsSection,
    effectiveViewMode,
    shouldLoadCatalogTitles,
    view,
  ]);

  React.useEffect(() => {
    const visibleIdSet = new Set(visibleTitles.map((title) => title.id));
    setSelectedTitleIds((current) => {
      if (current.size === 0) {
        return current;
      }
      const next = new Set(
        [...current].filter((titleId) => visibleIdSet.has(titleId)),
      );
      return sameIdSet(current, next) ? current : next;
    });
  }, [visibleTitles]);

  const {
    moviesPath,
    setMoviesPath,
    seriesPath,
    setSeriesPath,
    saveSetting,
    mediaSettingsLoading,
    mediaSettingsSaving,
    qualityProfiles,
    qualityProfileEntries,
    qualityProfileParseError,
    globalQualityProfileId,
    globalScoringPersona,
    categoryQualityProfileOverrides,
    categoryRequiredAudioLanguages,
    saveCategoryRequiredAudioLanguages,
    categoryPersonaSelections,
    categoryFolderTemplates,
    setCategoryFolderTemplates,
    categoryRenameTemplates,
    setCategoryRenameTemplates,
    categoryRenameEnabled,
    setCategoryRenameEnabled,
    categoryRenameCollisionPolicies,
    setCategoryRenameCollisionPolicies,
    categoryRenameMissingMetadataPolicies,
    setCategoryRenameMissingMetadataPolicies,
    categoryFillerPolicies,
    setCategoryFillerPolicies,
    categoryRecapPolicies,
    setCategoryRecapPolicies,
    categoryMonitorSpecials,
    setCategoryMonitorSpecials,
    categoryInterSeasonMovies,
    setCategoryInterSeasonMovies,
    categoryMonitorFillerMovies,
    setCategoryMonitorFillerMovies,
    nfoWriteOnImport,
    setNfoWriteOnImport,
    plexmatchWriteOnImport,
    setPlexmatchWriteOnImport,
    importMode,
    setImportMode,
    setPermissionsLinux,
    setSetPermissionsLinux,
    fileChmod,
    setFileChmod,
    folderChmod,
    setFolderChmod,
    chownGroup,
    setChownGroup,
    localPathStyle,
    saveCategoryQualityProfileOverride,
    saveCategoryScoringPersonaOverride,
    updateCategoryMediaProfileSettings,
    refreshMediaSettings,
  } = useMediaSettings({
    activeQualityScopeId,
    view,
  });

  const contentSettingsLabel =
    view === "movies"
      ? t("settings.moviesSettings")
      : view === "series"
        ? t("settings.seriesSettings")
        : t("settings.animeSettings");
  const activeFacetLabel =
    activeFacet === "movie"
      ? t("nav.movies")
      : activeFacet === "series"
        ? t("nav.series")
        : t("nav.anime");
  const {
    downloadClients,
    activeScopeRouting,
    activeScopeRoutingOrder,
    downloadClientRoutingLoading,
    downloadClientRoutingSaving,
    hydrateDownloadClientRouting,
    updateDownloadClientRoutingForScope,
    moveDownloadClientInScope,
  } = useDownloadClientRouting({
    activeQualityScopeId,
  });
  const {
    indexers,
    activeScopeRouting: activeScopeIndexerRouting,
    activeScopeRoutingOrder: activeScopeIndexerRoutingOrder,
    indexerRoutingLoading,
    indexerRoutingSaving,
    hydrateIndexerRouting,
    setIndexerEnabledForScope,
    updateIndexerRoutingForScope,
    moveIndexerInScope,
  } = useIndexerRouting({
    activeQualityScopeId,
  });
  const [routingInitLoading, setRoutingInitLoading] = React.useState(false);

  const [ruleSets, setRuleSets] = React.useState<RuleSetRecord[]>([]);
  const [rulesLoading, setRulesLoading] = React.useState(true);
  const [rulesSaving, setRulesSaving] = React.useState(false);
  const [libraryScanNotice, setLibraryScanNotice] = React.useState<
    string | null
  >(null);
  const [titleMonitoringLoadingById, setTitleMonitoringLoadingById] =
    React.useState<Record<string, boolean>>({});

  React.useEffect(() => {
    if (!activeLibraryScanSession) {
      setLibraryScanNotice(null);
    }
  }, [activeLibraryScanSession]);

  React.useEffect(() => {
    if (!startedLibraryScanSessionId) {
      return;
    }

    const session = getSessionById(startedLibraryScanSessionId);
    if (!session) {
      return;
    }

    if (
      session.status !== "completed" &&
      session.status !== "warning" &&
      session.status !== "failed"
    ) {
      return;
    }

    if (session.summary) {
      setLibraryScanSummary(session.summary);
    }

    setStartedLibraryScanSessionId(null);
  }, [getSessionById, setLibraryScanSummary, startedLibraryScanSessionId]);

  React.useEffect(() => {
    if (!startedLibraryScanSessionId || startedLibraryScanSession) {
      return;
    }

    let cancelled = false;
    const retryDelaysMs = [0, 400, 1_200];
    const timers = retryDelaysMs.map((delayMs) =>
      window.setTimeout(() => {
        if (cancelled) {
          return;
        }
        void refreshLibraryScanSessions().catch((error) => {
          console.error(
            "[library-scan] failed to reconcile started scan session:",
            error,
          );
        });
      }, delayMs),
    );
    const releaseTimer = window.setTimeout(() => {
      if (!cancelled) {
        setStartedLibraryScanSessionId(null);
      }
    }, 4_000);

    return () => {
      cancelled = true;
      timers.forEach((timer) => window.clearTimeout(timer));
      window.clearTimeout(releaseTimer);
    };
  }, [
    refreshLibraryScanSessions,
    startedLibraryScanSession,
    startedLibraryScanSessionId,
  ]);

  React.useEffect(() => {
    setLibraryScanNotice(null);
  }, [activeFacet]);

  const refreshRuleSets = React.useCallback(async () => {
    setRulesLoading(true);
    try {
      const { data, error } = await client.query(ruleSetsQuery, {}).toPromise();
      if (error) throw error;
      setRuleSets(data.ruleSets || []);
    } catch {
      // silent — rules panel is non-critical
    } finally {
      setRulesLoading(false);
    }
  }, [client]);

  const onToggleRuleFacet = React.useCallback(
    async (ruleSetId: string, enabled: boolean) => {
      const rule = ruleSets.find((r) => r.id === ruleSetId);
      if (!rule) return;

      const nextFacets = enabled
        ? [...rule.appliedFacets, activeFacet]
        : rule.appliedFacets.filter((f) => f !== activeFacet);

      setRulesSaving(true);
      try {
        const { error } = await client
          .mutation(updateRuleSetMutation, {
            input: {
              id: ruleSetId,
              name: rule.name,
              description: rule.description,
              regoSource: rule.regoSource,
              priority: rule.priority,
              appliedFacets: nextFacets,
            },
          })
          .toPromise();
        if (error) throw error;
        setGlobalStatus(
          t("status.ruleToggled", {
            name: rule.name,
            state: enabled ? t("label.enabled") : t("label.disabled"),
          }),
        );
        await refreshRuleSets();
      } catch (error) {
        setGlobalStatus(
          error instanceof Error ? error.message : t("status.failedToUpdate"),
        );
      } finally {
        setRulesSaving(false);
      }
    },
    [activeFacet, client, refreshRuleSets, ruleSets, setGlobalStatus, t],
  );
  const reloadTitles = React.useCallback(
    async (
      queryOverride?: string,
      libraryIdsOverride?: string[],
    ): Promise<TitleRecord[] | null> => {
      setTitleLoading(true);
      setTitleStatus(t("title.loading"));
      const query = (queryOverride ?? activeCatalogQueryRef.current).trim();
      const libraryIds = libraryIdsOverride ?? selectedLibraryIds;
      const queryKey = titleCatalogQueryKey({
        facet: activeFacet,
        query,
        libraryIds,
        filters: effectiveTitleQuickFilters,
        sort: effectiveTitleCatalogSort,
      });
      activeCatalogListFiltersRef.current = buildActiveCatalogListFilters(
        activeFacet,
        query,
        libraryIds,
      );
      const requestSeq = ++catalogTitleRequestSeqRef.current;
      catalogPageLoadInFlightRef.current = false;
      catalogQueryKeyRef.current = queryKey;
      setCatalogPaginationState({ ...emptyTitleCatalogState, queryKey });

      try {
        const { data, error } = await client
          .query(
            titlesQuery,
            buildTitleCatalogQueryVariables({
              facet: activeFacet,
              libraryIds,
              query,
              filters: effectiveTitleQuickFilters,
              sort: effectiveTitleCatalogSort,
              limit: TITLE_CATALOG_PAGE_SIZE,
              offset: 0,
            }),
            { requestPolicy: "network-only" },
          )
          .toPromise();
        if (error) {
          throw error;
        }
        if (
          requestSeq !== catalogTitleRequestSeqRef.current ||
          catalogQueryKeyRef.current !== queryKey
        ) {
          return null;
        }

        const page = data?.titles ?? {};
        const nextTitles = (page.items ?? []) as TitleRecord[];
        setMonitoredTitles((current) =>
          mergeCatalogTitlesPreservingImages(current, nextTitles),
        );
        mergeTitleContextTitles(nextTitles);
        setCatalogPaginationState({
          queryKey,
          hasMore: Boolean(page.hasMore),
          nextOffset:
            typeof page.offset === "number" && typeof page.limit === "number"
              ? page.offset + nextTitles.length
              : nextTitles.length,
          totalCount:
            typeof page.totalCount === "number"
              ? page.totalCount
              : nextTitles.length,
          loadingMore: false,
        });
        setTitleStatus(
          t("title.statusTemplate", {
            count:
              typeof page.totalCount === "number"
                ? page.totalCount
                : nextTitles.length,
          }),
        );
        return nextTitles;
      } catch (error) {
        if (requestSeq !== catalogTitleRequestSeqRef.current) {
          return null;
        }
        setTitleStatus(
          error instanceof Error ? error.message : t("status.failedToLoad"),
        );
        return null;
      } finally {
        if (requestSeq === catalogTitleRequestSeqRef.current) {
          setTitleLoading(false);
        }
      }
    },
    [
      activeFacet,
      client,
      effectiveTitleQuickFilters,
      effectiveTitleCatalogSort,
      mergeTitleContextTitles,
      selectedLibraryIds,
      setMonitoredTitles,
      setTitleLoading,
      setTitleStatus,
      t,
    ],
  );

  const refreshTitles = React.useCallback(
    async (query?: string) => {
      await reloadTitles(query ?? titleFilter);
    },
    [reloadTitles, titleFilter],
  );

  const handleCatalogDiscoveryAction = React.useCallback(
    async (item: CatalogDiscoveryItem) => {
      if (item.ownedInInput) {
        return;
      }
      if (!canManageTitle && !canRequestMedia) {
        setGlobalStatus(t("status.permissionDenied"));
        return;
      }
      const facet = discoveryItemFacet(item);
      if (!facet) {
        setGlobalStatus(t("status.apiError"));
        return;
      }
      try {
        await ensureCatalogConfigReady(facet);
        const { data, error } = await client
          .query(
            discoveryItemDetailQuery,
            {
              input: {
                targetKey: item.targetKey,
                includeUnresolved: true,
              },
            },
            { requestPolicy: "network-only" },
          )
          .toPromise();
        if (error) {
          throw error;
        }
        const detailItem =
          (data?.discoveryItemDetail as CatalogDiscoveryItem | null | undefined) ??
          item;
        const target = {
          result: metadataResultForDiscoveryItem(detailItem),
          facet,
        };
        if (canManageTitle) {
          setAddDiscoveryDialogTarget(target);
        } else {
          setRequestDiscoveryDialogTarget(target);
        }
      } catch (caught) {
        setGlobalStatus(
          caught instanceof Error ? caught.message : t("status.apiError"),
        );
      }
    },
    [
      canManageTitle,
      canRequestMedia,
      client,
      ensureCatalogConfigReady,
      setGlobalStatus,
      t,
    ],
  );

  const loadMoreCatalogTitles = React.useCallback(async () => {
    if (
      !shouldLoadCatalogTitles ||
      !catalogPaginationState.hasMore ||
      catalogPaginationState.loadingMore ||
      catalogPageLoadInFlightRef.current
    ) {
      return;
    }

    const requestSeq = catalogTitleRequestSeqRef.current;
    const query = activeCatalogQueryRef.current.trim();
    const queryKey = titleCatalogQueryKey({
      facet: activeFacet,
      query,
      libraryIds: selectedLibraryIds,
      filters: effectiveTitleQuickFilters,
      sort: effectiveTitleCatalogSort,
    });
    if (catalogPaginationState.queryKey !== queryKey) {
      return;
    }
    const offset = catalogPaginationState.nextOffset;
    catalogPageLoadInFlightRef.current = true;
    setCatalogPaginationState((current) => ({ ...current, loadingMore: true }));

    try {
      const { data, error } = await client
        .query(
          titlesQuery,
          buildTitleCatalogQueryVariables({
            facet: activeFacet,
            libraryIds: selectedLibraryIds,
            query,
            filters: effectiveTitleQuickFilters,
            sort: effectiveTitleCatalogSort,
            limit: TITLE_CATALOG_PAGE_SIZE,
            offset,
          }),
          { requestPolicy: "network-only" },
        )
        .toPromise();
      if (error) {
        throw error;
      }
      if (
        requestSeq !== catalogTitleRequestSeqRef.current ||
        catalogQueryKeyRef.current !== queryKey
      ) {
        return;
      }

      const page = data?.titles ?? {};
      const nextTitles = (page.items ?? []) as TitleRecord[];
      setMonitoredTitles((current) =>
        appendCatalogTitlesPreservingImages(current, nextTitles),
      );
      mergeTitleContextTitles(nextTitles);
      setCatalogPaginationState({
        queryKey,
        hasMore: Boolean(page.hasMore),
        nextOffset:
          typeof page.offset === "number"
            ? page.offset + nextTitles.length
            : offset + nextTitles.length,
        totalCount:
          typeof page.totalCount === "number"
            ? page.totalCount
            : catalogPaginationState.totalCount,
        loadingMore: false,
      });
    } catch (error) {
      if (
        requestSeq === catalogTitleRequestSeqRef.current &&
        catalogQueryKeyRef.current === queryKey
      ) {
        setTitleStatus(
          error instanceof Error ? error.message : t("status.failedToLoad"),
        );
      }
    } finally {
      if (
        requestSeq === catalogTitleRequestSeqRef.current &&
        catalogQueryKeyRef.current === queryKey
      ) {
        catalogPageLoadInFlightRef.current = false;
        setCatalogPaginationState((current) => ({
          ...current,
          loadingMore: false,
        }));
      }
    }
  }, [
    activeFacet,
    catalogPaginationState.hasMore,
    catalogPaginationState.loadingMore,
    catalogPaginationState.nextOffset,
    catalogPaginationState.queryKey,
    catalogPaginationState.totalCount,
    client,
    effectiveTitleQuickFilters,
    effectiveTitleCatalogSort,
    mergeTitleContextTitles,
    selectedLibraryIds,
    setMonitoredTitles,
    setTitleStatus,
    shouldLoadCatalogTitles,
    t,
  ]);

  const recordCriticalCatalogMutation = React.useCallback(() => {
    latestCriticalMutationEpochRef.current = reactiveRefreshEpoch();
  }, []);

  const clearDeletionFallbackTimers = React.useCallback(() => {
    for (const timer of deletionFallbackTimersRef.current) {
      clearTimeout(timer);
    }
    deletionFallbackTimersRef.current = [];
  }, []);

  const handleTitleDeletionJobSnapshot = React.useCallback(
    (run: JobRun | null) => {
      if (
        !run ||
        run.jobKey !== "title_deletion" ||
        !deletionJobIdsRef.current.has(run.id) ||
        !isTerminalJobRunStatus(run.status)
      ) {
        return false;
      }

      deletionJobIdsRef.current.delete(run.id);
      if (deletionJobIdsRef.current.size === 0) {
        clearDeletionFallbackTimers();
        setPendingDeletedTitleIds(new Set());
      }
      void refreshTitles();
      return true;
    },
    [clearDeletionFallbackTimers, refreshTitles],
  );

  const refreshTrackedDeletionJobs = React.useCallback(async () => {
    if (deletionJobIdsRef.current.size === 0) {
      return;
    }

    try {
      const { data, error } = await client
        .query<{
          jobRuns?: unknown[];
        }>(
          jobRunsQuery,
          { jobKey: "title_deletion", limit: 10 },
          { requestPolicy: "network-only" },
        )
        .toPromise();
      if (error) {
        throw error;
      }

      for (const rawRun of data?.jobRuns ?? []) {
        if (handleTitleDeletionJobSnapshot(normalizeJobRun(rawRun))) {
          break;
        }
      }
    } catch (error) {
      console.error("[title-deletion-job-runs] refresh failed:", error);
    }
  }, [client, handleTitleDeletionJobSnapshot]);

  const scheduleDeletionJobFallbackChecks = React.useCallback(() => {
    clearDeletionFallbackTimers();
    deletionFallbackTimersRef.current =
      TITLE_DELETION_JOB_FALLBACK_DELAYS_MS.map((delayMs) =>
        setTimeout(() => {
          void refreshTrackedDeletionJobs();
        }, delayMs),
      );
  }, [clearDeletionFallbackTimers, refreshTrackedDeletionJobs]);

  React.useEffect(
    () => clearDeletionFallbackTimers,
    [clearDeletionFallbackTimers],
  );

  React.useEffect(() => {
    const refreshIfTrackingDeletion = () => {
      if (deletionJobIdsRef.current.size > 0) {
        void refreshTrackedDeletionJobs();
      }
    };
    const handleVisibilityChange = () => {
      if (document.visibilityState === "visible") {
        refreshIfTrackingDeletion();
      }
    };

    window.addEventListener("focus", refreshIfTrackingDeletion);
    document.addEventListener("visibilitychange", handleVisibilityChange);
    return () => {
      window.removeEventListener("focus", refreshIfTrackingDeletion);
      document.removeEventListener("visibilitychange", handleVisibilityChange);
    };
  }, [refreshTrackedDeletionJobs]);

  const {
    bulkDeleteDialogOpen,
    setBulkDeleteDialogOpen,
    bulkDeleteFilesOnDisk,
    setBulkDeleteFilesOnDisk,
    bulkDeleteTypedConfirmation,
    setBulkDeleteTypedConfirmation,
    bulkDeletePreviewLoading,
    setBulkDeletePreviewLoading,
    bulkDeletePreviewError,
    setBulkDeletePreviewError,
    setBulkDeletePreviewsByTitleId,
    closeBulkDeleteDialog,
    bulkDeletePreview,
    bulkDeleteConfirmDisabled,
    confirmBulkDeleteTitles,
    openBulkTitleDelete,
  } = useBulkDelete({
    selectedTitles,
    selectedTitleLibraryIds,
    bulkActionBusy,
    setBulkActionBusy,
    client,
    t,
    setGlobalStatus,
    recordCriticalCatalogMutation,
    registerInteractiveJobRun,
    scheduleDeletionJobFallbackChecks,
    setPendingDeletedTitleIds,
    setSelectedTitleIds,
    deletionJobIdsRef,
    batchFailureDetail,
    withFailureDetail,
    aggregateDeletePreviews,
  });

  React.useEffect(() => {
    if (selectedTitles.length > 0) {
      return;
    }
    setBulkEditDialogOpen(false);
    setBulkDeleteDialogOpen(false);
    setBulkDeleteFilesOnDisk(false);
    setBulkDeleteTypedConfirmation("");
    setBulkDeletePreviewLoading(false);
    setBulkDeletePreviewError(null);
    setBulkDeletePreviewsByTitleId({});
  }, [selectedTitles.length]);

  useDeferredWsSubscription<{ data?: { jobRunEvents?: unknown } }>({
    requestKey: "mediaContentTitleDeletionJobRuns",
    request: { query: jobRunEventsSubscription },
    onNext(result) {
      handleTitleDeletionJobSnapshot(
        normalizeJobRun(result.data?.jobRunEvents),
      );
    },
    onError(error) {
      console.error("[title-deletion-job-runs] subscription error:", error);
    },
  });

  const applyRefreshedTitleRecord = React.useCallback(
    (titleId: string, title: TitleRecord | null, requestEpoch: number) => {
      if (requestEpoch <= latestCriticalMutationEpochRef.current) {
        return;
      }

      setTitleContextTitles((current) => {
        if (!title) {
          return current.filter((item) => item.id !== titleId);
        }
        return upsertCatalogTitleRecord(current, title);
      });

      setMonitoredTitles((current) => {
        const next = [...current];
        const existingIndex = next.findIndex((item) => item.id === titleId);

        if (!title) {
          if (existingIndex !== -1) {
            next.splice(existingIndex, 1);
          }
          setTitleStatus(t("title.statusTemplate", { count: next.length }));
          return next;
        }

        if (
          !catalogTitleMatchesActiveListFilters(
            title,
            activeCatalogListFiltersRef.current,
          )
        ) {
          if (existingIndex === -1) {
            return current;
          }
          next.splice(existingIndex, 1);
          setTitleStatus(t("title.statusTemplate", { count: next.length }));
          return next;
        }

        if (existingIndex === -1) {
          next.push(title);
        } else {
          next[existingIndex] = mergePreferLoadedImageFields(
            next[existingIndex],
            title,
          );
        }
        setTitleStatus(t("title.statusTemplate", { count: next.length }));
        return next;
      });
    },
    [setMonitoredTitles, setTitleStatus, t],
  );

  const pendingHydrationPosterTitleIds = React.useMemo(() => {
    const nowMs = Date.now();
    return monitoredTitles
      .filter((title) => isPendingHydrationPosterTitle(title, nowMs))
      .map((title) => title.id);
  }, [monitoredTitles]);
  const pendingHydrationPosterTitleIdsKey = React.useMemo(
    () => pendingHydrationPosterTitleIds.join("|"),
    [pendingHydrationPosterTitleIds],
  );

  const refreshTitlePanelDetail = React.useCallback(
    async (titleId: string) => {
      const requestEpoch = reactiveRefreshEpoch();
      const currentTitle =
        monitoredTitles.find((title) => title.id === titleId) ?? null;
      const detailResult = await client
        .query<{ title?: TitleRecord | null }>(
          titlePanelDetailQueryForTitle(currentTitle),
          { id: titleId },
          { requestPolicy: "network-only" },
        )
        .toPromise();
      if (detailResult.error) {
        throw detailResult.error;
      }
      if (detailResult.data?.title) {
        applyRefreshedTitleRecord(
          titleId,
          detailResult.data.title,
          requestEpoch,
        );
      }
    },
    [applyRefreshedTitleRecord, client, monitoredTitles],
  );

  // When a slug deep link selects a title that isn't part of the currently
  // loaded catalog page, fetch its detail so the inline overview pane can render
  // it instead of the list bouncing back. A genuine miss closes to the list.
  const titleContextSourceTitlesRef = React.useRef(titleContextSourceTitles);
  titleContextSourceTitlesRef.current = titleContextSourceTitles;
  React.useEffect(() => {
    const titleId = routeOverviewTitleId;
    if (!titleId) {
      setSelectedOverviewDetailLoading(false);
      return;
    }
    if (
      titleContextSourceTitlesRef.current.some((title) => title.id === titleId)
    ) {
      setSelectedOverviewDetailLoading(false);
      return;
    }
    let cancelled = false;
    setSelectedOverviewDetailLoading(true);
    void refreshTitlePanelDetail(titleId)
      .catch(() => {
        if (!cancelled) {
          onCloseOverview();
        }
      })
      .finally(() => {
        if (!cancelled) {
          setSelectedOverviewDetailLoading(false);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [routeOverviewTitleId, refreshTitlePanelDetail, onCloseOverview]);

  const previewTitleRename = React.useCallback(
    async (title: TitleRecord): Promise<MediaRenamePlan | null> => {
      try {
        const { data, error } = await client
          .query<{ mediaRenamePreview: MediaRenamePlan }>(
            mediaRenamePreviewQuery,
            {
              input: {
                facet: title.facet,
                titleId: title.id,
                dryRun: true,
              },
            },
          )
          .toPromise();
        if (error) {
          throw error;
        }

        const plan = data?.mediaRenamePreview ?? null;
        if (plan) {
          setGlobalStatus(
            t("status.renamePreviewGenerated", {
              total: plan.total,
              renamable: plan.renamable,
            }),
          );
        }
        return plan;
      } catch (error) {
        setGlobalStatus(
          error instanceof Error ? error.message : t("status.apiError"),
        );
        return null;
      }
    },
    [client, setGlobalStatus, t],
  );

  const applyTitleRename = React.useCallback(
    async (title: TitleRecord, plan: MediaRenamePlan) => {
      try {
        recordCriticalCatalogMutation();
        const { data, error } = await client
          .mutation<{
            applyMediaRename: {
              applied: number;
              skipped: number;
              failed: number;
            };
          }>(applyMediaRenameMutation, {
            input: {
              facet: title.facet,
              titleId: title.id,
              fingerprint: plan.fingerprint,
            },
          })
          .toPromise();
        if (error) {
          throw error;
        }

        const result = data?.applyMediaRename;
        setGlobalStatus(
          t("status.renameApplied", {
            applied: result?.applied ?? 0,
            skipped: result?.skipped ?? 0,
            failed: result?.failed ?? 0,
          }),
        );
        await refreshTitlePanelDetail(title.id);
        return true;
      } catch (error) {
        setGlobalStatus(
          error instanceof Error ? error.message : t("status.apiError"),
        );
        return false;
      }
    },
    [
      client,
      recordCriticalCatalogMutation,
      refreshTitlePanelDetail,
      setGlobalStatus,
      t,
    ],
  );

  const requestDeleteSelectedOverviewMediaFile = React.useCallback(
    (title: TitleRecord, fileId: string) => {
      const file =
        title.mediaFiles?.find((candidate) => candidate.id === fileId) ?? null;
      if (!file) {
        return;
      }
      setSelectedOverviewMediaFileToDelete({
        titleId: title.id,
        file,
      });
      setSelectedOverviewMediaFileDeleteTypedConfirmation("");
    },
    [],
  );

  const makeSelectedOverviewMovieFilePrimary = React.useCallback(
    async (title: TitleRecord, fileId: string) => {
      if (title.facet !== "movie") {
        return;
      }
      setSelectedOverviewPrimaryMovieFileUpdatingId(fileId);
      try {
        recordCriticalCatalogMutation();
        const { error } = await client
          .mutation(setPrimaryMovieFileMutation, {
            input: {
              titleId: title.id,
              fileId,
            },
          })
          .toPromise();
        if (error) {
          throw error;
        }
        setGlobalStatus(t("status.primaryMovieFileUpdated"));
        await refreshTitlePanelDetail(title.id);
      } catch (error) {
        setGlobalStatus(
          userFacingGraphQlErrorMessage(error, t("status.apiError")),
        );
      } finally {
        setSelectedOverviewPrimaryMovieFileUpdatingId(null);
      }
    },
    [
      client,
      recordCriticalCatalogMutation,
      refreshTitlePanelDetail,
      setGlobalStatus,
      t,
    ],
  );

  const selectedOverviewTitleForPanelHydration = React.useMemo(
    () =>
      selectedOverviewTitleId
        ? (titleContextSourceTitles.find(
            (title) => title.id === selectedOverviewTitleId,
          ) ?? null)
        : null,
    [selectedOverviewTitleId, titleContextSourceTitles],
  );
  const selectedPanelHydrationTitleId =
    selectedOverviewTitleForPanelHydration?.id ?? null;
  const selectedPanelHydrationIsRouteOverview =
    selectedPanelHydrationTitleId !== null &&
    selectedPanelHydrationTitleId === routeOverviewTitleId;
  const selectedPanelHydrationMetadataFetchedAt =
    selectedOverviewTitleForPanelHydration?.metadataFetchedAt ?? "";
  const selectedPanelHydrationCreatedAt =
    selectedOverviewTitleForPanelHydration?.createdAt ?? "";
  const selectedPanelNeedsPanelDetails =
    selectedOverviewTitleForPanelHydration !== null
      ? !hasSelectedTitlePanelDetails(selectedOverviewTitleForPanelHydration)
      : false;
  const selectedPanelNeedsEpisodeDetails =
    selectedOverviewTitleForPanelHydration !== null
      ? !hasSelectedTitleEpisodeDetails(selectedOverviewTitleForPanelHydration)
      : false;

  const activeCatalogDiscoveryGroups = catalogDiscoveryGroups;

  const loadSelectedOverviewExternalSubtitles = React.useCallback(
    async (titleId: string) => {
      const { data, error } = await client
        .query<{ externalSubtitles?: ExternalSubtitleRecord[] }>(
          externalSubtitlesQuery,
          { titleId },
          { requestPolicy: "network-only" },
        )
        .toPromise();
      if (error) {
        throw error;
      }
      return data?.externalSubtitles ?? [];
    },
    [client],
  );
  const refreshSelectedOverviewExternalSubtitles = React.useCallback(
    async () => {
      const titleId = selectedPanelHydrationTitleId;
      if (
        !shouldLoadCatalogTitles ||
        !titleId ||
        selectedPanelHydrationIsRouteOverview
      ) {
        setSelectedOverviewExternalSubtitleState({
          titleId: null,
          entries: [],
        });
        return;
      }

      try {
        const entries = await loadSelectedOverviewExternalSubtitles(titleId);
        setSelectedOverviewExternalSubtitleState((current) =>
          current.titleId === titleId ? { titleId, entries } : current,
        );
      } catch (error) {
        console.error(
          "[selected-title-external-subtitles-refresh] refresh failed:",
          error,
        );
        setSelectedOverviewExternalSubtitleState((current) =>
          current.titleId === titleId ? { titleId, entries: [] } : current,
        );
      }
    },
    [
      loadSelectedOverviewExternalSubtitles,
      selectedPanelHydrationIsRouteOverview,
      selectedPanelHydrationTitleId,
      shouldLoadCatalogTitles,
    ],
  );

  React.useEffect(() => {
    if (
      !shouldLoadCatalogTitles ||
      !selectedPanelHydrationTitleId ||
      selectedPanelHydrationIsRouteOverview
    ) {
      selectedPanelHydrationKeyRef.current = null;
      return;
    }

    if (!selectedPanelNeedsPanelDetails && !selectedPanelNeedsEpisodeDetails) {
      selectedPanelHydrationKeyRef.current = null;
      return;
    }

    const titleId = selectedPanelHydrationTitleId;
    const requestKey = [
      titleId,
      selectedPanelHydrationMetadataFetchedAt,
      selectedPanelHydrationCreatedAt,
      selectedPanelNeedsEpisodeDetails ? "episodes" : "panel",
    ].join(":");
    if (selectedPanelHydrationKeyRef.current === requestKey) {
      return;
    }
    selectedPanelHydrationKeyRef.current = requestKey;

    let cancelled = false;
    const requestEpoch = reactiveRefreshEpoch();
    const detailQuery = titlePanelDetailQueryForTitle(
      selectedOverviewTitleForPanelHydration,
    );
    void client
      .query<{ title?: TitleRecord | null }>(
        detailQuery,
        { id: titleId },
        { requestPolicy: "network-only" },
      )
      .toPromise()
      .then(({ data, error }) => {
        if (cancelled) {
          return;
        }
        if (error) {
          console.error(
            "[selected-title-panel-refresh] refresh failed:",
            error,
          );
          if (selectedPanelHydrationKeyRef.current === requestKey) {
            selectedPanelHydrationKeyRef.current = null;
          }
          return;
        }
        if (data?.title) {
          applyRefreshedTitleRecord(titleId, data.title, requestEpoch);
        }
      })
      .catch((error: unknown) => {
        if (!cancelled) {
          console.error(
            "[selected-title-panel-refresh] refresh failed:",
            error,
          );
          if (selectedPanelHydrationKeyRef.current === requestKey) {
            selectedPanelHydrationKeyRef.current = null;
          }
        }
      });

    return () => {
      cancelled = true;
      if (selectedPanelHydrationKeyRef.current === requestKey) {
        selectedPanelHydrationKeyRef.current = null;
      }
    };
  }, [
    applyRefreshedTitleRecord,
    client,
    selectedPanelHydrationCreatedAt,
    selectedPanelHydrationMetadataFetchedAt,
    selectedPanelHydrationTitleId,
    selectedPanelHydrationIsRouteOverview,
    selectedPanelNeedsEpisodeDetails,
    selectedPanelNeedsPanelDetails,
    selectedOverviewTitleForPanelHydration,
    shouldLoadCatalogTitles,
  ]);

  React.useEffect(() => {
    if (
      !shouldLoadCatalogTitles ||
      !selectedPanelHydrationTitleId ||
      selectedPanelHydrationIsRouteOverview
    ) {
      setSelectedOverviewExternalSubtitleState({ titleId: null, entries: [] });
      return;
    }

    let cancelled = false;
    const titleId = selectedPanelHydrationTitleId;
    setSelectedOverviewExternalSubtitleState((current) =>
      current.titleId === titleId ? current : { titleId, entries: [] },
    );
    void loadSelectedOverviewExternalSubtitles(titleId)
      .then((entries) => {
        if (!cancelled) {
          setSelectedOverviewExternalSubtitleState({ titleId, entries });
        }
      })
      .catch((error: unknown) => {
        if (!cancelled) {
          console.error(
            "[selected-title-external-subtitles-refresh] refresh failed:",
            error,
          );
          setSelectedOverviewExternalSubtitleState((current) =>
            current.titleId === titleId ? { titleId, entries: [] } : current,
          );
        }
      });

    return () => {
      cancelled = true;
    };
  }, [
    loadSelectedOverviewExternalSubtitles,
    selectedPanelHydrationIsRouteOverview,
    selectedPanelHydrationTitleId,
    shouldLoadCatalogTitles,
  ]);

  React.useEffect(() => {
    if (!shouldLoadCatalogTitles || !selectedPanelHydrationTitleId) {
      setSelectedOverviewBlocklistState({ titleId: null, entries: [] });
      return;
    }

    let cancelled = false;
    const titleId = selectedPanelHydrationTitleId;
    setSelectedOverviewBlocklistState((current) =>
      current.titleId === titleId ? current : { titleId, entries: [] },
    );
    void client
      .query<{ titleReleaseBlocklist?: TitleReleaseBlocklistEntry[] }>(
        titleReleaseBlocklistQuery,
        {
          titleId,
          limit: 6,
        },
      )
      .toPromise()
      .then(({ data, error }) => {
        if (cancelled) {
          return;
        }
        if (error) {
          console.error(
            "[selected-title-blocklist-refresh] refresh failed:",
            error,
          );
          setSelectedOverviewBlocklistState((current) =>
            current.titleId === titleId ? { titleId, entries: [] } : current,
          );
          return;
        }
        setSelectedOverviewBlocklistState({
          titleId,
          entries: data?.titleReleaseBlocklist ?? [],
        });
      })
      .catch((error: unknown) => {
        if (!cancelled) {
          console.error(
            "[selected-title-blocklist-refresh] refresh failed:",
            error,
          );
          setSelectedOverviewBlocklistState((current) =>
            current.titleId === titleId ? { titleId, entries: [] } : current,
          );
        }
      });

    return () => {
      cancelled = true;
    };
  }, [client, selectedPanelHydrationTitleId, shouldLoadCatalogTitles]);

  React.useEffect(() => {
    if (
      !shouldLoadCatalogTitles ||
      pendingHydrationPosterTitleIds.length === 0
    ) {
      return;
    }

    const refreshPendingHydrationPosters = () => {
      pendingHydrationPosterTitleIds.forEach((titleId) => {
        queueCatalogTitleRefresh({
          titleId,
          apply(title, requestEpoch) {
            applyRefreshedTitleRecord(titleId, title, requestEpoch);
          },
          onError(error) {
            console.error(
              "[catalog-hydration-poster-refresh] refresh failed:",
              error,
            );
          },
        });
      });
    };

    refreshPendingHydrationPosters();
    const intervalId = window.setInterval(
      refreshPendingHydrationPosters,
      HYDRATION_POSTER_REFRESH_INTERVAL_MS,
    );

    return () => {
      window.clearInterval(intervalId);
    };
  }, [
    applyRefreshedTitleRecord,
    pendingHydrationPosterTitleIds,
    pendingHydrationPosterTitleIdsKey,
    queueCatalogTitleRefresh,
    shouldLoadCatalogTitles,
  ]);

  const onAddSubmit = React.useCallback(
    async (event: React.FormEvent<HTMLFormElement>) => {
      event.preventDefault();
      if (!titleNameForQueue.trim()) {
        setGlobalStatus(t("status.titleRequired"));
        return;
      }
      if (!queueFacet) {
        setGlobalStatus(t("status.facetRequired"));
        return;
      }

      await runTvdbSearch(titleNameForQueue.trim());
    },
    [queueFacet, runTvdbSearch, setGlobalStatus, titleNameForQueue, t],
  );

  const addTvdbToCatalog = React.useCallback(
    async (candidate: MetadataTvdbSearchItem) => {
      const name = candidate.name.trim();
      if (!name) {
        setGlobalStatus(t("status.titleRequired"));
        return;
      }

      const tvdbId = String(candidate.tvdbId).trim();
      const imdbId = candidate.imdbId?.trim();
      const externalIds = [
        { source: "tvdb", value: tvdbId },
        ...(imdbId ? [{ source: "imdb", value: imdbId }] : []),
      ];

      const monitorType = monitoredForQueue ? "allEpisodes" : "none";
      try {
        const { data, error } = await client
          .mutation(addTitleMutation, {
            input: {
              name,
              facet: queueFacet,
              monitored: monitoredForQueue,
              tags: [],
              options: {
                monitorType,
                ...(queueFacet === "movie"
                  ? {}
                  : { useSeasonFolders: seasonFoldersForQueue }),
                ...(queueFacet === "anime"
                  ? {
                      monitorSpecials: false,
                      interSeasonMovies: true,
                    }
                  : {}),
              },
              externalIds,
              ...(queueFacet === "movie"
                ? { minAvailability: minAvailabilityForQueue }
                : {}),
            },
          })
          .toPromise();
        if (error) throw error;
        setTitleNameForQueue(data.addTitle.title.name);
        setGlobalStatus(
          t(
            monitoredForQueue
              ? "status.catalogAddSuccessAutoSearch"
              : "status.catalogAddSuccess",
            { name: data.addTitle.title.name },
          ),
        );
        if (shouldLoadCatalogTitles && data?.addTitle?.title) {
          mergeTitleContextTitles([data.addTitle.title as TitleRecord]);
          setMonitoredTitles((current) => {
            const title = data.addTitle.title as TitleRecord;
            const next = upsertCatalogTitleRecord(
              current,
              title,
              activeCatalogListFiltersRef.current,
            );
            if (next !== current) {
              setTitleStatus(t("title.statusTemplate", { count: next.length }));
            }
            return next;
          });
        }
      } catch (error) {
        setGlobalStatus(
          error instanceof Error ? error.message : t("status.queueFailed"),
        );
      }
    },
    [
      minAvailabilityForQueue,
      monitoredForQueue,
      queueFacet,
      client,
      mergeTitleContextTitles,
      shouldLoadCatalogTitles,
      setMonitoredTitles,
      setGlobalStatus,
      setTitleStatus,
      t,
      seasonFoldersForQueue,
      setTitleNameForQueue,
    ],
  );

  const queueExisting = React.useCallback(
    async (title: TitleRecord) => {
      try {
        const input = {
          titleId: title.id,
          scope: { title: true },
        };
        const payload = await retryWithReplaceOnConflict(
          input,
          async (nextInput) => {
            const { data, error } = await client
              .mutation(queueBestReleaseMutation, { input: nextInput })
              .toPromise();
            if (error) throw error;
            return data?.queueBestRelease;
          },
          "A download is already in progress for this title.",
          confirmReplaceConflict,
        );
        assertNoReplaceConflict(
          payload,
          "A download is already in progress for this title.",
        );
        const queuedMessage = t("status.queuedLatest", { name: title.name });
        setGlobalStatus(queuedMessage);
      } catch (error) {
        setGlobalStatus(
          userFacingGraphQlErrorMessage(error, t("status.queueFailed")),
        );
      }
    },
    [client, confirmReplaceConflict, setGlobalStatus, t],
  );

  const runInteractiveSearchForTitle = React.useCallback(
    async (title: TitleRecord) => {
      interactiveSearchAbortRef.current?.abort();
      const abortController = new AbortController();
      interactiveSearchAbortRef.current = abortController;

      try {
        const { data, error } = await client
          .query(searchForTitleQuery, { titleId: title.id }, {
            fetch: makeAbortableFetch(abortController.signal),
          })
          .toPromise();
        if (abortController.signal.aborted) {
          return [];
        }
        if (error) throw error;
        return (data?.searchReleases ?? []) as Release[];
      } catch (error) {
        if (abortController.signal.aborted || isAbortError(error)) {
          return [];
        }
        setGlobalStatus(
          error instanceof Error ? error.message : t("status.searchFailed"),
        );
        return [];
      } finally {
        if (interactiveSearchAbortRef.current === abortController) {
          interactiveSearchAbortRef.current = null;
        }
      }
    },
    [client, setGlobalStatus, t],
  );

  const queueExistingFromRelease = React.useCallback(
    async (title: TitleRecord, release: Release) => {
      if (!release.candidateToken) {
        const message = t("status.releaseMissingCandidateToken");
        setGlobalStatus(message);
        throw new Error(message);
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
            const { data, error } = await client
              .mutation(queueExistingMutation, { input: nextInput })
              .toPromise();
            if (error) throw error;
            return data?.queueExistingTitleDownload;
          },
          "A download is already in progress for this title.",
          confirmReplaceConflict,
        );
        assertNoReplaceConflict(
          payload,
          "A download is already in progress for this title.",
        );
        const queuedMessage = t("status.queuedLatest", { name: title.name });
        setGlobalStatus(queuedMessage);
      } catch (error) {
        setGlobalStatus(
          userFacingGraphQlErrorMessage(error, t("status.queueFailed")),
        );
        throw error;
      }
    },
    [client, confirmReplaceConflict, setGlobalStatus, t],
  );

  const queueAdditionalFromRelease = React.useCallback(
    async (title: TitleRecord, release: Release) => {
      if (!release.candidateToken) {
        const message = t("status.releaseMissingCandidateToken");
        setGlobalStatus(message);
        throw new Error(message);
      }

      try {
        const { data, error } = await client
          .mutation(queueExistingMutation, {
            input: {
              titleId: title.id,
              scope: releaseQueueScopeInput(release, { title: true }),
              candidateToken: release.candidateToken,
              purpose: "ADDITIONAL_FILE",
            },
          })
          .toPromise();
        if (error) throw error;
        assertNoReplaceConflict(
          data?.queueExistingTitleDownload,
          "A download is already in progress for this title.",
        );
        setGlobalStatus(t("status.queuedLatest", { name: title.name }));
      } catch (error) {
        setGlobalStatus(
          userFacingGraphQlErrorMessage(error, t("status.queueFailed")),
        );
        throw error;
      }
    },
    [client, setGlobalStatus, t],
  );

  const toggleTitleMonitored = React.useCallback(
    async (title: TitleRecord, monitored: boolean) => {
      const titleId = title.id;
      setTitleMonitoringLoadingById((previous) => ({
        ...previous,
        [titleId]: true,
      }));
      try {
        const { error } = await client
          .mutation(setTitleMonitoredMutation, {
            input: { titleId, monitored },
          })
          .toPromise();
        if (error) throw error;
        setMonitoredTitles((previous) =>
          previous.map((item) =>
            item.id === titleId ? { ...item, monitored } : item,
          ),
        );
        setTitleContextTitles((previous) =>
          previous.map((item) =>
            item.id === titleId ? { ...item, monitored } : item,
          ),
        );
        setGlobalStatus(
          monitored
            ? t("status.titleMonitoringEnabled")
            : t("status.titleMonitoringDisabled"),
        );
      } catch (error) {
        setGlobalStatus(
          error instanceof Error ? error.message : t("status.apiError"),
        );
      } finally {
        setTitleMonitoringLoadingById((previous) => {
          const next = { ...previous };
          delete next[titleId];
          return next;
        });
      }
    },
    [client, setGlobalStatus, setMonitoredTitles, t],
  );

  const toggleTitleSelection = React.useCallback((titleId: string) => {
    setSelectedTitleIds((current) => {
      const next = new Set(current);
      if (next.has(titleId)) {
        next.delete(titleId);
      } else {
        next.add(titleId);
      }
      return next;
    });
  }, []);

  const toggleTitleQuickMonitoringFilter = React.useCallback(
    (nextFilter: "monitored" | "unmonitored") => {
      React.startTransition(() => {
        setTitleQuickFilters((current) => ({
          ...current,
          monitored: nextFilter === "monitored" ? !current.monitored : current.monitored,
          unmonitored:
            nextFilter === "unmonitored"
              ? !current.unmonitored
              : current.unmonitored,
        }));
      });
    },
    [],
  );

  const toggleTitleQuickStatusFilter = React.useCallback(
    (nextFilter: "continuing" | "ended") => {
      React.startTransition(() => {
        setTitleQuickFilters((current) => ({
          ...current,
          continuing:
            nextFilter === "continuing" ? !current.continuing : current.continuing,
          ended: nextFilter === "ended" ? !current.ended : current.ended,
        }));
      });
    },
    [],
  );

  const clearTitleQuickFilters = React.useCallback(() => {
    React.startTransition(() => {
      setTitleQuickFilters(EMPTY_TITLE_QUICK_FILTERS);
    });
  }, []);

  const updateTitleCatalogSort = React.useCallback(
    (nextKey: TitleTableSortKey) => {
      setTitleCatalogSort((current) => {
        if (current.key === nextKey) {
          return {
            key: nextKey,
            direction: current.direction === "asc" ? "desc" : "asc",
          };
        }

        return {
          key: nextKey,
          direction: defaultSortDirectionForTitleKey(nextKey),
        };
      });
    },
    [],
  );

  const toggleAllVisibleTitles = React.useCallback(
    (checked: boolean) => {
      setSelectedTitleIds(
        checked ? new Set(visibleTitles.map((title) => title.id)) : new Set(),
      );
    },
    [visibleTitles],
  );

  const clearSelectedTitles = React.useCallback(() => {
    setSelectedTitleIds((current) =>
      current.size === 0 ? current : new Set(),
    );
  }, []);

  const selectOverviewTitle = React.useCallback((titleId: string | null) => {
    setSelectedOverviewTitleId(titleId);
  }, []);

  const clearSelectedOverviewTitle = React.useCallback(() => {
    setSelectedOverviewTitleId(null);
  }, []);

  const setViewMode = React.useCallback(
    (nextMode: ContentViewMode) => {
      setDesktopViewModes((current) => ({
        ...current,
        [view]: nextMode,
      }));
    },
    [view],
  );

  const bulkMonitorTitles = React.useCallback(
    async (monitored: boolean) => {
      const targets = [...selectedTitles];
      if (targets.length === 0 || bulkActionBusy) {
        return;
      }

      setBulkActionBusy(true);
      try {
        const variables = Object.fromEntries(
          targets.map((title, index) => [
            `input${index}`,
            { titleId: title.id, monitored },
          ]),
        );
        const result = await client
          .mutation<
            Record<string, { id: string; monitored: boolean }>
          >(buildSetTitleMonitoredBatchMutation(targets.length), variables)
          .toPromise();
        const payload = result.data ?? {};
        const refreshedTitles = await reloadTitles();
        let { succeededIds, failedIds } = refreshedTitles
          ? inferMonitoredBatchOutcome(targets, refreshedTitles, monitored)
          : {
              succeededIds: [] as string[],
              failedIds: [...targets.map((title) => title.id)],
            };
        if (!refreshedTitles && !result.error) {
          succeededIds = [];
          failedIds = [];
          targets.forEach((title, index) => {
            if (payload[batchItemAlias(index)]) {
              succeededIds.push(title.id);
            } else {
              failedIds.push(title.id);
            }
          });
        }
        setSelectedTitleIds(new Set(failedIds));

        const detail = batchFailureDetail(result.error);
        if (succeededIds.length === 0) {
          setGlobalStatus(
            withFailureDetail(
              monitored
                ? t("status.bulkMonitorFailed")
                : t("status.bulkUnmonitorFailed"),
              detail,
            ),
          );
          return;
        }

        if (failedIds.length > 0) {
          setGlobalStatus(
            withFailureDetail(
              monitored
                ? t("status.bulkMonitorPartial", {
                    count: succeededIds.length,
                    failed: failedIds.length,
                  })
                : t("status.bulkUnmonitorPartial", {
                    count: succeededIds.length,
                    failed: failedIds.length,
                  }),
              detail,
            ),
          );
          return;
        }

        setGlobalStatus(
          monitored
            ? t("status.bulkMonitorSuccess", { count: succeededIds.length })
            : t("status.bulkUnmonitorSuccess", { count: succeededIds.length }),
        );
      } catch (error) {
        setGlobalStatus(
          withFailureDetail(
            monitored
              ? t("status.bulkMonitorFailed")
              : t("status.bulkUnmonitorFailed"),
            batchFailureDetail(error),
          ),
        );
      } finally {
        setBulkActionBusy(false);
      }
    },
    [bulkActionBusy, client, reloadTitles, selectedTitles, setGlobalStatus, t],
  );

  const applyBulkTitleOptions = React.useCallback(
    async (changes: TitleOptionUpdates) => {
      const targets = [...selectedTitles];
      if (targets.length === 0 || bulkActionBusy) {
        return;
      }

      setBulkActionBusy(true);
      try {
        const variables = Object.fromEntries(
          targets.map((title, index) => [
            `input${index}`,
            {
              titleId: title.id,
              options: changes,
            },
          ]),
        );
        const result = await client
          .mutation<
            Record<string, { id: string }>
          >(buildUpdateTitleBatchMutation(targets.length), variables)
          .toPromise();
        const payload = result.data ?? {};
        const refreshedTitles = await reloadTitles();
        let { succeededIds, failedIds } = refreshedTitles
          ? inferTitleUpdateBatchOutcome(targets, refreshedTitles, changes)
          : {
              succeededIds: [] as string[],
              failedIds: [...targets.map((title) => title.id)],
            };
        if (!refreshedTitles && !result.error) {
          succeededIds = [];
          failedIds = [];
          targets.forEach((title, index) => {
            if (payload[batchItemAlias(index)]) {
              succeededIds.push(title.id);
            } else {
              failedIds.push(title.id);
            }
          });
        }
        setSelectedTitleIds(new Set(failedIds));

        const detail = batchFailureDetail(result.error);
        if (succeededIds.length === 0) {
          setGlobalStatus(
            withFailureDetail(t("status.bulkTitleUpdateFailed"), detail),
          );
          return;
        }

        setBulkEditDialogOpen(false);
        if (failedIds.length > 0) {
          setGlobalStatus(
            withFailureDetail(
              t("status.bulkTitleUpdatePartial", {
                count: succeededIds.length,
                failed: failedIds.length,
              }),
              detail,
            ),
          );
          return;
        }

        setGlobalStatus(
          t("status.bulkTitleUpdateSuccess", { count: succeededIds.length }),
        );
      } catch (error) {
        setGlobalStatus(
          withFailureDetail(
            t("status.bulkTitleUpdateFailed"),
            batchFailureDetail(error),
          ),
        );
      } finally {
        setBulkActionBusy(false);
      }
    },
    [bulkActionBusy, client, reloadTitles, selectedTitles, setGlobalStatus, t],
  );

  React.useEffect(() => {
    if (rootFolderValidationSnapshot === null) {
      setInvalidRootLibraryIds([]);
      setInvalidRootPathsByLibraryId({});
      setValidatedRootFolderSnapshotKey(null);
      return;
    }

    const { key, librariesWithConfiguredRoots } =
      rootFolderValidationSnapshot;

    if (librariesWithConfiguredRoots.length === 0) {
      setInvalidRootLibraryIds([]);
      setInvalidRootPathsByLibraryId({});
      setValidatedRootFolderSnapshotKey(key);
      return;
    }

    let cancelled = false;

    const validateRoots = async () => {
      const invalidIds = new Set<string>();
      const invalidPathsByLibraryId: Record<string, string[]> = {};

      await Promise.all(
        librariesWithConfiguredRoots.map(async (library) => {
          const configuredPaths = library.roots
            .map((root) => root.path.trim())
            .filter((path) => path.length > 0);
          if (configuredPaths.length === 0) {
            return;
          }

          const validationResults = await Promise.all(
            configuredPaths.map(async (path) => {
              const { error } = await client
                .query(
                  browsePathQuery,
                  { path },
                  { requestPolicy: "network-only" },
                )
                .toPromise();
              return error != null;
            }),
          );

          const invalidPaths = configuredPaths.filter(
            (_path, index) => validationResults[index],
          );

          if (invalidPaths.length > 0) {
            invalidIds.add(library.id);
            invalidPathsByLibraryId[library.id] = invalidPaths;
          }
        }),
      );

      if (!cancelled) {
        setInvalidRootLibraryIds([...invalidIds]);
        setInvalidRootPathsByLibraryId(invalidPathsByLibraryId);
        setValidatedRootFolderSnapshotKey(key);
      }
    };

    void validateRoots().catch((error) => {
      console.error(
        "[library-root-validation] failed to validate root folders:",
        error,
      );
      if (!cancelled) {
        setInvalidRootLibraryIds([]);
        setInvalidRootPathsByLibraryId({});
        setValidatedRootFolderSnapshotKey(key);
      }
    });

    return () => {
      cancelled = true;
    };
  }, [client, rootFolderValidationSnapshot]);

  React.useEffect(() => {
    if (!shouldLoadCatalogTitles || !canManageLibrarySettings) {
      setCatalogHasValidRoot(null);
      return;
    }

    let cancelled = false;
    setCatalogHasValidRoot(null);

    void client
      .query(
        catalogHasValidRootQuery,
        { facet: activeFacet },
        { requestPolicy: "network-only" },
      )
      .toPromise()
      .then(({ data, error }) => {
        if (cancelled) {
          return;
        }
        if (error) {
          console.error(
            "[catalog-root-validation] failed to validate catalog roots:",
            error,
          );
          setCatalogHasValidRoot(null);
          return;
        }
        setCatalogHasValidRoot(data?.catalogHasValidRoot ?? null);
      })
      .catch((error) => {
        if (cancelled) {
          return;
        }
        console.error(
          "[catalog-root-validation] failed to validate catalog roots:",
          error,
        );
        setCatalogHasValidRoot(null);
      });

    return () => {
      cancelled = true;
    };
  }, [activeFacet, canManageLibrarySettings, client, shouldLoadCatalogTitles]);

  const openBulkTitleEdit = React.useCallback(() => {
    if (selectedTitles.length === 0 || bulkActionBusy) {
      return;
    }
    if (selectedTitleLibraryIds.length !== 1) {
      setGlobalStatus("Bulk actions require titles from one library.");
      return;
    }
    setBulkEditDialogOpen(true);
  }, [
    bulkActionBusy,
    selectedTitleLibraryIds.length,
    selectedTitles.length,
    setGlobalStatus,
  ]);

  const requestDeleteTitle = React.useCallback(
    (title: TitleRecord) => {
      setTitleToDelete(title);
      setDeleteFilesOnDisk(false);
      setTitleDeleteTypedConfirmation("");
    },
    [setTitleDeleteTypedConfirmation, setTitleToDelete, setDeleteFilesOnDisk],
  );

  const closeDeleteTitleDialog = React.useCallback(() => {
    setTitleToDelete(null);
    setDeleteFilesOnDisk(false);
    setTitleDeleteTypedConfirmation("");
  }, [setDeleteFilesOnDisk, setTitleDeleteTypedConfirmation, setTitleToDelete]);

  React.useEffect(() => {
    if (!deleteFilesOnDisk) {
      setTitleDeleteTypedConfirmation("");
    }
  }, [deleteFilesOnDisk]);

  const confirmDeleteTitle = React.useCallback(async () => {
    if (!titleToDelete) {
      return;
    }

    const titleId = titleToDelete.id;
    setDeleteTitleLoadingById((previous) => ({
      ...previous,
      [titleId]: true,
    }));

    try {
      let previewFingerprint: string | undefined;
      if (deleteFilesOnDisk) {
        if (!titleDeletePreview) {
          throw new Error("Delete preview is not ready yet.");
        }
        previewFingerprint = titleDeletePreview.fingerprint;
      }

      const result = await client
        .mutation<{
          deleteTitles?: {
            acceptedTitleIds?: string[];
            jobRun?: unknown;
          };
        }>(deleteTitlesMutation, {
          input: {
            items: [
              {
                titleId,
                ...(deleteFilesOnDisk ? { previewFingerprint } : {}),
              },
            ],
            deleteFilesOnDisk,
            ...(deleteFilesOnDisk && titleDeleteTypedConfirmation.trim()
              ? { typedConfirmation: titleDeleteTypedConfirmation.trim() }
              : {}),
          },
        })
        .toPromise();
      if (result.error) throw result.error;
      const acceptedIds = result.data?.deleteTitles?.acceptedTitleIds ?? [];
      if (acceptedIds.length > 0) {
        recordCriticalCatalogMutation();
      }
      const run = normalizeJobRun(result.data?.deleteTitles?.jobRun);
      if (run) {
        deletionJobIdsRef.current.add(run.id);
        registerInteractiveJobRun(run);
        scheduleDeletionJobFallbackChecks();
      }
      setPendingDeletedTitleIds((current) => {
        const next = new Set(current);
        for (const id of acceptedIds) {
          next.add(id);
        }
        return next;
      });
      setGlobalStatus(`Queued deletion for ${titleToDelete.name}.`);
    } catch (error) {
      setGlobalStatus(
        error instanceof Error ? error.message : t("status.failedToDelete"),
      );
    } finally {
      setDeleteTitleLoadingById((previous) => {
        const next = { ...previous };
        delete next[titleId];
        return next;
      });
      closeDeleteTitleDialog();
    }
  }, [
    closeDeleteTitleDialog,
    deleteFilesOnDisk,
    client,
    recordCriticalCatalogMutation,
    registerInteractiveJobRun,
    scheduleDeletionJobFallbackChecks,
    titleDeletePreview,
    titleDeleteTypedConfirmation,
    t,
    titleToDelete,
    setGlobalStatus,
    setDeleteTitleLoadingById,
  ]);

  const closeSelectedOverviewMediaFileDeleteDialog = React.useCallback(() => {
    if (selectedOverviewMediaFileDeleteLoading) {
      return;
    }
    setSelectedOverviewMediaFileToDelete(null);
    setSelectedOverviewMediaFileDeleteTypedConfirmation("");
  }, [selectedOverviewMediaFileDeleteLoading]);

  const confirmDeleteSelectedOverviewMediaFile = React.useCallback(async () => {
    if (
      !selectedOverviewMediaFileToDelete ||
      !selectedOverviewMediaFileDeletePreview
    ) {
      return;
    }
    setSelectedOverviewMediaFileDeleteLoading(true);
    try {
      recordCriticalCatalogMutation();
      const { error } = await client
        .mutation(deleteMediaFileMutation, {
          input: {
            fileId: selectedOverviewMediaFileToDelete.file.id,
            deleteFromDisk: true,
            previewFingerprint: selectedOverviewMediaFileDeletePreview.fingerprint,
            typedConfirmation:
              selectedOverviewMediaFileDeleteTypedConfirmation.trim() ||
              undefined,
          },
        })
        .toPromise();
      if (error) {
        throw error;
      }
      await refreshTitlePanelDetail(selectedOverviewMediaFileToDelete.titleId);
      setSelectedOverviewMediaFileToDelete(null);
      setSelectedOverviewMediaFileDeleteTypedConfirmation("");
    } catch (error) {
      setGlobalStatus(
        userFacingGraphQlErrorMessage(error, t("status.apiError")),
      );
    } finally {
      setSelectedOverviewMediaFileDeleteLoading(false);
    }
  }, [
    client,
    recordCriticalCatalogMutation,
    refreshTitlePanelDetail,
    selectedOverviewMediaFileDeletePreview,
    selectedOverviewMediaFileDeleteTypedConfirmation,
    selectedOverviewMediaFileToDelete,
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
  const deleteSelectedOverviewMediaFileConfirmDisabled =
    selectedOverviewMediaFileDeletePreviewLoading ||
    !!selectedOverviewMediaFileDeletePreviewError ||
    !selectedOverviewMediaFileDeletePreview ||
    (selectedOverviewMediaFileDeletePreview.requiresTypedConfirmation &&
      selectedOverviewMediaFileDeleteTypedConfirmation.trim() !== "DELETE");

  const refreshLibraries = React.useCallback(async (): Promise<
    LibraryRecord[] | null
  > => {
    if (!isMediaView) {
      setLibraries([]);
      return [];
    }
    const permission =
      contentSettingsSection === "library" && !canManageConfig
        ? "manageLibrary"
        : "view";
    setLibrariesLoading(true);
    try {
      const { data, error } = await client
        .query(
          librariesQuery,
          { facet: activeFacet, permission },
          { requestPolicy: "network-only" },
        )
        .toPromise();
      if (error) throw error;
      const nextLibraries = (data?.libraries ?? []) as LibraryRecord[];
      setLibraries(nextLibraries);
      setSelectedLibraryIds((current) => {
        const normalized = normalizeLibraryFilterSelection(current, nextLibraries);
        return sameStringArray(current, normalized) ? current : normalized;
      });
      return nextLibraries;
    } catch (error) {
      setGlobalStatus(
        error instanceof Error ? error.message : t("status.failedToLoad"),
      );
      return null;
    } finally {
      setLibrariesLoading(false);
    }
  }, [
    activeFacet,
    canManageConfig,
    client,
    contentSettingsSection,
    isMediaView,
    setGlobalStatus,
    t,
  ]);

  const refreshRootValidationLibraries = React.useCallback(async (): Promise<
    LibraryRecord[] | null
  > => {
    if (!isMediaView) {
      setRootValidationLibraries([]);
      return [];
    }
    const permission =
      contentSettingsSection === "library" && !canManageConfig
        ? "manageLibrary"
        : "view";
    setRootValidationLibrariesLoading(true);
    try {
      const { data, error } = await client
        .query(
          librariesQuery,
          { facet: null, permission },
          { requestPolicy: "network-only" },
        )
        .toPromise();
      if (error) throw error;
      const nextLibraries = (data?.libraries ?? []) as LibraryRecord[];
      setRootValidationLibraries(nextLibraries);
      return nextLibraries;
    } catch (error) {
      setGlobalStatus(
        error instanceof Error ? error.message : t("status.failedToLoad"),
      );
      return null;
    } finally {
      setRootValidationLibrariesLoading(false);
    }
  }, [
    canManageConfig,
    client,
    contentSettingsSection,
    isMediaView,
    setGlobalStatus,
    t,
  ]);

  const loadLibrarySettings = React.useCallback(
    async (libraryId: string): Promise<LibrarySettingsRecord | null> => {
      const { data, error } = await client
        .query<{
          librarySettings: LibrarySettingsRecord;
        }>(
          librarySettingsQuery,
          { libraryId },
          { requestPolicy: "network-only" },
        )
        .toPromise();
      if (error) {
        throw error;
      }
      return data?.librarySettings ?? null;
    },
    [client],
  );

  const loadFacetDownloadClientRouting = React.useCallback(
    async (
      scopeId: LibraryRecord["facet"],
    ): Promise<DownloadClientRoutingEntry[]> => {
      const { data, error } = await client
        .query<{
          downloadClientRouting: DownloadClientRoutingEntry[];
        }>(
          downloadClientRoutingQuery,
          { scopeId },
          { requestPolicy: "network-only" },
        )
        .toPromise();
      if (error) {
        throw error;
      }
      return data?.downloadClientRouting ?? [];
    },
    [client],
  );

  React.useEffect(() => {
    const canManageDownloadClientRouting =
      canManageSystemSettings || canManageCatalogSettings;

    if (
      !isMediaView ||
      contentSettingsSection !== "library" ||
      !canManageDownloadClientRouting
    ) {
      setLibraryDownloadClients([]);
      setLibraryDownloadClientsLoading(false);
      return;
    }

    let cancelled = false;
    setLibraryDownloadClientsLoading(true);
    void client
      .query<{ downloadClientConfigs: DownloadClientRecord[] }>(
        libraryDownloadClientsQuery,
        {},
        { requestPolicy: "network-only" },
      )
      .toPromise()
      .then(({ data, error }) => {
        if (cancelled) {
          return;
        }
        if (error) {
          throw error;
        }
        setLibraryDownloadClients(data?.downloadClientConfigs ?? []);
      })
      .catch((error) => {
        if (cancelled) {
          return;
        }
        setGlobalStatus(
          error instanceof Error ? error.message : t("status.failedToLoad"),
        );
      })
      .finally(() => {
        if (!cancelled) {
          setLibraryDownloadClientsLoading(false);
        }
      });

    return () => {
      cancelled = true;
    };
  }, [
    canManageCatalogSettings,
    canManageSystemSettings,
    client,
    contentSettingsSection,
    isMediaView,
    setGlobalStatus,
    t,
  ]);

  const createLibrary = React.useCallback(
    async (input: {
      name: string;
      roots: RootFolderOption[];
      settings?: LibrarySettingsDraft;
    }) => {
      setLibrarySettingsSaving(true);
      try {
        const { data, error } = await client
          .mutation<{ createLibrary: LibraryRecord }>(createLibraryMutation, {
            input: {
              facet: activeFacet,
              name: input.name,
              roots: libraryRootsInput(input.roots),
              settings: librarySettingsInput(input.settings),
            },
          })
          .toPromise();
        if (error) throw error;
        const library = data?.createLibrary ?? null;
        await refreshLibraries();
        await refreshRootValidationLibraries();
        if (library) {
          setSelectedLibraryIds([library.id]);
          setGlobalStatus(t("settings.libraryCreated"));
          toast.success(t("settings.libraryCreated"));
        }
        return library;
      } catch (error) {
        setGlobalStatus(
          error instanceof Error
            ? error.message
            : t("settings.librarySaveFailed"),
        );
        return null;
      } finally {
        setLibrarySettingsSaving(false);
      }
    },
    [
      activeFacet,
      client,
      refreshLibraries,
      refreshRootValidationLibraries,
      setGlobalStatus,
      t,
    ],
  );

  const updateLibrary = React.useCallback(
    async (
      libraryId: string,
      input: {
        name: string;
        roots: RootFolderOption[];
        settings?: LibrarySettingsDraft;
      },
    ) => {
      setLibrarySettingsSaving(true);
      try {
        const { data, error } = await client
          .mutation<{ updateLibrary: LibraryRecord }>(updateLibraryMutation, {
            input: {
              libraryId,
              name: input.name,
              roots: libraryRootsInput(input.roots),
              settings: librarySettingsInput(input.settings),
            },
          })
          .toPromise();
        if (error) throw error;
        const library = data?.updateLibrary ?? null;
        await refreshLibraries();
        await refreshRootValidationLibraries();
        if (library) {
          setGlobalStatus(t("settings.librarySaved"));
          toast.success(t("settings.librarySaved"));
        }
        return library;
      } catch (error) {
        setGlobalStatus(
          error instanceof Error
            ? error.message
            : t("settings.librarySaveFailed"),
        );
        return null;
      } finally {
        setLibrarySettingsSaving(false);
      }
    },
    [
      client,
      refreshLibraries,
      refreshRootValidationLibraries,
      setGlobalStatus,
      t,
    ],
  );

  const deleteLibrary = React.useCallback(
    async (libraryId: string) => {
      setLibrarySettingsSaving(true);
      try {
        const { data, error } = await client
          .mutation<{ deleteLibrary: { id: string; deleted: boolean } }>(
            deleteLibraryMutation,
            {
              id: libraryId,
            },
          )
          .toPromise();
        if (error) throw error;
        if (!data?.deleteLibrary?.deleted) {
          throw new Error(t("settings.libraryDeleteFailed"));
        }
        setSelectedLibraryIds((current) =>
          current.filter(
            (selectedLibraryId) => selectedLibraryId !== libraryId,
          ),
        );
        await refreshLibraries();
        await refreshRootValidationLibraries();
        setGlobalStatus(t("settings.libraryDeleted"));
        return true;
      } catch (error) {
        setGlobalStatus(
          error instanceof Error
            ? error.message
            : t("settings.libraryDeleteFailed"),
        );
        return false;
      } finally {
        setLibrarySettingsSaving(false);
      }
    },
    [
      client,
      refreshLibraries,
      refreshRootValidationLibraries,
      setGlobalStatus,
      t,
    ],
  );

  const handleLibraryScan = React.useCallback(
    async (libraryId?: string) => {
      const targetLibraryId =
        libraryId ?? singleSelectedLibraryId(selectedLibraryIds);
      if (!targetLibraryId) {
        setLibraryScanNotice("Choose a library to scan.");
        return;
      }
      if (activeLibraryScanSession) {
        setLibraryScanNotice(
          t("settings.libraryScanAlreadyRunning", {
            facet: activeFacetLabel,
          }),
        );
        return;
      }

      setLibraryScanNotice(null);
      setLibraryScanLoading(true);
      setLibraryScanSummary(null);
      setStartedLibraryScanSessionId(null);
      try {
        const result = await client
          .mutation(scanLibraryMutation, {
            input: { libraryId: targetLibraryId },
          })
          .toPromise();
        if (result.error) throw result.error;
        const sessionId = result.data?.scanLibrary?.sessionId ?? null;
        setStartedLibraryScanSessionId(sessionId);
        void refreshLibraryScanSessions().catch((error) => {
          console.error(
            "[library-scan] failed to refresh active scan sessions:",
            error,
          );
        });
      } catch (error) {
        console.error("[library-scan] mutation failed:", error);
        const message =
          error instanceof Error ? error.message : String(error ?? "");
        if (/library scan already running/i.test(message)) {
          setLibraryScanNotice(
            t("settings.libraryScanAlreadyRunning", {
              facet: activeFacetLabel,
            }),
          );
          return;
        }
        if (
          error != null &&
          typeof error === "object" &&
          "networkError" in error &&
          (error as { networkError?: unknown }).networkError != null
        ) {
          toast.error(
            error instanceof Error
              ? error.message
              : t("settings.libraryScanFailed"),
          );
          setGlobalStatus(
            error instanceof Error
              ? error.message
              : t("settings.libraryScanFailed"),
          );
          return;
        }
        setGlobalStatus(
          error instanceof Error
            ? error.message
            : t("settings.libraryScanFailed"),
        );
      } finally {
        setLibraryScanLoading(false);
      }
    },
    [
      activeFacetLabel,
      activeLibraryScanSession,
      client,
      selectedLibraryIds,
      refreshLibraryScanSessions,
      setLibraryScanLoading,
      setLibraryScanNotice,
      setLibraryScanSummary,
      setStartedLibraryScanSessionId,
      setGlobalStatus,
      t,
    ],
  );

  React.useEffect(() => {
    if (!titleStatus) {
      setTitleStatus(t("title.noManaged"));
    }
  }, [t, titleStatus, setTitleStatus]);

  React.useEffect(() => {
    if (shouldLoadCatalogTitles) {
      return;
    }
    void refreshLibraries();
  }, [refreshLibraries, shouldLoadCatalogTitles]);

  React.useEffect(() => {
    if (
      shouldLoadCatalogTitles ||
      catalogBootstrapState.facet !== activeFacet ||
      !catalogBootstrapState.loading
    ) {
      return;
    }

    setCatalogBootstrapState((current) =>
      current.facet === activeFacet && current.loading
        ? { ...current, loading: false }
        : current,
    );
  }, [
    activeFacet,
    catalogBootstrapState.facet,
    catalogBootstrapState.loading,
    shouldLoadCatalogTitles,
  ]);

  React.useEffect(() => {
    if (contentSettingsSection !== "library" || !isMediaView) {
      setRootValidationLibraries([]);
      setRootValidationLibrariesLoading(false);
      return;
    }
    void refreshRootValidationLibraries();
  }, [contentSettingsSection, isMediaView, refreshRootValidationLibraries]);

  // Load media settings once per view/scope change (subscription handles live updates).
  // Deferred pattern: StrictMode unmount/remount cancels the stale call.
  React.useEffect(() => {
    if (!shouldLoadMediaSettings) return;
    let cancelled = false;
    const timer = setTimeout(() => {
      if (!cancelled) void refreshMediaSettings();
    }, 0);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [shouldLoadMediaSettings, refreshMediaSettings]);

  React.useEffect(() => {
    if (!isMediaView) {
      return;
    }

    const isGeneralSettingsSection =
      contentSettingsSection === "library" ||
      contentSettingsSection === "general";
    const isRoutingSection = contentSettingsSection === "routing";

    if (shouldLoadCatalogTitles) {
      if (!catalogInitialLoadComplete) {
        if (catalogBootstrapInFlight) {
          return;
        }

        // Keep bootstrap completion stable across rerenders while loading.
        // Effect-local cleanup would cancel the bootstrap as soon as the
        // loading state rerendered, leaving the catalog permanently blank.
        const requestSeq = ++catalogBootstrapRequestSeqRef.current;
        skipNextCatalogOverviewReloadRef.current = false;
        setCatalogBootstrapState({
          facet: activeFacet,
          loading: true,
          initialLoadComplete: false,
        });

        const finalizeBootstrap = () => {
          if (catalogBootstrapRequestSeqRef.current !== requestSeq) {
            return;
          }

          skipNextCatalogOverviewReloadRef.current = true;
          setCatalogBootstrapState({
            facet: activeFacet,
            loading: false,
            initialLoadComplete: true,
          });
        };

        const librariesPromise = refreshLibraries();
        void (async () => {
          await reloadTitles(debouncedTitleFilter, []);
          if (catalogBootstrapRequestSeqRef.current !== requestSeq) {
            return;
          }

          const nextLibraries = await librariesPromise;
          if (catalogBootstrapRequestSeqRef.current !== requestSeq) {
            return;
          }

          const normalizedSelectedLibraryIds = nextLibraries
            ? normalizeLibraryFilterSelection(selectedLibraryIds, nextLibraries)
            : [];
          const librarySelectionChanged = !sameStringArray(
            selectedLibraryIds,
            normalizedSelectedLibraryIds,
          );
          setSelectedLibraryIds((current) =>
            librarySelectionChanged
              ? normalizedSelectedLibraryIds
              : current,
          );

          if (librarySelectionChanged) {
            await reloadTitles(debouncedTitleFilter, normalizedSelectedLibraryIds);
            if (catalogBootstrapRequestSeqRef.current !== requestSeq) {
              return;
            }
          }
          finalizeBootstrap();
        })();
        setRoutingInitLoading(false);
        return;
      }

      if (skipNextCatalogOverviewReloadRef.current) {
        skipNextCatalogOverviewReloadRef.current = false;
      } else {
        void reloadTitles(debouncedTitleFilter);
      }
      setRoutingInitLoading(false);
      return;
    }
    if (isRoutingSection) {
      let cancelled = false;
      setRoutingInitLoading(true);
      void client
        .query(routingPageInitQuery, { scopeId: activeQualityScopeId })
        .toPromise()
        .then(({ data, error }) => {
          if (cancelled) {
            return;
          }
          if (error) {
            throw error;
          }
          hydrateDownloadClientRouting(
            data?.downloadClientConfigs || [],
            data.downloadClientRouting || [],
          );
          hydrateIndexerRouting(
            data?.indexers || [],
            data.indexerRouting || [],
          );
        })
        .catch((error) => {
          if (cancelled) {
            return;
          }
          setGlobalStatus(
            error instanceof Error ? error.message : t("status.failedToLoad"),
          );
        })
        .finally(() => {
          if (!cancelled) {
            setRoutingInitLoading(false);
          }
        });

      return () => {
        cancelled = true;
      };
    }
    setRoutingInitLoading(false);
    if (isGeneralSettingsSection && canManageConfig) {
      void refreshRuleSets();
    }
  }, [
    activeFacet,
    activeQualityScopeId,
    catalogBootstrapInFlight,
    catalogBootstrapLoading,
    catalogInitialLoadComplete,
    canManageConfig,
    client,
    contentSettingsSection,
    refreshLibraries,
    hydrateDownloadClientRouting,
    hydrateIndexerRouting,
    isMediaView,
    refreshRuleSets,
    debouncedTitleFilter,
    reloadTitles,
    selectedLibraryIds,
    setGlobalStatus,
    shouldLoadCatalogTitles,
    t,
    view,
  ]);

  const addDiscoveryFacet = addDiscoveryDialogTarget?.facet ?? activeFacet;
  const addDiscoveryResult =
    addDiscoveryDialogTarget?.result ?? EMPTY_SEARCH_RESULT;
  const requestDiscoveryFacet =
    requestDiscoveryDialogTarget?.facet ?? activeFacet;
  const requestDiscoveryResult =
    requestDiscoveryDialogTarget?.result ?? EMPTY_SEARCH_RESULT;
  const handleAddDiscoveryDialogOpenChange = (open: boolean) => {
    if (!open) {
      setAddDiscoveryDialogTarget(null);
    }
  };
  const handleRequestDiscoveryDialogOpenChange = (open: boolean) => {
    if (!open) {
      setRequestDiscoveryDialogTarget(null);
    }
  };

  return (
    <>
      <MediaContentView
        state={{
          view,
          contentSettingsSection,
          canManageConfig,
          canManageSystemSettings,
          canManageCatalogSettings,
          canManageLibrarySettings,
          contentSettingsLabel,
          moviesPath,
          setMoviesPath,
          seriesPath,
          setSeriesPath,
          saveSetting,
          localPathStyle,
          mediaSettingsLoading,
          librarySettingsSaving,
          qualityProfiles: qualityProfiles,
          qualityProfileEntries,
          qualityProfileParseError,
          globalQualityProfileId,
          globalScoringPersona,
          categoryQualityProfileOverrides,
          categoryRequiredAudioLanguages,
          saveCategoryRequiredAudioLanguages,
          categoryPersonaSelections,
          activeQualityScopeId,
          categoryFolderTemplates,
          setCategoryFolderTemplates,
          categoryRenameTemplates,
          setCategoryRenameTemplates,
          categoryRenameEnabled,
          setCategoryRenameEnabled,
          categoryRenameCollisionPolicies,
          setCategoryRenameCollisionPolicies,
          categoryRenameMissingMetadataPolicies,
          setCategoryRenameMissingMetadataPolicies,
          categoryFillerPolicies,
          setCategoryFillerPolicies,
          categoryRecapPolicies,
          setCategoryRecapPolicies,
          categoryMonitorSpecials,
          setCategoryMonitorSpecials,
          categoryInterSeasonMovies,
          setCategoryInterSeasonMovies,
          categoryMonitorFillerMovies,
          setCategoryMonitorFillerMovies,
          nfoWriteOnImport,
          setNfoWriteOnImport,
          plexmatchWriteOnImport,
          setPlexmatchWriteOnImport,
          importMode,
          setImportMode,
          setPermissionsLinux,
          setSetPermissionsLinux,
          fileChmod,
          setFileChmod,
          folderChmod,
          setFolderChmod,
          chownGroup,
          setChownGroup,
          qualityProfileInheritValue: QUALITY_PROFILE_INHERIT_VALUE,
          toProfileOptions,
          handleFacetPersonaSave: saveCategoryScoringPersonaOverride,
          saveCategoryQualityProfileOverride,
          updateCategoryMediaProfileSettings,
          mediaSettingsSaving,
          titleNameForQueue,
          setTitleNameForQueue,
          queueFacet,
          setQueueFacet,
          addTvdbCandidateToCatalog: addTvdbToCatalog,
          monitoredForQueue,
          setMonitoredForQueue,
          seasonFoldersForQueue,
          setSeasonFoldersForQueue,
          minAvailabilityForQueue,
          setMinAvailabilityForQueue,
          tvdbCandidates,
          onAddSubmit,
          titleFilter,
          setTitleFilter,
          refreshTitles,
          titleLoading,
          catalogTotalTitleCount: catalogPaginationState.totalCount,
          catalogHasMoreTitles: catalogPaginationState.hasMore,
          catalogLoadingMoreTitles: catalogPaginationState.loadingMore,
          loadMoreCatalogTitles,
          titleCatalogSortKey: titleCatalogSort.key,
          titleCatalogSortDirection: titleCatalogSort.direction,
          updateTitleCatalogSort,
          catalogBootstrapLoading,
          catalogInitialLoadComplete,
          monitoredTitles: visibleTitles,
          titleContextTitles: titleContextSourceTitles,
          catalogDiscoveryGroups: activeCatalogDiscoveryGroups,
          canManageTitle,
          canRequestMedia,
          onCatalogDiscoveryAction: handleCatalogDiscoveryAction,
          titleQuickFilters,
          titleQuickFilterCounts,
          toggleTitleQuickMonitoringFilter,
          toggleTitleQuickStatusFilter,
          clearTitleQuickFilters,
          queueExisting,
          toggleTitleMonitored,
          runInteractiveSearchForTitle,
          queueExistingFromRelease,
          queueAdditionalFromRelease,
          isTogglingTitleMonitoredById: titleMonitoringLoadingById,
          downloadClients,
          activeScopeRouting,
          activeScopeRoutingOrder,
          downloadClientRoutingLoading:
            downloadClientRoutingLoading || routingInitLoading,
          downloadClientRoutingSaving,
          updateDownloadClientRoutingForScope,
          moveDownloadClientInScope,
          indexers,
          activeScopeIndexerRouting,
          activeScopeIndexerRoutingOrder,
          indexerRoutingLoading: indexerRoutingLoading || routingInitLoading,
          indexerRoutingSaving,
          setIndexerEnabledForScope,
          updateIndexerRoutingForScope,
          moveIndexerInScope,
          ruleSets,
          rulesLoading,
          rulesSaving,
          onToggleRuleFacet,
          libraryScanLoading: libraryScanInProgress,
          libraryScanDisabled:
            libraryScanInProgress || selectedLibraryIds.length !== 1,
          libraryScanNotice,
          libraryScanSummary,
          libraries,
          librariesLoading,
          libraryDownloadClients,
          libraryDownloadClientsLoading,
          rootValidationLibraries,
          rootValidationLibrariesLoading,
          catalogHasValidRoot,
          invalidRootPathsByLibraryId,
          selectedLibraryIds,
          allLibrariesValue: ALL_LIBRARIES_VALUE,
          setSelectedLibraryIds,
          loadLibrarySettings,
          loadFacetDownloadClientRouting,
          createLibrary,
          updateLibrary,
          deleteLibrary,
          onOpenOverview,
          onCloseOverview,
          selectedOverviewTitleId,
          selectedOverviewDetailLoading,
          routeOverviewPending,
          routeOverviewEpisodeId,
          selectedOverviewBlocklistEntries,
          selectedOverviewExternalSubtitles,
          refreshSelectedOverviewExternalSubtitles,
          deleteSelectedOverviewMediaFile:
            requestDeleteSelectedOverviewMediaFile,
          makeSelectedOverviewMovieFilePrimary,
          selectedOverviewPrimaryMovieFileUpdatingId,
          previewTitleRename,
          applyTitleRename,
          setSelectedOverviewTitleId: selectOverviewTitle,
          clearSelectedOverviewTitle,
          scanLibrary: handleLibraryScan,
          deleteCatalogTitle: requestDeleteTitle,
          isDeletingCatalogTitleById: deleteTitleLoadingById,
          isMobile,
          viewMode: effectiveViewMode,
          setViewMode,
          selectedTitleIds,
          toggleTitleSelection,
          toggleAllVisibleTitles,
          clearSelectedTitles,
          bulkActionBusy,
          bulkMonitorTitles,
          openBulkTitleEdit,
          openBulkTitleDelete,
        }}
      />
      {canManageTitle ? (
        <AddToCatalogDialog
          open={addDiscoveryDialogTarget !== null}
          onOpenChange={handleAddDiscoveryDialogOpenChange}
          result={addDiscoveryResult}
          facet={addDiscoveryFacet}
          catalogQualityProfileOptions={catalogQualityProfileOptions}
          catalogConfigLoading={catalogConfigLoading}
          defaultQualityProfileId={resolveDefaultQualityProfileIdForFacet(
            addDiscoveryFacet,
          )}
          manageableLibraries={librariesByFacet[addDiscoveryFacet] ?? []}
          rootFolderOptions={rootFoldersByFacet[addDiscoveryFacet] ?? []}
          onAdd={async (result, facet, options) => {
            const titleId = await addMetadataSearchResultToCatalog(
              result,
              facet,
              options,
            );
            if (titleId) {
              await Promise.all([
                refreshTitles(),
                refreshCatalogDiscovery(),
              ]);
            }
            return titleId;
          }}
        />
      ) : null}
      {!canManageTitle && canRequestMedia ? (
        <RequestMediaDialog
          open={requestDiscoveryDialogTarget !== null}
          onOpenChange={handleRequestDiscoveryDialogOpenChange}
          result={requestDiscoveryResult}
          facet={requestDiscoveryFacet}
          requestableLibraries={
            requestableLibrariesByFacet[requestDiscoveryFacet] ?? []
          }
          qualityProfileOptions={catalogQualityProfileOptions}
          onRequest={async (result, facet, options) => {
            const accepted = await requestMetadataSearchResult(
              result,
              facet,
              options,
            );
            if (accepted) {
              await Promise.all([
                refreshTitles(),
                refreshCatalogDiscovery(),
              ]);
            }
            return accepted;
          }}
        />
      ) : null}
      <BulkTitleEditDialog
        open={bulkEditDialogOpen}
        onOpenChange={setBulkEditDialogOpen}
        view={view}
        selectedTitles={selectedTitles}
        qualityProfiles={qualityProfiles}
        rootFolders={bulkRootFolders}
        busy={bulkActionBusy}
        onSubmit={applyBulkTitleOptions}
      />
      <ConfirmDialog
        open={bulkDeleteDialogOpen}
        title={t("title.bulkDeleteTitle")}
        description={t("title.bulkDeleteDescription", {
          count: selectedTitles.length,
        })}
        confirmLabel={t("label.delete")}
        cancelLabel={t("label.cancel")}
        isBusy={bulkActionBusy}
        confirmDisabled={bulkDeleteConfirmDisabled}
        onConfirm={confirmBulkDeleteTitles}
        onCancel={closeBulkDeleteDialog}
      >
        <div className="space-y-3">
          <label className="flex items-center gap-2">
            <Checkbox
              checked={bulkDeleteFilesOnDisk}
              onCheckedChange={(checked) =>
                setBulkDeleteFilesOnDisk(checked === true)
              }
              disabled={bulkActionBusy}
            />
            <span className="text-xs text-card-foreground">
              {t("title.deleteFilesOnDisk")}
            </span>
          </label>
          {bulkDeleteFilesOnDisk ? (
            <DeletePreviewSummary
              preview={bulkDeletePreview}
              loading={bulkDeletePreviewLoading}
              error={bulkDeletePreviewError}
              typedConfirmation={bulkDeleteTypedConfirmation}
              onTypedConfirmationChange={setBulkDeleteTypedConfirmation}
            />
          ) : null}
        </div>
      </ConfirmDialog>
      <ConfirmDialog
        open={titleToDelete !== null}
        title={t("label.delete")}
        description={
          titleToDelete
            ? t("status.deleteCatalogConfirm", { name: titleToDelete.name })
            : t("label.delete")
        }
        confirmLabel={t("label.delete")}
        cancelLabel={t("label.cancel")}
        isBusy={
          titleToDelete !== null
            ? !!deleteTitleLoadingById[titleToDelete.id]
            : false
        }
        confirmDisabled={deleteTitleConfirmDisabled}
        onConfirm={confirmDeleteTitle}
        onCancel={closeDeleteTitleDialog}
      >
        <div className="space-y-3">
          <label className="flex items-center gap-2">
            <Checkbox
              checked={deleteFilesOnDisk}
              onCheckedChange={(checked) =>
                setDeleteFilesOnDisk(checked === true)
              }
              disabled={
                titleToDelete !== null
                  ? !!deleteTitleLoadingById[titleToDelete.id]
                  : false
              }
            />
            <span className="text-xs text-card-foreground">
              {t("title.deleteFilesOnDisk")}
            </span>
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
        open={selectedOverviewMediaFileToDelete !== null}
        title={t("mediaFile.delete")}
        description={
          selectedOverviewMediaFileToDelete?.file.filePath ??
          t("mediaFile.delete")
        }
        confirmLabel={t("label.delete")}
        cancelLabel={t("label.cancel")}
        isBusy={selectedOverviewMediaFileDeleteLoading}
        confirmDisabled={deleteSelectedOverviewMediaFileConfirmDisabled}
        onConfirm={confirmDeleteSelectedOverviewMediaFile}
        onCancel={closeSelectedOverviewMediaFileDeleteDialog}
      >
        <DeletePreviewSummary
          preview={selectedOverviewMediaFileDeletePreview}
          loading={selectedOverviewMediaFileDeletePreviewLoading}
          error={selectedOverviewMediaFileDeletePreviewError}
          typedConfirmation={selectedOverviewMediaFileDeleteTypedConfirmation}
          onTypedConfirmationChange={
            setSelectedOverviewMediaFileDeleteTypedConfirmation
          }
        />
      </ConfirmDialog>
      {replaceConflictDialog}
    </>
  );
});
