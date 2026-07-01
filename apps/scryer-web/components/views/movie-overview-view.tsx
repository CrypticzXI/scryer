
import * as React from "react";
import { FolderOpen, Loader2, Pause, Play, RotateCcw, Search, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { TitleHistoryModal } from "@/components/common/title-history-modal";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useUiDateTimeFormat } from "@/lib/context/ui-settings-context";
import type { Translate } from "@/components/root/types";
import type { Release, WantedItem } from "@/lib/types";
import { useClient } from "urql";
import type {
  MediaRenamePlan,
  TitleReleaseBlocklistEntry,
  TitleDetail,
  TitleCollection,
  TitleHistoryEvent,
  TitleMediaFile,
} from "@/components/containers/movie-overview-container";
import { MediaFilesOnDiskPanel } from "@/components/common/media-files-on-disk-panel";
import { MediaRenamePlanPanel } from "@/components/common/media-rename-plan-panel";
import { MovieOverviewDownloadList } from "@/components/common/download-queue-overview";
import { SearchResultBuckets } from "@/components/common/release-search-results";
import { TitleSearchDownloadClientNotice } from "@/components/common/title-search-download-client-notice";
import { releaseSupportsAdditionalFileQueue } from "@/lib/utils/release-queue-scope";
import { OverviewControlPanel } from "@/components/views/overview-control-panel";
import { OverviewBackLink } from "@/components/views/overview-back-link";
import {
  TitleMoreLikeThisStrip,
  type TitleMoreLikeThisStripActions,
} from "@/components/views/title-more-like-this-strip";
import { TitleRatingsStrip } from "@/components/views/title-ratings-strip";
import {
  localizedTitleStatus,
  localizedWantedPhase,
  localizedWantedStatus,
} from "@/components/views/overview-localization";
import { TitlePosterSlot } from "@/components/title-poster-slot";
import type { TitleOptionUpdates } from "@/lib/types/title-options";
import type { LibraryRootRecord } from "@/lib/types/titles";
import type { WantedSearchPhase, WantedStatus } from "@/lib/types";
import { SubtitleLanguagePicker } from "@/components/common/subtitle-language-picker";
import { setTitleRequiredAudioMutation } from "@/lib/graphql/mutations";
import type { DownloadQueueItem } from "@/lib/types/download-queue";
import type { ExternalSubtitleRecord } from "@/lib/types/subtitles";
import type { UiDateTimeFormat } from "@/lib/types/settings";
import { formatUiDate, formatUiDateTime } from "@/lib/utils/date-format";
import {
  AnidbExternalLink,
  ImdbExternalLink,
  TmdbExternalLink,
  TvdbMovieExternalLink,
} from "@/components/common/external-media-links";
// ─── helpers ────────────────────────────────────────────────────────────────

function formatDate(iso: string, dateTimeFormat: UiDateTimeFormat) {
  return formatUiDate(iso, dateTimeFormat, { fallback: iso });
}

function formatDateTime(
  iso: string | null | undefined,
  dateTimeFormat: UiDateTimeFormat,
) {
  return formatUiDateTime(iso, dateTimeFormat, { fallback: "—" });
}

function formatRuntime(minutes: number | null | undefined) {
  if (!minutes || minutes <= 0) return null;
  const h = Math.floor(minutes / 60);
  const m = minutes % 60;
  if (h === 0) return `${m}m`;
  return m > 0 ? `${h}h ${m}m` : `${h}h`;
}

function prettifyTagValue(raw: string) {
  const trimmed = raw.trim();
  if (!trimmed) return trimmed;
  if (trimmed.toLowerCase() === "4k") return "4K";
  return trimmed;
}

function resolveMonitorTypeLabel(t: Translate, value: string) {
  switch (value) {
    case "monitored":
      return t("search.monitorType.monitored");
    case "unmonitored":
      return t("search.monitorType.unmonitored");
    case "futureEpisodes":
      return t("search.monitorType.futureEpisodes");
    case "missingAndFutureEpisodes":
      return t("search.monitorType.missingAndFutureEpisodes");
    case "allEpisodes":
      return t("search.monitorType.allEpisodes");
    case "none":
      return t("search.monitorType.none");
    default:
      return value;
  }
}

function formatTitleTag(t: Translate, tag: string, qualityProfiles?: { id: string; name: string }[]) {
  const qualityPrefix = "scryer:quality-profile:";
  const monitorPrefix = "scryer:monitor-type:";
  const seasonFolderPrefix = "scryer:season-folder:";

  if (tag.startsWith(qualityPrefix)) {
    const rawId = tag.slice(qualityPrefix.length).replace(/^"|"$/g, "");
    const profile = qualityProfiles?.find((p) => p.id === rawId);
    const value = profile ? profile.name : prettifyTagValue(rawId);
    return {
      label: `${t("settings.qualityProfileSection")}: ${value}`,
      className: "bg-indigo-500/20 text-indigo-200",
    };
  }

  if (tag.startsWith(monitorPrefix)) {
    const value = tag.slice(monitorPrefix.length).trim();
    return {
      label: `${t("search.addConfigMonitorType")}: ${resolveMonitorTypeLabel(t, value)}`,
      className: "bg-sky-500/20 text-sky-200",
    };
  }

  if (tag.startsWith(seasonFolderPrefix)) {
    const value = tag.slice(seasonFolderPrefix.length).trim();
    const translatedValue =
      value === "enabled"
        ? t("search.seasonFolder.enabled")
        : value === "disabled"
          ? t("search.seasonFolder.disabled")
          : value;
    return {
      label: `${t("search.addConfigSeasonFolder")}: ${translatedValue}`,
      className: "bg-emerald-500/20 text-emerald-700 dark:text-emerald-200",
    };
  }

  return {
    label: tag,
    className: "bg-accent text-muted-foreground",
  };
}

const MONITOR_TYPE_TAG_PREFIX = "scryer:monitor-type:";

function wantedStatusClass(status: WantedStatus) {
  switch (status) {
    case "wanted":
      return "bg-blue-500/20 text-blue-300";
    case "grabbed":
      return "bg-amber-500/20 text-amber-300";
    case "completed":
      return "bg-emerald-500/20 text-emerald-300";
    case "paused":
      return "bg-muted text-muted-foreground";
    default:
      return "bg-muted text-muted-foreground";
  }
}

function wantedPhaseClass(phase: WantedSearchPhase) {
  switch (phase) {
    case "primary":
      return "bg-emerald-500/15 text-emerald-300";
    case "pre_release":
    case "pre_air":
      return "bg-fuchsia-500/15 text-fuchsia-300";
    case "secondary":
      return "bg-yellow-500/15 text-yellow-300";
    default:
      return "bg-muted text-muted-foreground";
  }
}

// ─── title settings ──────────────────────────────────────────────────────────

const INHERIT_VALUE = "__inherit__";

function TitleSettingsPanel({
  title,
  qualityProfiles,
  defaultRootFolder,
  rootFolders,
  onUpdateTitleOptions,
  onTitleChanged,
  onOpenFixMatch,
}: {
  title: TitleDetail;
  qualityProfiles: { id: string; name: string }[];
  defaultRootFolder: string;
  rootFolders: LibraryRootRecord[];
  onUpdateTitleOptions: (options: TitleOptionUpdates) => Promise<void>;
  onTitleChanged?: () => Promise<void> | void;
  onOpenFixMatch?: () => void;
}) {
  const t = useTranslate();
  const client = useClient();
  const setGlobalStatus = useGlobalStatus();
  const currentProfileId = title.qualityProfileId?.trim() || INHERIT_VALUE;
  const currentRootFolderId = title.rootFolderId?.trim() || "";
  const sortedRootFolders = React.useMemo(
    () =>
      [...rootFolders].sort((left, right) => {
        if (left.isDefault !== right.isDefault) {
          return left.isDefault ? -1 : 1;
        }
        return left.path.localeCompare(right.path);
      }),
    [rootFolders],
  );
  const rootFolderById = React.useMemo(
    () => new Map(rootFolders.map((root) => [root.id, root])),
    [rootFolders],
  );
  const rootFolderSelectValue = rootFolderById.has(currentRootFolderId)
    ? currentRootFolderId
    : sortedRootFolders[0]?.id ?? "";
  const requiredAudioLanguages =
    title.effectiveRequiredAudioLanguages ?? [];
  const hasAudioOverride = title.inheritsRequiredAudioLanguages === false;
  const [saving, setSaving] = React.useState(false);
  const [audioSaving, setAudioSaving] = React.useState(false);

  const handleProfileChange = async (value: string) => {
    setSaving(true);
    try {
      await onUpdateTitleOptions({
        qualityProfileId: value === INHERIT_VALUE ? "" : value,
      });
    } finally {
      setSaving(false);
    }
  };

  const handleRootFolderChange = async (value: string) => {
    if (!value.trim()) {
      return;
    }
    setSaving(true);
    try {
      await onUpdateTitleOptions({
        rootFolderId: value,
      });
    } finally {
      setSaving(false);
    }
  };

  const folderLabel = (path: string) =>
    path.split("/").filter(Boolean).pop() ?? path;

  const handleRequiredAudioChange = async (languages: string[]) => {
    setAudioSaving(true);
    try {
      const { error } = await client
        .mutation(setTitleRequiredAudioMutation, {
          input: { titleId: title.id, facet: title.facet, languages },
        })
        .toPromise();
      if (error) {
        throw error;
      }
      await onTitleChanged?.();
    } catch {
      setGlobalStatus(t("status.failedToUpdate"));
    } finally {
      setAudioSaving(false);
    }
  };

  const handleResetAudioOverride = async () => {
    setAudioSaving(true);
    try {
      const { error } = await client
        .mutation(setTitleRequiredAudioMutation, {
          input: { titleId: title.id, facet: title.facet, languages: null },
        })
        .toPromise();
      if (error) {
        throw error;
      }
      await onTitleChanged?.();
    } catch {
      setGlobalStatus(t("status.failedToUpdate"));
    } finally {
      setAudioSaving(false);
    }
  };

  return (
    <div id="movie-overview-title-settings" className="p-4">
      <div className="grid gap-4 md:grid-cols-2">
        <div className="min-w-0">
          <label className="mb-1 block text-xs font-medium text-muted-foreground">
            {t("title.qualityProfile")}
          </label>
          <Select
            value={currentProfileId}
            onValueChange={(v) => void handleProfileChange(v)}
            disabled={saving || qualityProfiles.length === 0}
          >
            <SelectTrigger id="movie-overview-settings-quality-profile" className="h-9 w-full">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value={INHERIT_VALUE}>
                {t("title.inheritDefault")}
              </SelectItem>
              {qualityProfiles.map((p) => (
                <SelectItem key={p.id} value={p.id}>
                  {p.name}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>

        <div className="min-w-0">
          <label className="mb-1 block text-xs font-medium text-muted-foreground">
            {t("title.rootFolder")}
          </label>
          <Select
            value={rootFolderSelectValue}
            onValueChange={(value) => void handleRootFolderChange(value)}
            disabled={saving || sortedRootFolders.length === 0}
          >
            <SelectTrigger id="movie-overview-settings-root-folder" className="h-9 w-full font-[var(--font-code)] text-sm">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {sortedRootFolders.map((root) => (
                <SelectItem key={root.id} value={root.id}>
                  {root.isDefault
                    ? t("title.defaultRootFolder", {
                        path: folderLabel(root.path || defaultRootFolder),
                      })
                    : folderLabel(root.path)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>

        <div className="min-w-0">
          <label className="mb-1 block text-xs font-medium text-muted-foreground">
            {t("title.requiredAudioLanguages")}
          </label>
          <div id="movie-overview-settings-required-audio-languages">
            <SubtitleLanguagePicker
              value={requiredAudioLanguages}
              onChange={(codes) => void handleRequiredAudioChange(codes)}
              compact
              disabled={audioSaving}
            />
          </div>
          {hasAudioOverride ? (
            <button
              id="movie-overview-settings-required-audio-reset"
              type="button"
              className="mt-1 text-xs text-primary hover:underline"
              onClick={() => void handleResetAudioOverride()}
              disabled={audioSaving}
            >
              {t("title.requiredAudioResetInherit")}
            </button>
          ) : null}
        </div>
      </div>

      {onOpenFixMatch ? (
        <div className="mt-5 flex items-center justify-between gap-3 rounded-lg border border-border/70 bg-muted/20 px-3 py-3">
          <div className="min-w-0">
            <p className="text-sm font-medium text-foreground">{t("title.fixMatchHeading")}</p>
            <p className="text-xs text-muted-foreground">
              {t("title.fixMatchDescriptionMovie")}
            </p>
          </div>
          <Button
            id="movie-overview-settings-fix-match"
            type="button"
            variant="primary"
            size="sm"
            className="shrink-0"
            onClick={onOpenFixMatch}
          >
            <Search className="mr-2 h-4 w-4" />
            {t("title.fixMatchAction")}
          </Button>
        </div>
      ) : null}
    </div>
  );
}

// ─── main view ────────────────────────────────────────────────────────────────

type Props = {
  canManageTitle: boolean;
  loading: boolean;
  title: TitleDetail | null;
  collections: TitleCollection[];
  events: TitleHistoryEvent[];
  searchResults: Release[];
  searching: boolean;
  hasDownloadClients: boolean;
  showSearchPrerequisiteNotice: boolean;
  renamePlan: MediaRenamePlan | null;
  renameEnabled: boolean;
  renamePreviewing: boolean;
  renameApplying: boolean;
  interactiveSearchAttempted: boolean;
  searchMonitoredLoading: boolean;
  refreshAndScanLoading: boolean;
  deleteLoading: boolean;
  onSearch: () => void;
  onQueue: (r: Release) => void;
  onQueueAdditional?: (r: Release) => void;
  onSearchMonitored: () => void;
  onRefreshAndScan: () => void;
  onTitleChanged?: () => Promise<void> | void;
  onPreviewRename: () => void;
  onApplyRename: () => void;
  onBackToList?: () => void;
  qualityProfiles: { id: string; name: string }[];
  defaultRootFolder: string;
  rootFolders: LibraryRootRecord[];
  onUpdateTitleOptions: (options: TitleOptionUpdates) => Promise<void>;
  onSetTitleMonitored: (monitored: boolean) => Promise<void>;
  monitoredUpdating: boolean;
  wantedItem: WantedItem | null;
  wantedActionLoading: "pause" | "resume" | "reset" | null;
  onPauseWanted: () => Promise<void>;
  onResumeWanted: () => Promise<void>;
  onResetWanted: () => Promise<void>;
  onTriggerMismatchRecovery: () => Promise<void>;
  onRequestDeleteTitle?: () => void;
  blocklistEntries: TitleReleaseBlocklistEntry[];
  clearingReleaseBlocklistEntryId?: string | null;
  onClearReleaseBlocklistEntry?: (entryId: string) => Promise<void> | void;
  mediaFiles: TitleMediaFile[];
  downloadQueueItems: DownloadQueueItem[];
  subtitleDownloads: ExternalSubtitleRecord[];
  primaryMovieFileUpdatingId?: string | null;
  onDeleteFile?: (fileId: string) => void;
  onMakePrimaryFile?: (fileId: string) => Promise<void> | void;
  onRefreshSubtitles?: () => void;
  onOpenFixMatch?: () => void;
  moreLikeThisActions?: TitleMoreLikeThisStripActions;
};

export function MovieOverviewView({
  canManageTitle,
  loading,
  title,
  collections = [],
  events: _events,
  searchResults = [],
  searching,
  hasDownloadClients,
  showSearchPrerequisiteNotice,
  renamePlan,
  renameEnabled,
  renamePreviewing,
  renameApplying,
  interactiveSearchAttempted,
  searchMonitoredLoading,
  refreshAndScanLoading,
  deleteLoading,
  onSearch,
  onQueue,
  onQueueAdditional,
  onSearchMonitored,
  onRefreshAndScan,
  onTitleChanged,
  onPreviewRename,
  onApplyRename,
  onBackToList,
  qualityProfiles,
  defaultRootFolder,
  rootFolders,
  onUpdateTitleOptions,
  onSetTitleMonitored,
  monitoredUpdating,
  wantedItem,
  wantedActionLoading,
  onPauseWanted,
  onResumeWanted,
  onResetWanted,
  onTriggerMismatchRecovery,
  onRequestDeleteTitle,
  blocklistEntries = [],
  clearingReleaseBlocklistEntryId = null,
  onClearReleaseBlocklistEntry,
  mediaFiles = [],
  downloadQueueItems = [],
  subtitleDownloads = [],
  primaryMovieFileUpdatingId = null,
  onDeleteFile,
  onMakePrimaryFile,
  onRefreshSubtitles,
  onOpenFixMatch,
  moreLikeThisActions,
}: Props) {
  const t = useTranslate();
  const dateTimeFormat = useUiDateTimeFormat();
  const [historyOpen, setHistoryOpen] = React.useState(false);
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
          id="movie-overview-back-link"
          label={t("title.backToFacet", { facet: t("nav.movies") })}
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

  const externalIds = title.externalIds ?? [];
  const imdbId = title.imdbId ?? externalIds.find((e) => e.source === "imdb")?.value;
  const anidbId = externalIds.find((e) => e.source === "anidb")?.value;
  const tmdbId = externalIds.find((e) => e.source === "tmdb")?.value;
  const tvdbId = externalIds.find((e) => e.source === "tvdb")?.value;

  const posterUrl = title.posterUrl;
  const overview = title.overview;
  const genres = title.genres ?? [];
  const runtime = formatRuntime(title.runtimeMinutes);
  const year = title.year;
  const studio = title.studio;
  const sortedMediaFiles = mediaFiles
    .map((file, index) => ({ file, index }))
    .sort((left, right) => {
      const leftRank = left.file.role === "primary" ? 0 : 1;
      const rightRank = right.file.role === "primary" ? 0 : 1;
      return leftRank - rightRank || left.index - right.index;
    })
    .map(({ file }) => file);
  const hasMediaFiles = sortedMediaFiles.length > 0;
  const orphanCollections = collections.filter(
    (collection) => !collection.orderedPath || !mediaFiles.some((file) => file.filePath === collection.orderedPath),
  );
  const wantedStatusLabel = wantedItem?.status
    ? localizedWantedStatus(t, wantedItem.status)
    : null;
  const wantedPhaseLabel = wantedItem?.searchPhase
    ? localizedWantedPhase(t, wantedItem.searchPhase)
    : null;
  const searchPrerequisiteNotice = canManageTitle && !hasDownloadClients && showSearchPrerequisiteNotice
    ? <TitleSearchDownloadClientNotice />
    : null;
  const interactiveSearchPanel = canManageTitle ? (
    <div className="p-4">
      {searchPrerequisiteNotice ? (
        searchPrerequisiteNotice
      ) : searching ? (
        <div className="flex flex-col items-center gap-4 py-8">
          <Loader2 className="h-8 w-8 animate-spin text-emerald-500" />
          <p className="text-sm text-muted-foreground">{t("title.searchingReleases")}</p>
          <div className="w-full space-y-2">
            {[1, 2, 3].map((index) => (
              <div
                key={index}
                className="h-12 animate-pulse rounded-lg bg-muted"
                style={{ animationDelay: `${index * 150}ms` }}
              />
            ))}
          </div>
        </div>
      ) : searchResults.length > 0 ? (
        <SearchResultBuckets
          results={searchResults}
          onQueue={onQueue}
          onQueueAdditional={onQueueAdditional}
          canQueueAdditional={(release) =>
            releaseSupportsAdditionalFileQueue(release, title.facet)
          }
          requireCandidateToken
        />
      ) : interactiveSearchAttempted ? (
        <p className="text-sm text-muted-foreground">
          {t("title.noReleasesFound", { name: title.name })}
        </p>
      ) : null}
    </div>
  ) : null;
  return (
    <div className="space-y-4">
      {/* title header with poster */}
      {(() => {
        const overviewBackdropUrl = title.backgroundUrl;
        return (
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
                src={posterUrl}
                sourceSrc={title.posterSourceUrl}
                metadataFetchedAt={title.metadataFetchedAt}
                createdAt={title.createdAt}
                alt={title.name}
                className="block h-48 w-32 rounded-lg object-cover shadow-lg sm:h-[270px] sm:w-[180px]"
                placeholderClassName="flex h-48 w-32 items-center justify-center rounded-lg bg-muted text-sm text-muted-foreground/60 sm:h-[270px] sm:w-[180px]"
                emptyLabel={t("title.noPoster")}
              />
            </div>

            <div className="relative min-w-0 flex-1 flex flex-col pr-12">
              {onBackToList ? (
                <Button
                  type="button"
                  variant="outline"
                  size="icon"
                  className="absolute right-0 top-0 z-20 size-10 rounded-[11px] border border-[var(--scry-border2)] bg-[var(--scry-card2)] text-[var(--scry-ink2)] shadow-[0_12px_30px_rgba(0,0,0,0.35)] backdrop-blur-sm transition hover:bg-[var(--scry-hover)] hover:text-[var(--scry-ink2)]"
                  aria-label={t("label.close")}
                  title={t("label.close")}
                  onClick={() => onBackToList()}
                >
                  <X className="h-5 w-5" />
                </Button>
              ) : null}
              <h1 className="text-xl font-bold text-foreground sm:text-2xl">
                {title.name}
                {year ? (
                  <span className="block text-base font-normal text-muted-foreground sm:ml-2 sm:inline sm:text-lg">
                    ({year})
                  </span>
                ) : null}
              </h1>

              <div className="mt-2 flex flex-wrap items-center gap-2">
                <span className={`inline-flex items-center rounded-full px-2.5 py-0.5 text-xs font-medium ${title.monitored ? "bg-emerald-500/20 text-emerald-700 dark:text-emerald-300" : "bg-accent text-muted-foreground"}`}>
                  {title.monitored
                    ? t("title.monitored")
                    : t("search.monitorType.unmonitored")}
                </span>
              {localizedTitleStatus(t, title.contentStatus) ? (
                <span className="inline-flex items-center rounded-full border border-border px-2.5 py-0.5 text-xs font-medium capitalize text-muted-foreground">
                  {localizedTitleStatus(t, title.contentStatus)}
                </span>
              ) : null}
                {runtime ? (
                  <span className="text-xs text-muted-foreground">{runtime}</span>
                ) : null}
                {studio ? (
                  <span className="text-xs text-muted-foreground">{studio}</span>
                ) : null}
                {(title.tags ?? [])
                  .filter((tag) => !tag.startsWith(MONITOR_TYPE_TAG_PREFIX))
                  .map((tag) => {
                  const formattedTag = formatTitleTag(t, tag, qualityProfiles);
                  return (
                    <span
                      key={tag}
                      className={`inline-flex items-center rounded-full px-2.5 py-0.5 text-xs font-medium ${formattedTag.className}`}
                    >
                      {formattedTag.label}
                    </span>
                  );
                })}
              </div>

              <div className="mt-3 flex flex-wrap items-center gap-2">
                {wantedItem ? (
                  <>
                    <span
                      className={`inline-flex items-center rounded-full px-2.5 py-0.5 text-xs font-medium ${wantedStatusClass(wantedItem.status)}`}
                    >
                      {wantedStatusLabel}
                    </span>
                    <span
                      className={`inline-flex items-center rounded-full px-2.5 py-0.5 text-xs font-medium capitalize ${wantedPhaseClass(wantedItem.searchPhase)}`}
                    >
                      {wantedPhaseLabel}
                    </span>
                    <span className="text-xs text-muted-foreground">
                      {t("wanted.colNextSearch")}:{" "}
                      {formatDateTime(wantedItem.nextSearchAt, dateTimeFormat)}
                    </span>
                    {canManageTitle ? (
                      <>
                        {wantedItem.status === "paused" ? (
                          <Button
                            size="sm"
                            variant="ghost"
                            onClick={() => void onResumeWanted()}
                            disabled={wantedActionLoading !== null}
                          >
                            {wantedActionLoading === "resume" ? (
                              <Loader2 className="h-4 w-4 animate-spin" />
                            ) : (
                              <Play className="h-4 w-4" />
                            )}
                            {t("wanted.resume")}
                          </Button>
                        ) : (
                          <Button
                            size="sm"
                            variant="ghost"
                            onClick={() => void onPauseWanted()}
                            disabled={wantedActionLoading !== null}
                          >
                            {wantedActionLoading === "pause" ? (
                              <Loader2 className="h-4 w-4 animate-spin" />
                            ) : (
                              <Pause className="h-4 w-4" />
                            )}
                            {t("wanted.pause")}
                          </Button>
                        )}
                        <Button
                          size="sm"
                          variant="ghost"
                          onClick={() => void onResetWanted()}
                          disabled={wantedActionLoading !== null}
                        >
                          {wantedActionLoading === "reset" ? (
                            <Loader2 className="h-4 w-4 animate-spin" />
                          ) : (
                            <RotateCcw className="h-4 w-4" />
                          )}
                          {t("wanted.reset")}
                        </Button>
                        {wantedItem.mismatchRecoveryEligible ? (
                          <Button
                            size="sm"
                            variant="ghost"
                            onClick={() => void onTriggerMismatchRecovery()}
                            disabled={wantedActionLoading !== null}
                          >
                            <RotateCcw className="h-4 w-4" />
                            Recover mismatch
                          </Button>
                        ) : null}
                      </>
                    ) : null}
                  </>
                ) : title.monitored && !hasMediaFiles ? (
                  <span className="text-xs text-muted-foreground">{t("title.noWantedItem")}</span>
                ) : null}
              </div>

              {/* genres */}
              {genres.length > 0 ? (
                <div className="mt-2 flex flex-wrap gap-1.5">
                  {genres.map((genre) => (
                    <span key={genre} className="rounded bg-muted px-2 py-0.5 text-xs text-muted-foreground">
                      {genre}
                    </span>
                  ))}
                </div>
              ) : null}

              <TitleRatingsStrip ratings={title.ratings} />

              {overview ? (
                <p className="mt-4 text-sm leading-relaxed text-foreground/70">{overview}</p>
              ) : null}

              <div className="mt-auto flex flex-wrap gap-3 pt-3 text-sm">
                <ImdbExternalLink imdbId={imdbId} />
                <TvdbMovieExternalLink tvdbId={tvdbId} slug={title.slug} />
                <TmdbExternalLink mediaType="movie" tmdbId={tmdbId} />
                <AnidbExternalLink anidbId={anidbId} />
                {(title.externalIds ?? [])
                  .filter((e) => e.source !== "imdb" && e.source !== "tvdb" && e.source !== "anidb" && e.source !== "tmdb")
                  .map((e) => (
                    <div key={e.source}>
                      <span className="text-muted-foreground capitalize">{e.source} </span>
                      <span className="font-[var(--font-code)] text-card-foreground">{e.value}</span>
                    </div>
                  ))}
                <span className="ml-auto text-xs text-muted-foreground/60">
                  {t("title.addedAt", {
                    date: formatDate(title.createdAt, dateTimeFormat),
                  })}
                </span>
              </div>
            </div>
          </div>
        </CardContent>
      </Card>
        );
      })()}

      {canManageTitle ? (
        <OverviewControlPanel
          monitored={title.monitored}
          searchMonitoredLabel={t("label.search")}
          monitoredUpdating={monitoredUpdating}
          searchMonitoredLoading={searchMonitoredLoading}
          interactiveSearchLoading={searching}
          refreshAndScanLoading={refreshAndScanLoading}
          deleteLoading={deleteLoading}
          onToggleMonitoring={() => void onSetTitleMonitored(!title.monitored)}
          onSearchMonitored={() => void onSearchMonitored()}
          onInteractiveSearch={() => void onSearch()}
          onRefreshAndScan={() => void onRefreshAndScan()}
          onRequestDelete={onRequestDeleteTitle}
          onHistory={() => setHistoryOpen(true)}
          searchNotice={searchPrerequisiteNotice}
          settingsPanel={(
              <TitleSettingsPanel
                title={title}
                qualityProfiles={qualityProfiles}
                defaultRootFolder={defaultRootFolder}
                rootFolders={rootFolders}
                onUpdateTitleOptions={onUpdateTitleOptions}
                onTitleChanged={onTitleChanged}
                onOpenFixMatch={onOpenFixMatch}
              />
          )}
          interactiveSearchPanel={interactiveSearchPanel}
        />
      ) : null}

      {downloadQueueItems.length > 0 ? (
        <Card>
          <CardHeader>
            <CardTitle className="text-base">{t("activity.activity")}</CardTitle>
          </CardHeader>
          <CardContent>
            <MovieOverviewDownloadList items={downloadQueueItems} />
          </CardContent>
        </Card>
      ) : null}

      {/* files on disk */}
      <Card>
        <CardHeader>
          <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
              <CardTitle className="flex items-center gap-2 text-base">
                <FolderOpen className="h-4 w-4" />
              {t("title.filesOnDisk")}
              </CardTitle>
            {canManageTitle && renameEnabled ? (
              <Button
                id="movie-overview-rename-preview"
                className="w-full sm:w-auto"
                size="sm"
                variant="primary"
                onClick={onPreviewRename}
                disabled={renamePreviewing || collections.length === 0}
              >
                {renamePreviewing ? t("rename.previewing") : t("rename.previewButton")}
              </Button>
            ) : null}
          </div>
        </CardHeader>
        <CardContent>
          {sortedMediaFiles.length === 0 && orphanCollections.length === 0 ? (
            <MediaFilesOnDiskPanel<TitleMediaFile>
              emptyMessage={t("title.noFilesTracked")}
              emptyHint={t("title.noFilesTrackedHint")}
              mediaFiles={[]}
              showSubtitleSearch={false}
              emptyAction={canManageTitle ? (
                <Button
                  id="movie-overview-files-refresh-and-scan"
                  size="sm"
                  onClick={onRefreshAndScan}
                  disabled={refreshAndScanLoading}
                >
                  {t("settings.libraryScanButton")}
                </Button>
              ) : null}
            />
          ) : (
            <div className="space-y-2">
              {sortedMediaFiles.length > 0 ? (
                <MediaFilesOnDiskPanel<TitleMediaFile>
                  emptyMessage={t("title.noFilesTracked")}
                  mediaFiles={sortedMediaFiles}
                  subtitleDownloads={subtitleDownloads}
                  onRefreshSubtitles={canManageTitle ? onRefreshSubtitles : undefined}
                  onDeleteFile={canManageTitle ? onDeleteFile : undefined}
                  onMakePrimaryFile={canManageTitle ? onMakePrimaryFile : undefined}
                  primaryFileUpdatingId={primaryMovieFileUpdatingId}
                  showPrimaryRoleBadge
                  showSubtitleSearch={canManageTitle}
                  fileRowIdPrefix="movie-overview-media-file"
                  filePathIdPrefix="movie-overview-media-file-path"
                  roleIdPrefix="movie-overview-media-file-role"
                  subtitleSearchIdPrefix="movie-overview-search-subtitles"
                  deleteFileIdPrefix="movie-overview-delete-file"
                  makePrimaryFileIdPrefix="movie-overview-make-primary-file"
                />
              ) : null}

              {orphanCollections.map((collection) => {
                const qualityHint = collection.label ?? null;
                return (
                  <div key={collection.id} className="rounded-lg border border-border p-3">
                    <div className="flex items-start justify-between gap-2">
                      <div className="min-w-0 space-y-1.5">
                        {collection.orderedPath ? (
                          <p className="truncate font-[var(--font-code)] text-xs text-muted-foreground">{collection.orderedPath}</p>
                        ) : (
                          <p className="text-sm text-muted-foreground">{t("mediaFile.pathNotRecorded")}</p>
                        )}
                        <div className="flex flex-wrap gap-2 text-xs text-muted-foreground">
                          <span className="capitalize">{collection.collectionType}</span>
                          {qualityHint ? (
                            <span className="rounded bg-accent px-1 py-0.5 text-card-foreground">{qualityHint}</span>
                          ) : null}
                          <span className="text-muted-foreground/60">
                            {t("title.addedAt", {
                              date: formatDate(collection.createdAt, dateTimeFormat),
                            })}
                          </span>
                        </div>
                      </div>
                    </div>
                  </div>
                );
              })}
            </div>
          )}

          {canManageTitle && renamePlan ? (
            <div className="mt-5">
              <MediaRenamePlanPanel
                plan={renamePlan}
                applying={renameApplying}
                applyDisabled={renameApplying || renamePlan.renamable === 0}
                applyButtonId="movie-overview-rename-apply"
                onApply={onApplyRename}
              />
            </div>
          ) : null}
          </CardContent>
        </Card>

      <TitleMoreLikeThisStrip
        items={title.moreLikeThis ?? []}
        fallbackYearLabel={t("nav.movies")}
        {...moreLikeThisActions}
      />

      <details className="rounded-xl border border-border bg-card text-card-foreground overflow-hidden">
        <summary className="cursor-pointer select-none px-4 py-3 text-sm font-medium text-card-foreground">
          <span className="inline-flex items-center gap-2">
            {t("title.blockedReleases")}
            <span className="rounded-full bg-muted px-2 py-0.5 text-xs text-muted-foreground">
              {blocklistEntries.length}
            </span>
          </span>
        </summary>
        <div className="border-t border-border p-4">
          {blocklistEntries.length === 0 ? (
            <p className="text-sm text-muted-foreground">
              {t("title.noBlockedReleases")}
            </p>
          ) : (
            <div className="space-y-2">
              {blocklistEntries.map((entry) => (
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
                        <span className="text-muted-foreground/60">
                          {formatDateTime(entry.attemptedAt, dateTimeFormat)}
                        </span>
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
        />
      ) : null}
    </div>
  );
}
