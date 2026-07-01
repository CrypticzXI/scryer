import * as React from "react";
import {
  ChevronDown,
  File as FileIcon,
  HardDrive,
  Loader2,
  Search,
  Star,
  Trash2,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { ExternalSubtitleSection } from "@/components/common/external-subtitle-section";
import { MediaInfoBadges, type MediaInfoFile } from "@/components/common/media-info-badges";
import { SubtitleSearchModal } from "@/components/views/subtitle-search-modal";
import { useTranslate } from "@/lib/context/translate-context";
import { useUiDateTimeFormat } from "@/lib/context/ui-settings-context";
import type { ExternalSubtitleRecord } from "@/lib/types/subtitles";
import type { UiDateTimeFormat } from "@/lib/types/settings";
import { formatUiDate } from "@/lib/utils/date-format";
import {
  boxedActionButtonBaseClass,
  boxedActionButtonToneClass,
  boxedTextActionButtonBaseClass,
} from "@/lib/utils/action-button-styles";
import { selectorId } from "@/lib/utils/dom-ids";
import { cn } from "@/lib/utils";

export type MediaFileOnDisk = MediaInfoFile & {
  id: string;
  filePath: string;
  sizeBytes: number | null | undefined;
  role?: string | null;
  createdAt?: string | null;
};

function compactMediaValue(value: string | null | undefined): string | null {
  const trimmed = value?.trim();
  return trimmed ? trimmed : null;
}

function selectedTitleResolutionLabel(file: MediaInfoFile): string | null {
  if (compactMediaValue(file.resolution)) {
    return compactMediaValue(file.resolution);
  }
  if (file.videoHeight && file.videoHeight > 0) {
    return `${file.videoHeight}p`;
  }
  return null;
}

function selectedTitleCodecLabel(file: MediaInfoFile): string | null {
  return compactMediaValue(file.videoCodecParsed) ?? compactMediaValue(file.videoCodec);
}

function selectedTitleAudioLabel(file: MediaInfoFile): string | null {
  return compactMediaValue(file.audioCodecParsed) ?? compactMediaValue(file.audioCodec);
}

function selectedTitleSubtitleLabel(file: MediaInfoFile): string {
  const subtitleCount =
    file.subtitleStreams.length ||
    file.subtitleLanguages.length ||
    file.subtitleCodecs.length;

  return subtitleCount > 0 ? `${subtitleCount} subs` : "No subs";
}

type MediaFilesOnDiskPanelProps<TFile extends MediaFileOnDisk> = {
  title?: string;
  emptyMessage: string;
  emptyHint?: string;
  emptyAction?: React.ReactNode;
  mediaFiles: TFile[];
  subtitleDownloads?: ExternalSubtitleRecord[];
  onRefreshSubtitles?: () => Promise<void> | void;
  onDeleteFile?: (fileId: string) => void;
  onMakePrimaryFile?: (fileId: string) => Promise<void> | void;
  primaryFileUpdatingId?: string | null;
  showPrimaryRoleBadge?: boolean;
  showSubtitleSearch?: boolean;
  presentation?: "default" | "selected-title";
  fileRowIdPrefix?: string;
  filePathIdPrefix?: string;
  roleIdPrefix?: string;
  subtitleSearchIdPrefix?: string;
  deleteFileIdPrefix?: string;
  makePrimaryFileIdPrefix?: string;
};

export function MediaFilesOnDiskPanel<TFile extends MediaFileOnDisk>({
  title,
  emptyMessage,
  emptyHint,
  emptyAction,
  mediaFiles,
  subtitleDownloads = [],
  onRefreshSubtitles,
  onDeleteFile,
  onMakePrimaryFile,
  primaryFileUpdatingId,
  showPrimaryRoleBadge = false,
  showSubtitleSearch = Boolean(onRefreshSubtitles),
  presentation = "default",
  fileRowIdPrefix = "media-file",
  filePathIdPrefix,
  roleIdPrefix,
  subtitleSearchIdPrefix = "media-file-search-subtitles",
  deleteFileIdPrefix = "media-file-delete",
  makePrimaryFileIdPrefix = "media-file-make-primary",
}: MediaFilesOnDiskPanelProps<TFile>) {
  const t = useTranslate();
  const dateTimeFormat = useUiDateTimeFormat();
  const [subtitleSearchTarget, setSubtitleSearchTarget] = React.useState<{
    mediaFileId: string;
    filePath: string;
  } | null>(null);
  const subtitleDownloadsByMediaFile = React.useMemo(
    () =>
      subtitleDownloads.reduce<Record<string, ExternalSubtitleRecord[]>>(
        (grouped, download) => {
          (grouped[download.mediaFileId] ??= []).push(download);
          return grouped;
        },
        {},
      ),
    [subtitleDownloads],
  );
  const canSearchSubtitles = showSubtitleSearch && Boolean(onRefreshSubtitles);
  const selectedTitlePresentation = presentation === "selected-title";
  const orderedMediaFiles = React.useMemo(
    () =>
      mediaFiles
        .map((file, index) => ({ file, index }))
        .sort((left, right) => {
          const roleDelta =
            mediaFileRoleSortRank(left.file.role) - mediaFileRoleSortRank(right.file.role);
          return roleDelta !== 0 ? roleDelta : left.index - right.index;
        })
        .map(({ file }) => file),
    [mediaFiles],
  );

  return (
    <div>
      {title ? (
        <p className="mb-1 text-xs font-medium text-muted-foreground">{title}</p>
      ) : null}
      {orderedMediaFiles.length > 0 ? (
        <div className={cn(selectedTitlePresentation ? "space-y-4" : "space-y-2")}>
          {orderedMediaFiles.map((file, fileIndex) => {
            const downloads = subtitleDownloadsByMediaFile[file.id] ?? [];
            const role = file.role?.toLowerCase() ?? "";
            const isAdditionalFile = role === "additional";
            const isPrimaryFile = role === "primary";
            const isPromotingFile = primaryFileUpdatingId === file.id;
            const fileDate = formatMediaFileDate(file.createdAt, dateTimeFormat);
            const PathIcon = selectedTitlePresentation ? FileIcon : HardDrive;
            const unknownLabel = t("label.unknown");
            const selectedTitleBadges = [
              {
                className:
                  "bg-[var(--scry-facet-series-bg)] text-[var(--scry-facet-series-text)]",
                label: selectedTitleResolutionLabel(file) ?? unknownLabel,
              },
              {
                className:
                  "bg-[var(--scry-facet-movie-bg)] text-[var(--scry-facet-movie-text)]",
                label: selectedTitleCodecLabel(file) ?? unknownLabel,
              },
              {
                className:
                  "bg-[var(--scry-facet-anime-bg)] text-[var(--scry-facet-anime-text)]",
                label: selectedTitleAudioLabel(file) ?? unknownLabel,
              },
            ];

            return (
              <div
                key={file.id}
                id={selectorId(fileRowIdPrefix, file.id)}
                className={cn(
                  "text-sm",
                  selectedTitlePresentation
                    ? fileIndex === 0
                      ? "bg-transparent"
                      : "border-t border-[var(--scry-line3)] pt-4"
                    : "rounded-lg bg-card/55 px-4 py-4",
                )}
              >
                <div
                  className={cn(
                    "grid gap-4 lg:grid-cols-[minmax(0,1fr)_auto]",
                    selectedTitlePresentation ? "lg:items-start" : "lg:items-center",
                  )}
                >
                  <div className="min-w-0 space-y-3">
                    <div
                      className={cn(
                        "flex gap-2.5",
                        selectedTitlePresentation ? "items-center" : "items-start",
                      )}
                    >
                      <PathIcon
                        className={cn(
                          "shrink-0",
                          selectedTitlePresentation
                            ? "h-3.5 w-3.5 text-[var(--scry-faint)]"
                            : "mt-0.5 h-3.5 w-3.5 text-muted-foreground/60",
                        )}
                      />
                      <p
                        id={filePathIdPrefix ? selectorId(filePathIdPrefix, file.id) : undefined}
                        title={file.filePath}
                        className={cn(
                          "min-w-0 leading-5",
                          selectedTitlePresentation
                            ? "truncate font-[var(--font-code)] text-[12px] text-[var(--scry-text2)]"
                            : "break-all font-[var(--font-code)] text-sm text-muted-foreground",
                        )}
                      >
                        {file.filePath}
                      </p>
                    </div>
                    <div
                      className={cn(
                        "flex flex-wrap items-center gap-2",
                        selectedTitlePresentation
                          ? "text-[11px] text-[var(--scry-faint)]"
                          : "text-xs text-muted-foreground/60",
                      )}
                    >
                      {showPrimaryRoleBadge && isPrimaryFile ? (
                        <span
                          id={selectorId(roleIdPrefix ?? fileRowIdPrefix, "primary", file.id)}
                          className={cn(
                            selectedTitlePresentation
                              ? "rounded-[6px] bg-emerald-500/15 px-2 py-0.5 text-[10.5px] font-bold text-emerald-300"
                              : "rounded-full border border-emerald-500/30 bg-emerald-500/15 px-1.5 py-0.5 text-emerald-700 dark:text-emerald-300",
                          )}
                        >
                          {t("mediaFile.primary")}
                        </span>
                      ) : null}
                      {isAdditionalFile ? (
                        <span
                          id={selectorId(roleIdPrefix ?? fileRowIdPrefix, "additional", file.id)}
                          className={cn(
                            selectedTitlePresentation
                              ? "rounded-[6px] bg-sky-500/15 px-2 py-0.5 text-[10.5px] font-bold text-sky-300"
                              : "rounded-full border border-sky-500/30 bg-sky-500/15 px-1.5 py-0.5 text-sky-700 dark:text-sky-300",
                          )}
                        >
                          {t("mediaFile.additional")}
                        </span>
                      ) : null}
                      {fileDate ? <span>{fileDate}</span> : null}
                      {file.acquisitionScore != null ? (
                        <span title={file.scoringLog ?? undefined}>
                          {t("mediaFile.score", { score: file.acquisitionScore })}
                        </span>
                      ) : null}
                    </div>
                    {selectedTitlePresentation ? (
                      <div className="flex flex-wrap gap-1.5">
                        {selectedTitleBadges.map((badge, badgeIndex) => (
                          <span
                            key={`${badge.label}-${badgeIndex}`}
                            className={cn(
                              "rounded-[6px] px-[9px] py-[3px] text-[10.5px] font-semibold",
                              badge.className,
                            )}
                          >
                            {badge.label}
                          </span>
                        ))}
                        <span className="inline-flex items-center gap-1 rounded-[6px] bg-[var(--scry-chip)] px-[9px] py-[3px] text-[10.5px] font-semibold text-[var(--scry-muted2)]">
                          {selectedTitleSubtitleLabel(file)}
                          <ChevronDown className="h-[11px] w-[11px]" />
                        </span>
                      </div>
                    ) : (
                      <MediaInfoBadges file={file} />
                    )}
                    <ExternalSubtitleSection
                      downloads={downloads}
                      onChanged={onRefreshSubtitles}
                    />
                  </div>
                  <div
                    className={cn(
                      "flex shrink-0 flex-col items-start gap-3 lg:items-end lg:self-start",
                      selectedTitlePresentation ? "lg:pl-4" : "lg:pl-6",
                    )}
                  >
                    <div className="text-left sm:text-right">
                      <div
                        className={cn(
                          "font-semibold tracking-tight",
                          selectedTitlePresentation
                            ? "text-[23px] text-white"
                            : "text-3xl text-foreground sm:text-4xl",
                        )}
                        style={{
                          fontFamily:
                            "var(--font-space-grotesk), var(--font-inter), ui-sans-serif, system-ui, -apple-system, sans-serif",
                        }}
                      >
                        {formatMediaFileSize(file.sizeBytes)}
                      </div>
                    </div>
                    {(canSearchSubtitles || onMakePrimaryFile || onDeleteFile) ? (
                      <div className="flex items-start gap-2 lg:justify-end">
                        <div className="flex flex-wrap items-center gap-2">
                          {canSearchSubtitles ? (
                            <Button
                              type="button"
                              size="sm"
                              variant="secondary"
                              id={selectorId(subtitleSearchIdPrefix, file.id)}
                              className={
                                selectedTitlePresentation
                                  ? "h-8 rounded-[8px] border border-[var(--scry-border2)] bg-[var(--scry-soft)] px-3 text-[11.5px] font-medium text-[var(--scry-text2)] shadow-none hover:bg-[var(--scry-hover)]"
                                  : cn(
                                      boxedTextActionButtonBaseClass,
                                      boxedActionButtonToneClass.search,
                                    )
                              }
                              onClick={() =>
                                setSubtitleSearchTarget({
                                  mediaFileId: file.id,
                                  filePath: file.filePath,
                                })
                              }
                              title={t("subtitle.search")}
                              aria-label={t("subtitle.search")}
                            >
                              <Search className="mr-1.5 h-3.5 w-3.5" />
                              {t("subtitle.search")}
                            </Button>
                          ) : null}
                          {isAdditionalFile && onMakePrimaryFile ? (
                            <Button
                              type="button"
                              size="sm"
                              variant="secondary"
                              id={selectorId(makePrimaryFileIdPrefix, file.id)}
                              className={
                                selectedTitlePresentation
                                  ? "h-8 rounded-[8px] border border-[var(--scry-border2)] bg-[var(--scry-soft)] px-3 text-[11.5px] font-medium text-[var(--scry-text2)] shadow-none hover:bg-[var(--scry-hover)]"
                                  : cn(
                                      boxedTextActionButtonBaseClass,
                                      boxedActionButtonToneClass.search,
                                    )
                              }
                              onClick={() => {
                                void onMakePrimaryFile(file.id);
                              }}
                              disabled={isPromotingFile}
                              title={t("mediaFile.makePrimary")}
                              aria-label={t("mediaFile.makePrimary")}
                            >
                              {isPromotingFile ? (
                                <Loader2 className="mr-1.5 h-3.5 w-3.5 animate-spin" />
                              ) : (
                                <Star className="mr-1.5 h-3.5 w-3.5" />
                              )}
                              {t("mediaFile.makePrimary")}
                            </Button>
                          ) : null}
                        </div>
                        {onDeleteFile ? (
                          <Button
                            type="button"
                            size="icon-sm"
                            variant="secondary"
                            id={selectorId(deleteFileIdPrefix, file.id)}
                            onClick={() => onDeleteFile(file.id)}
                            className={
                              selectedTitlePresentation
                                ? "h-8 w-8 rounded-[8px] border border-[#3a1620] bg-[rgba(120,30,40,0.25)] p-0 text-[#ef6a7a] shadow-none hover:bg-[rgba(120,30,40,0.34)] hover:text-[#ef6a7a]"
                                : cn(
                                    boxedActionButtonBaseClass,
                                    boxedActionButtonToneClass.delete,
                                  )
                            }
                            title={t("mediaFile.delete")}
                            aria-label={t("mediaFile.delete")}
                          >
                            <Trash2 className="h-4 w-4" />
                          </Button>
                        ) : null}
                      </div>
                    ) : null}
                  </div>
                </div>
              </div>
            );
          })}
        </div>
      ) : (
        <div className="space-y-3">
          <p className="text-sm italic text-muted-foreground/60">
            {emptyMessage}
            {emptyHint ? ` ${emptyHint}` : ""}
          </p>
          {emptyAction}
        </div>
      )}
      {subtitleSearchTarget && onRefreshSubtitles ? (
        <SubtitleSearchModal
          open={true}
          onOpenChange={(open) => {
            if (!open) {
              setSubtitleSearchTarget(null);
            }
          }}
          mediaFileId={subtitleSearchTarget.mediaFileId}
          filePath={subtitleSearchTarget.filePath}
          downloads={subtitleDownloadsByMediaFile[subtitleSearchTarget.mediaFileId] ?? []}
          onChanged={() => {
            void onRefreshSubtitles();
          }}
        />
      ) : null}
    </div>
  );
}

function mediaFileRoleSortRank(role: string | null | undefined) {
  const normalized = role?.toLowerCase() ?? "";
  if (normalized === "primary") {
    return 0;
  }
  if (normalized === "additional") {
    return 1;
  }
  return 2;
}

function formatMediaFileDate(
  iso: string | null | undefined,
  dateTimeFormat: UiDateTimeFormat,
) {
  if (!iso) {
    return null;
  }

  return formatUiDate(iso, dateTimeFormat, { fallback: iso });
}

function formatMediaFileSize(sizeBytes: number | null | undefined) {
  const bytes = sizeBytes ?? Number.NaN;
  if (!Number.isFinite(bytes) || bytes <= 0) {
    return "-";
  }
  if (bytes >= 1024 ** 3) {
    return `${(bytes / 1024 ** 3).toFixed(2)} GB`;
  }
  if (bytes >= 1024 ** 2) {
    return `${(bytes / 1024 ** 2).toFixed(2)} MB`;
  }
  if (bytes >= 1024) {
    return `${(bytes / 1024).toFixed(2)} KB`;
  }
  return `${bytes.toFixed(0)} B`;
}
