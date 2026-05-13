import * as React from "react";
import { MediaContentView } from "@/components/views/media-content-view";
import {
  addTitleMutation,
  buildDeleteTitleBatchMutation,
  buildSetTitleMonitoredBatchMutation,
  buildUpdateTitleBatchMutation,
  createLibraryMutation,
  deleteLibraryMutation,
  queueBestReleaseMutation,
  queueExistingMutation,
  scanLibraryMutation,
  deleteTitleMutation,
  setTitleMonitoredMutation,
  updateLibraryMutation,
  updateRuleSetMutation,
} from "@/lib/graphql/mutations";
import {
  buildDeleteTitlePreviewBatchQuery,
  deleteTitlePreviewQuery,
  librariesQuery,
  librarySettingsQuery,
  ruleSetsQuery,
  routingPageInitQuery,
  searchForTitleQuery,
  titlesQuery,
} from "@/lib/graphql/queries";
import {
  CATEGORY_SCOPE_MAP,
  QUALITY_PROFILE_INHERIT_VALUE,
  viewToFacet,
} from "@/lib/constants/settings";
import { useClient } from "urql";
import type { ContentSettingsSection, OverviewTitleTarget, ViewId } from "@/components/root/types";
import {
  toProfileOptions,
} from "@/lib/utils/quality-profiles";
import {
  normalizeLibraryFilterSelection,
  selectedLibraryIdsToQueryValue,
  singleSelectedLibraryId,
} from "@/lib/utils/library-filter";
import { releaseQueueScopeInput } from "@/lib/utils/release-queue-scope";
import { useDownloadClientRouting } from "@/lib/hooks/use-download-client-routing";
import { useIndexerRouting } from "@/lib/hooks/use-indexer-routing";
import { useMediaSettings } from "@/lib/hooks/use-media-settings";
import { useIsMobile } from "@/lib/hooks/use-mobile";
import { useQueueFormState } from "@/lib/hooks/use-queue-form-state";
import { useTitleManagementState } from "@/lib/hooks/use-title-management-state";
import type {
  LibraryRecord,
  LibrarySettingsDraft,
  LibrarySettingsRecord,
  Release,
  RootFolderOption,
  TitleRecord,
  RuleSetRecord,
} from "@/lib/types";
import type { DeletePreview } from "@/lib/types/delete-preview";
import { Checkbox } from "@/components/ui/checkbox";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { useDownloadConflictConfirmation } from "@/components/common/download-conflict-confirmation";
import { DeletePreviewSummary } from "@/components/common/delete-preview-summary";
import type { MetadataTvdbSearchItem } from "@/lib/graphql/smg-queries";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useLibraryScanProgress } from "@/lib/context/library-scan-progress-context";
import { useSearchContext } from "@/lib/context/search-context";
import { useReactiveRefresh } from "@/lib/context/reactive-refresh-context";
import { useDeletePreview } from "@/lib/hooks/use-delete-preview";
import { useOverviewWindowScrollRestoration } from "@/lib/hooks/use-overview-window-scroll-restoration";
import { useTitleListReactiveRefresh } from "@/lib/hooks/use-title-list-reactive-refresh";
import type { TitleOptionUpdates } from "@/lib/types/title-options";
import { toast } from "sonner";
import { BulkTitleEditDialog } from "@/components/views/media-content/bulk-title-edit-dialog";
import {
  readStoredContentViewMode,
  writeStoredContentViewMode,
  type ContentViewMode,
} from "@/components/views/media-content/content-view-mode";
import {
  filterTitlesByQuickFilters,
  type TitleQuickFilters,
} from "@/components/views/media-content/title-quick-filters";
import {
  assertNoReplaceConflict,
  retryWithReplaceOnConflict,
} from "@/lib/utils/download-conflicts";

const HYDRATION_POSTER_REFRESH_WINDOW_MS = 5 * 60 * 1000;
const HYDRATION_POSTER_REFRESH_INTERVAL_MS = 2_500;
const ALL_LIBRARIES_VALUE = "__all__";

type MediaContentContainerProps = {
  view: ViewId;
  contentSettingsSection: ContentSettingsSection;
  canManageConfig: boolean;
  onOpenOverview: (targetView: ViewId, overviewTarget: OverviewTitleTarget) => void;
};

function sortCatalogTitles(titles: TitleRecord[]): TitleRecord[] {
  return [...titles].sort((left, right) => {
    const nameCompare = left.name.toLocaleLowerCase().localeCompare(
      right.name.toLocaleLowerCase(),
    );
    if (nameCompare !== 0) {
      return nameCompare;
    }
    return left.id.localeCompare(right.id);
  });
}

function mergePreferLoadedImageFields(
  current: TitleRecord,
  incoming: TitleRecord,
): TitleRecord {
  const incomingHasPoster = Boolean(incoming.posterUrl || incoming.posterSourceUrl);
  const incomingHasBanner = Boolean(incoming.bannerUrl || incoming.bannerSourceUrl);
  const incomingHasBackground = Boolean(
    incoming.backgroundUrl || incoming.backgroundSourceUrl,
  );

  return {
    ...incoming,
    posterUrl: incomingHasPoster ? incoming.posterUrl : (current.posterUrl ?? null),
    posterSourceUrl: incomingHasPoster
      ? incoming.posterSourceUrl
      : (current.posterSourceUrl ?? null),
    bannerUrl: incomingHasBanner ? incoming.bannerUrl : (current.bannerUrl ?? null),
    bannerSourceUrl: incomingHasBanner
      ? incoming.bannerSourceUrl
      : (current.bannerSourceUrl ?? null),
    backgroundUrl: incomingHasBackground
      ? incoming.backgroundUrl
      : (current.backgroundUrl ?? null),
    backgroundSourceUrl: incomingHasBackground
      ? incoming.backgroundSourceUrl
      : (current.backgroundSourceUrl ?? null),
    metadataFetchedAt: incoming.metadataFetchedAt ?? current.metadataFetchedAt,
  };
}

function mergeCatalogTitlesPreservingImages(
  currentTitles: TitleRecord[],
  incomingTitles: TitleRecord[],
): TitleRecord[] {
  const currentById = new Map(currentTitles.map((title) => [title.id, title]));

  return sortCatalogTitles(
    incomingTitles.map((title) => {
      const current = currentById.get(title.id);
      return current ? mergePreferLoadedImageFields(current, title) : title;
    }),
  );
}

function upsertCatalogTitleRecord(
  titles: TitleRecord[],
  title: TitleRecord,
): TitleRecord[] {
  const next = [...titles];
  const existingIndex = next.findIndex((item) => item.id === title.id);
  if (existingIndex === -1) {
    next.push(title);
  } else {
    next[existingIndex] = mergePreferLoadedImageFields(next[existingIndex], title);
  }
  return sortCatalogTitles(next);
}

function isPendingHydrationPosterTitle(title: TitleRecord, nowMs: number): boolean {
  if (title.posterUrl || title.posterSourceUrl || title.metadataFetchedAt != null) {
    return false;
  }

  const createdAtMs = title.createdAt ? Date.parse(title.createdAt) : Number.NaN;
  if (!Number.isFinite(createdAtMs)) {
    return true;
  }

  return nowMs - createdAtMs <= HYDRATION_POSTER_REFRESH_WINDOW_MS;
}

function sameIdSet(left: ReadonlySet<string>, right: ReadonlySet<string>): boolean {
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

function librarySettingsInput(settings: LibrarySettingsDraft | undefined) {
  if (!settings) {
    return undefined;
  }
  return {
    requiredAudioLanguages: settings.requiredAudioLanguages,
    qualityProfileId: settings.qualityProfileId,
    scoringPersona: settings.scoringPersona,
    fillerPolicy: settings.fillerPolicy,
    recapPolicy: settings.recapPolicy,
    monitorSpecials: settings.monitorSpecials,
    interSeasonMovies: settings.interSeasonMovies,
    monitorFillerMovies: settings.monitorFillerMovies,
    nfoWriteOnImport: settings.nfoWriteOnImport,
    plexmatchWriteOnImport: settings.plexmatchWriteOnImport,
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
  const refreshedById = new Map(refreshedTitles.map((title) => [title.id, title]));
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
  const refreshedById = new Map(refreshedTitles.map((title) => [title.id, title]));
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
      changes.rootFolderPath !== undefined &&
      (refreshed.rootFolderPath ?? "") !== changes.rootFolderPath
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

function inferTitleDeleteBatchOutcome(
  targets: TitleRecord[],
  refreshedTitles: TitleRecord[],
): { succeededIds: string[]; failedIds: string[] } {
  const remainingIds = new Set(refreshedTitles.map((title) => title.id));
  return splitSucceededTitleIds(targets, (title) => !remainingIds.has(title.id));
}

function aggregateDeletePreviews(previews: DeletePreview[]): DeletePreview | null {
  if (previews.length === 0) {
    return null;
  }

  const samplePaths = Array.from(
    new Set(previews.flatMap((preview) => preview.samplePaths)),
  ).slice(0, 12);
  const typedPrompt =
    previews.find((preview) => preview.requiresTypedConfirmation)
      ?.typedConfirmationPrompt ?? null;

  return {
    fingerprint: "",
    totalFileCount: previews.reduce(
      (sum, preview) => sum + preview.totalFileCount,
      0,
    ),
    mediaCount: previews.reduce((sum, preview) => sum + preview.mediaCount, 0),
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
    requiresTypedConfirmation: previews.some(
      (preview) => preview.requiresTypedConfirmation,
    ),
    typedConfirmationPrompt: typedPrompt,
    targetLabel: "",
    samplePaths,
  };
}

export const MediaContentContainer = React.memo(function MediaContentContainer({
  view,
  contentSettingsSection,
  canManageConfig,
  onOpenOverview,
}: MediaContentContainerProps) {
  const searchState = useSearchContext();
  const {
    queueFacet,
    setQueueFacet,
    runTvdbSearch,
    tvdbCandidates,
  } = searchState;
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const client = useClient();
  const { confirmReplaceConflict, replaceConflictDialog } =
    useDownloadConflictConfirmation();
  const { queueCatalogTitleRefresh } = useReactiveRefresh();
  const [titleDeleteTypedConfirmation, setTitleDeleteTypedConfirmation] =
    React.useState("");
  const [startedLibraryScanSessionId, setStartedLibraryScanSessionId] =
    React.useState<string | null>(null);
  const activeFacet = viewToFacet[view as keyof typeof viewToFacet] ?? "movie";
  const { getActiveSession, getSessionById, refreshSessions: refreshLibraryScanSessions } =
    useLibraryScanProgress();
  const activeLibraryScanSession = getActiveSession(activeFacet);
  const startedLibraryScanSession = startedLibraryScanSessionId
    ? getSessionById(startedLibraryScanSessionId)
    : null;
  const isMobile = useIsMobile();
  const activeQualityScopeId =
    CATEGORY_SCOPE_MAP[view as keyof typeof CATEGORY_SCOPE_MAP] ?? "movie";
  const isMediaView =
    view === "movies" || view === "series" || view === "anime";
  const shouldLoadCatalogTitles =
    isMediaView && contentSettingsSection === "overview";
  const shouldLoadMediaSettings = isMediaView;
  const [desktopViewMode, setDesktopViewMode] = React.useState<ContentViewMode>(
    () => readStoredContentViewMode(),
  );
  const effectiveViewMode: ContentViewMode = isMobile
    ? "poster"
    : desktopViewMode;
  const [selectedTitleIds, setSelectedTitleIds] = React.useState<Set<string>>(
    () => new Set(),
  );
  const [titleQuickFilters, setTitleQuickFilters] =
    React.useState<TitleQuickFilters>({
      monitored: false,
      unmonitored: false,
      continuing: false,
      ended: false,
    });
  const [bulkActionBusy, setBulkActionBusy] = React.useState(false);
  const [bulkEditDialogOpen, setBulkEditDialogOpen] = React.useState(false);
  const [bulkDeleteDialogOpen, setBulkDeleteDialogOpen] = React.useState(false);
  const [bulkDeleteFilesOnDisk, setBulkDeleteFilesOnDisk] =
    React.useState(false);
  const [bulkDeleteTypedConfirmation, setBulkDeleteTypedConfirmation] =
    React.useState("");
  const [bulkDeletePreviewLoading, setBulkDeletePreviewLoading] =
    React.useState(false);
  const [bulkDeletePreviewError, setBulkDeletePreviewError] = React.useState<
    string | null
  >(null);
  const [bulkDeletePreviewsByTitleId, setBulkDeletePreviewsByTitleId] =
    React.useState<Record<string, DeletePreview>>({});
  const [debouncedTitleFilter, setDebouncedTitleFilter] = React.useState("");
  const [libraries, setLibraries] = React.useState<LibraryRecord[]>([]);
  const [librariesLoading, setLibrariesLoading] = React.useState(false);
  const [catalogBootstrapState, setCatalogBootstrapState] = React.useState({
    facet: activeFacet,
    loading: false,
    initialLoadComplete: false,
  });
  const [rootValidationLibraries, setRootValidationLibraries] = React.useState<LibraryRecord[]>([]);
  const [rootValidationLibrariesLoading, setRootValidationLibrariesLoading] = React.useState(false);
  const [librarySettingsSaving, setLibrarySettingsSaving] = React.useState(false);
  const [selectedLibraryIds, setSelectedLibraryIds] = React.useState<string[]>([]);
  const activeCatalogQueryRef = React.useRef("");
  const catalogTitleRequestSeqRef = React.useRef(0);
  const catalogBootstrapRequestSeqRef = React.useRef(0);
  const skipNextCatalogOverviewReloadRef = React.useRef(false);

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
    shouldLoadCatalogTitles &&
    !catalogInitialLoadComplete;
  const titleDeletePreviewVariables = React.useMemo(
    () =>
      titleToDelete && deleteFilesOnDisk
        ? { input: { titleId: titleToDelete.id } }
        : null,
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
  const effectiveTitleQuickFilters = React.useMemo<TitleQuickFilters>(
    () => ({
      ...titleQuickFilters,
      continuing: activeFacet === "movie" ? false : titleQuickFilters.continuing,
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
  const monitoredTitlesWithLibraries = React.useMemo(
    () =>
      monitoredTitles.map((title) => ({
        ...title,
        libraryName:
          title.libraryName ?? libraryNameById.get(title.libraryId) ?? title.libraryId,
        librarySlug:
          title.librarySlug ?? librarySlugById.get(title.libraryId) ?? null,
      })),
    [libraryNameById, librarySlugById, monitoredTitles],
  );
  const visibleTitles = React.useMemo(
    () => filterTitlesByQuickFilters(monitoredTitlesWithLibraries, effectiveTitleQuickFilters),
    [effectiveTitleQuickFilters, monitoredTitlesWithLibraries],
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
        ? libraries.find((library) => library.id === selectedTitleLibraryIds[0]) ?? null
        : null,
    [libraries, selectedTitleLibraryIds],
  );
  const bulkRootFolders = React.useMemo(
    () =>
      (selectedTitleLibrary?.roots ?? []).map((root) => ({
        path: root.path,
        isDefault: root.isDefault,
      })),
    [selectedTitleLibrary],
  );

  useOverviewWindowScrollRestoration({
    enabled: shouldLoadCatalogTitles,
    ready: !titleLoading && visibleTitles.length > 0,
    storageKeySuffix: "window",
  });

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
    setTitleQuickFilters({
      monitored: false,
      unmonitored: false,
      continuing: false,
      ended: false,
    });
    setSelectedTitleIds(new Set());
    setSelectedLibraryIds([]);
  }, [activeFacet]);

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
    activeCatalogQueryRef.current = debouncedTitleFilter;
  }, [debouncedTitleFilter]);

  React.useEffect(() => {
    if (isMobile) {
      return;
    }
    writeStoredContentViewMode(desktopViewMode);
  }, [desktopViewMode, isMobile]);

  React.useEffect(() => {
    if (
      effectiveViewMode === "compact" &&
      shouldLoadCatalogTitles &&
      contentSettingsSection === "overview"
    ) {
      return;
    }
    setSelectedTitleIds((current) => (current.size === 0 ? current : new Set()));
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
    categoryRenameTemplates,
    setCategoryRenameTemplates,
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
      const requestSeq = ++catalogTitleRequestSeqRef.current;

      try {
        const { data, error } = await client
          .query(
            titlesQuery,
            {
              facet: activeFacet,
              libraryIds: selectedLibraryIdsToQueryValue(libraryIds),
              query: query || null,
            },
            { requestPolicy: "network-only" },
          )
          .toPromise();
        if (error) {
          throw error;
        }
        if (requestSeq !== catalogTitleRequestSeqRef.current) {
          return null;
        }

        const nextTitles = (data?.titles ?? []) as TitleRecord[];
        setMonitoredTitles((current) =>
          mergeCatalogTitlesPreservingImages(current, nextTitles),
        );
        setTitleStatus(t("title.statusTemplate", { count: nextTitles.length }));
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
      selectedLibraryIds,
      setMonitoredTitles,
      setTitleLoading,
      setTitleStatus,
      t,
    ],
  );

  const refreshTitles = React.useCallback(async (query?: string) => {
    await reloadTitles(query ?? titleFilter);
  }, [reloadTitles, titleFilter]);

  const applyRefreshedTitleRecord = React.useCallback(
    (titleId: string, title: TitleRecord | null) => {
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

        if (existingIndex === -1) {
          next.push(title);
        } else {
          next[existingIndex] = mergePreferLoadedImageFields(
            next[existingIndex],
            title,
          );
        }
        const sorted = sortCatalogTitles(next);
        setTitleStatus(t("title.statusTemplate", { count: next.length }));
        return sorted;
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

  useTitleListReactiveRefresh({
    facet: activeFacet,
    pause: !shouldLoadCatalogTitles,
    onTitleRefreshed: applyRefreshedTitleRecord,
  });

  React.useEffect(() => {
    if (!shouldLoadCatalogTitles || pendingHydrationPosterTitleIds.length === 0) {
      return;
    }

    const refreshPendingHydrationPosters = () => {
      pendingHydrationPosterTitleIds.forEach((titleId) => {
        queueCatalogTitleRefresh({
          titleId,
          apply(title) {
            applyRefreshedTitleRecord(titleId, title);
          },
          onError(error) {
            console.error("[catalog-hydration-poster-refresh] refresh failed:", error);
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
          setMonitoredTitles((current) => {
            const next = upsertCatalogTitleRecord(
              current,
              data.addTitle.title as TitleRecord,
            );
            setTitleStatus(t("title.statusTemplate", { count: next.length }));
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
        assertNoReplaceConflict(payload, "A download is already in progress for this title.");
        const queuedMessage = t("status.queuedLatest", { name: title.name });
        setGlobalStatus(queuedMessage);
      } catch (error) {
        setGlobalStatus(
          error instanceof Error ? error.message : t("status.queueFailed"),
        );
      }
    },
    [client, confirmReplaceConflict, setGlobalStatus, t],
  );

  const runInteractiveSearchForTitle = React.useCallback(
    async (title: TitleRecord) => {
      try {
        const { data, error } = await client
          .query(searchForTitleQuery, { titleId: title.id })
          .toPromise();
        if (error) throw error;
        return (data?.searchIndexersForTitle ?? []) as Release[];
      } catch (error) {
        setGlobalStatus(
          error instanceof Error ? error.message : t("status.searchFailed"),
        );
        return [];
      }
    },
    [client, setGlobalStatus, t],
  );

  const queueExistingFromRelease = React.useCallback(
    async (title: TitleRecord, release: Release) => {
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
            const { data, error } = await client
              .mutation(queueExistingMutation, { input: nextInput })
              .toPromise();
            if (error) throw error;
            return data?.queueExistingTitleDownload;
          },
          "A download is already in progress for this title.",
          confirmReplaceConflict,
        );
        assertNoReplaceConflict(payload, "A download is already in progress for this title.");
        const queuedMessage = t("status.queuedLatest", { name: title.name });
        setGlobalStatus(queuedMessage);
      } catch (error) {
        setGlobalStatus(
          error instanceof Error ? error.message : t("status.queueFailed"),
        );
      }
    },
    [client, confirmReplaceConflict, setGlobalStatus, t],
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
          [nextFilter]: !current[nextFilter],
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
          [nextFilter]: !current[nextFilter],
        }));
      });
    },
    [],
  );

  const clearTitleQuickFilters = React.useCallback(() => {
    React.startTransition(() => {
      setTitleQuickFilters({
        monitored: false,
        unmonitored: false,
        continuing: false,
        ended: false,
      });
    });
  }, []);

  const toggleAllVisibleTitles = React.useCallback(
    (checked: boolean) => {
      setSelectedTitleIds(
        checked ? new Set(visibleTitles.map((title) => title.id)) : new Set(),
      );
    },
    [visibleTitles],
  );

  const clearSelectedTitles = React.useCallback(() => {
    setSelectedTitleIds((current) => (current.size === 0 ? current : new Set()));
  }, []);

  const setViewMode = React.useCallback((nextMode: ContentViewMode) => {
    setDesktopViewMode(nextMode);
  }, []);

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
          .mutation<Record<string, { id: string; monitored: boolean }>>(
            buildSetTitleMonitoredBatchMutation(targets.length),
            variables,
          )
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
          .mutation<Record<string, { id: string }>>(
            buildUpdateTitleBatchMutation(targets.length),
            variables,
          )
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

  const closeBulkDeleteDialog = React.useCallback(() => {
    setBulkDeleteDialogOpen(false);
    setBulkDeleteFilesOnDisk(false);
    setBulkDeleteTypedConfirmation("");
    setBulkDeletePreviewLoading(false);
    setBulkDeletePreviewError(null);
    setBulkDeletePreviewsByTitleId({});
  }, []);

  React.useEffect(() => {
    if (!bulkDeleteFilesOnDisk) {
      setBulkDeleteTypedConfirmation("");
      setBulkDeletePreviewLoading(false);
      setBulkDeletePreviewError(null);
      setBulkDeletePreviewsByTitleId({});
    }
  }, [bulkDeleteFilesOnDisk]);

  React.useEffect(() => {
    if (!bulkDeleteDialogOpen || !bulkDeleteFilesOnDisk) {
      return;
    }

    const targets = [...selectedTitles];
    if (targets.length === 0) {
      setBulkDeletePreviewLoading(false);
      setBulkDeletePreviewError(null);
      setBulkDeletePreviewsByTitleId({});
      return;
    }

    let cancelled = false;
    setBulkDeletePreviewLoading(true);
    setBulkDeletePreviewError(null);

    const loadPreviews = async () => {
      try {
        const variables = Object.fromEntries(
          targets.map((title, index) => [
            `input${index}`,
            { titleId: title.id },
          ]),
        );
        const result = await client
          .query<Record<string, DeletePreview>>(
            buildDeleteTitlePreviewBatchQuery(targets.length),
            variables,
            { requestPolicy: "network-only" },
          )
          .toPromise();
        if (cancelled) {
          return;
        }

        let payload = result.data ?? {};
        if (Object.keys(payload).length === 0 && result.error) {
          const settled = await Promise.allSettled(
            targets.map(async (title) => {
              const single = await client
                .query<{ deleteTitlePreview: DeletePreview }>(
                  deleteTitlePreviewQuery,
                  { input: { titleId: title.id } },
                  { requestPolicy: "network-only" },
                )
                .toPromise();
              if (single.error || !single.data?.deleteTitlePreview) {
                throw single.error ?? new Error("delete title preview failed");
              }
              return [title.id, single.data.deleteTitlePreview] as const;
            }),
          );

          payload = {};
          settled.forEach((outcome, index) => {
            if (outcome.status === "fulfilled") {
              payload[batchItemAlias(index)] = outcome.value[1];
            }
          });
        }
        const nextPreviewsByTitleId: Record<string, DeletePreview> = {};
        let failedCount = 0;

        targets.forEach((title, index) => {
          const preview = payload[batchItemAlias(index)] as DeletePreview | undefined;
          if (preview) {
            nextPreviewsByTitleId[title.id] = preview;
          } else {
            failedCount += 1;
          }
        });

        setBulkDeletePreviewsByTitleId(nextPreviewsByTitleId);
        if (failedCount > 0) {
          setBulkDeletePreviewError(
            withFailureDetail(
              t("status.bulkDeletePreviewFailed", { failed: failedCount }),
              batchFailureDetail(result.error),
            ),
          );
        } else {
          setBulkDeletePreviewError(null);
        }
      } catch (error) {
        if (cancelled) {
          return;
        }
        setBulkDeletePreviewsByTitleId({});
        setBulkDeletePreviewError(
          withFailureDetail(
            t("status.bulkDeletePreviewFailed", { failed: targets.length }),
            batchFailureDetail(error),
          ),
        );
      } finally {
        if (!cancelled) {
          setBulkDeletePreviewLoading(false);
        }
      }
    };

    void loadPreviews();
    return () => {
      cancelled = true;
    };
  }, [
    bulkDeleteDialogOpen,
    bulkDeleteFilesOnDisk,
    client,
    selectedTitles,
    t,
  ]);

  const bulkDeletePreview = React.useMemo(
    () =>
      aggregateDeletePreviews(
        Object.values(bulkDeletePreviewsByTitleId).filter(Boolean),
      ),
    [bulkDeletePreviewsByTitleId],
  );
  const bulkDeletePreviewMissing =
    bulkDeleteFilesOnDisk &&
    selectedTitles.some((title) => !bulkDeletePreviewsByTitleId[title.id]);
  const bulkDeleteConfirmDisabled =
    bulkActionBusy ||
    selectedTitles.length === 0 ||
    (bulkDeleteFilesOnDisk &&
      (bulkDeletePreviewLoading ||
        !!bulkDeletePreviewError ||
        bulkDeletePreviewMissing ||
        !bulkDeletePreview ||
        (bulkDeletePreview.requiresTypedConfirmation &&
          bulkDeleteTypedConfirmation.trim() !== "DELETE")));

  const confirmBulkDeleteTitles = React.useCallback(async () => {
    const targets = [...selectedTitles];
    if (targets.length === 0 || bulkActionBusy) {
      return;
    }

    setBulkActionBusy(true);
    try {
      const variables = Object.fromEntries(
        targets.map((title, index) => {
          const preview = bulkDeletePreviewsByTitleId[title.id];
          return [
            `input${index}`,
            {
              titleId: title.id,
              ...(bulkDeleteFilesOnDisk
                ? {
                    deleteFilesOnDisk: true,
                    previewFingerprint: preview?.fingerprint,
                    ...(bulkDeleteTypedConfirmation.trim()
                      ? {
                          typedConfirmation:
                            bulkDeleteTypedConfirmation.trim(),
                        }
                      : {}),
                  }
                : {}),
            },
          ];
        }),
      );
      const result = await client
        .mutation<Record<string, boolean>>(
          buildDeleteTitleBatchMutation(targets.length),
          variables,
        )
        .toPromise();
      const payload = result.data ?? {};
      const refreshedTitles = await reloadTitles();
      let { succeededIds, failedIds } = refreshedTitles
        ? inferTitleDeleteBatchOutcome(targets, refreshedTitles)
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
          withFailureDetail(t("status.bulkTitleDeleteFailed"), detail),
        );
        return;
      }

      closeBulkDeleteDialog();
      if (failedIds.length > 0) {
        setGlobalStatus(
          withFailureDetail(
            t("status.bulkTitleDeletePartial", {
              count: succeededIds.length,
              failed: failedIds.length,
            }),
            detail,
          ),
        );
        return;
      }

      setGlobalStatus(
        t("status.bulkTitleDeleteSuccess", { count: succeededIds.length }),
      );
    } catch (error) {
      setGlobalStatus(
        withFailureDetail(
          t("status.bulkTitleDeleteFailed"),
          batchFailureDetail(error),
        ),
      );
    } finally {
      setBulkActionBusy(false);
    }
  }, [
    bulkActionBusy,
    bulkDeleteFilesOnDisk,
    bulkDeletePreviewsByTitleId,
    bulkDeleteTypedConfirmation,
    client,
    closeBulkDeleteDialog,
    reloadTitles,
    selectedTitles,
    setGlobalStatus,
    t,
  ]);

  const openBulkTitleEdit = React.useCallback(() => {
    if (selectedTitles.length === 0 || bulkActionBusy) {
      return;
    }
    if (selectedTitleLibraryIds.length !== 1) {
      setGlobalStatus("Bulk actions require titles from one library.");
      return;
    }
    setBulkEditDialogOpen(true);
  }, [bulkActionBusy, selectedTitleLibraryIds.length, selectedTitles.length, setGlobalStatus]);

  const openBulkTitleDelete = React.useCallback(() => {
    if (selectedTitles.length === 0 || bulkActionBusy) {
      return;
    }
    if (selectedTitleLibraryIds.length !== 1) {
      setGlobalStatus("Bulk actions require titles from one library.");
      return;
    }
    setBulkDeleteFilesOnDisk(false);
    setBulkDeleteTypedConfirmation("");
    setBulkDeletePreviewLoading(false);
    setBulkDeletePreviewError(null);
    setBulkDeletePreviewsByTitleId({});
    setBulkDeleteDialogOpen(true);
  }, [bulkActionBusy, selectedTitleLibraryIds.length, selectedTitles.length, setGlobalStatus]);

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
      const payload: {
        titleId: string;
        deleteFilesOnDisk?: boolean;
        previewFingerprint?: string;
        typedConfirmation?: string;
      } = {
        titleId,
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

      const { error } = await client
        .mutation(deleteTitleMutation, {
          input: payload,
        })
        .toPromise();
      if (error) throw error;
      setGlobalStatus(t("status.titleDeleted", { name: titleToDelete.name }));
      await refreshTitles();
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
    refreshTitles,
    client,
    titleDeletePreview,
    titleDeleteTypedConfirmation,
    t,
    titleToDelete,
    setGlobalStatus,
    setDeleteTitleLoadingById,
  ]);

  const deleteTitleConfirmDisabled =
    deleteFilesOnDisk &&
    (titleDeletePreviewLoading ||
      !!titleDeletePreviewError ||
      !titleDeletePreview ||
      (titleDeletePreview.requiresTypedConfirmation &&
        titleDeleteTypedConfirmation.trim() !== "DELETE"));

  const refreshLibraries = React.useCallback(async (): Promise<LibraryRecord[] | null> => {
    if (!isMediaView) {
      setLibraries([]);
      return [];
    }
    setLibrariesLoading(true);
    try {
      const { data, error } = await client
        .query(
          librariesQuery,
          { facet: activeFacet, permission: "view" },
          { requestPolicy: "network-only" },
        )
        .toPromise();
      if (error) throw error;
      const nextLibraries = (data?.libraries ?? []) as LibraryRecord[];
      setLibraries(nextLibraries);
      setSelectedLibraryIds((current) =>
        normalizeLibraryFilterSelection(current, nextLibraries),
      );
      return nextLibraries;
    } catch (error) {
      setGlobalStatus(
        error instanceof Error ? error.message : t("status.failedToLoad"),
      );
      return null;
    } finally {
      setLibrariesLoading(false);
    }
  }, [activeFacet, client, isMediaView, setGlobalStatus, t]);

  const refreshRootValidationLibraries = React.useCallback(
    async (): Promise<LibraryRecord[] | null> => {
      if (!isMediaView) {
        setRootValidationLibraries([]);
        return [];
      }
      setRootValidationLibrariesLoading(true);
      try {
        const { data, error } = await client
          .query(
            librariesQuery,
            { facet: null, permission: "view" },
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
    },
    [client, isMediaView, setGlobalStatus, t],
  );

  const loadLibrarySettings = React.useCallback(
    async (libraryId: string): Promise<LibrarySettingsRecord | null> => {
      const { data, error } = await client
        .query<{ librarySettings: LibrarySettingsRecord }>(
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

  const createLibrary = React.useCallback(
    async (input: { name: string; roots: RootFolderOption[]; settings?: LibrarySettingsDraft }) => {
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
        }
        return library;
      } catch (error) {
        setGlobalStatus(
          error instanceof Error ? error.message : t("settings.librarySaveFailed"),
        );
        return null;
      } finally {
        setLibrarySettingsSaving(false);
      }
    },
    [activeFacet, client, refreshLibraries, refreshRootValidationLibraries, setGlobalStatus, t],
  );

  const updateLibrary = React.useCallback(
    async (
      libraryId: string,
      input: { name: string; roots: RootFolderOption[]; settings?: LibrarySettingsDraft },
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
        }
        return library;
      } catch (error) {
        setGlobalStatus(
          error instanceof Error ? error.message : t("settings.librarySaveFailed"),
        );
        return null;
      } finally {
        setLibrarySettingsSaving(false);
      }
    },
    [client, refreshLibraries, refreshRootValidationLibraries, setGlobalStatus, t],
  );

  const deleteLibrary = React.useCallback(
    async (libraryId: string) => {
      setLibrarySettingsSaving(true);
      try {
        const { data, error } = await client
          .mutation<{ deleteLibrary: boolean }>(deleteLibraryMutation, {
            input: { libraryId },
          })
          .toPromise();
        if (error) throw error;
        if (!data?.deleteLibrary) {
          throw new Error(t("settings.libraryDeleteFailed"));
        }
        setSelectedLibraryIds((current) =>
          current.filter((selectedLibraryId) => selectedLibraryId !== libraryId),
        );
        await refreshLibraries();
        await refreshRootValidationLibraries();
        setGlobalStatus(t("settings.libraryDeleted"));
        return true;
      } catch (error) {
        setGlobalStatus(
          error instanceof Error ? error.message : t("settings.libraryDeleteFailed"),
        );
        return false;
      } finally {
        setLibrarySettingsSaving(false);
      }
    },
    [client, refreshLibraries, refreshRootValidationLibraries, setGlobalStatus, t],
  );

  const handleLibraryScan = React.useCallback(async (libraryId?: string) => {
    const targetLibraryId = libraryId ?? singleSelectedLibraryId(selectedLibraryIds);
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
        .mutation(scanLibraryMutation, { libraryId: targetLibraryId })
        .toPromise();
      if (result.error) throw result.error;
      const sessionId = result.data?.scanLibrary?.sessionId ?? null;
      setLibraryScanNotice(t("settings.libraryScanRunning"));
      setStartedLibraryScanSessionId(sessionId);
      void refreshLibraryScanSessions().catch((error) => {
        console.error("[library-scan] failed to refresh active scan sessions:", error);
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
  }, [
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
  ]);

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

        void Promise.all([
          refreshLibraries(),
          reloadTitles(debouncedTitleFilter, []),
        ]).finally(() => {
          if (catalogBootstrapRequestSeqRef.current !== requestSeq) {
            return;
          }

          skipNextCatalogOverviewReloadRef.current = true;
          setCatalogBootstrapState({
            facet: activeFacet,
            loading: false,
            initialLoadComplete: true,
          });
        });
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
    if (isGeneralSettingsSection) {
      void refreshRuleSets();
    }
  }, [
    activeFacet,
    activeQualityScopeId,
    catalogBootstrapInFlight,
    catalogBootstrapLoading,
    catalogInitialLoadComplete,
    client,
    contentSettingsSection,
    refreshLibraries,
    hydrateDownloadClientRouting,
    hydrateIndexerRouting,
    isMediaView,
    refreshRuleSets,
    debouncedTitleFilter,
    reloadTitles,
    setGlobalStatus,
    shouldLoadCatalogTitles,
    t,
    view,
  ]);

  return (
    <>
      <MediaContentView
        state={{
          view,
          contentSettingsSection,
          canManageConfig,
          contentSettingsLabel,
          moviesPath,
          setMoviesPath,
          seriesPath,
          setSeriesPath,
          saveSetting,
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
          categoryRenameTemplates,
          setCategoryRenameTemplates,
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
          catalogBootstrapLoading,
          catalogInitialLoadComplete,
          monitoredTitles: visibleTitles,
          titleQuickFilters,
          toggleTitleQuickMonitoringFilter,
          toggleTitleQuickStatusFilter,
          clearTitleQuickFilters,
          queueExisting,
          toggleTitleMonitored,
          runInteractiveSearchForTitle,
          queueExistingFromRelease,
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
          libraryScanDisabled: libraryScanInProgress || selectedLibraryIds.length !== 1,
          libraryScanNotice,
          libraryScanSummary,
          libraries,
          librariesLoading,
          rootValidationLibraries,
          rootValidationLibrariesLoading,
          selectedLibraryIds,
          allLibrariesValue: ALL_LIBRARIES_VALUE,
          setSelectedLibraryIds,
          loadLibrarySettings,
          createLibrary,
          updateLibrary,
          deleteLibrary,
          onOpenOverview,
          scanLibrary: handleLibraryScan,
          deleteCatalogTitle: requestDeleteTitle,
          isDeletingCatalogTitleById: deleteTitleLoadingById,
          isMobile,
          viewMode: desktopViewMode,
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
      {replaceConflictDialog}
    </>
  );
});
