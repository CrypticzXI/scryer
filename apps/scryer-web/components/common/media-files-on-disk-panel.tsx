import * as React from "react";
import { HardDrive, Loader2, Search, Star, Trash2 } from "lucide-react";
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
        <div className="space-y-2">
          {orderedMediaFiles.map((file) => {
            const downloads = subtitleDownloadsByMediaFile[file.id] ?? [];
            const role = file.role?.toLowerCase() ?? "";
            const isAdditionalFile = role === "additional";
            const isPrimaryFile = role === "primary";
            const isPromotingFile = primaryFileUpdatingId === file.id;
            const fileDate = formatMediaFileDate(file.createdAt, dateTimeFormat);

            return (
              <div
                key={file.id}
                id={selectorId(fileRowIdPrefix, file.id)}
                className="rounded-lg bg-card/55 px-4 py-4 text-sm"
              >
                <div className="grid gap-4 lg:grid-cols-[minmax(0,1fr)_auto] lg:items-center">
                  <div className="min-w-0 space-y-3">
                    <div className="flex items-start gap-2.5">
                      <HardDrive className="mt-0.5 h-3.5 w-3.5 shrink-0 text-muted-foreground/60" />
                      <p
                        id={filePathIdPrefix ? selectorId(filePathIdPrefix, file.id) : undefined}
                        className="min-w-0 break-all font-mono text-sm leading-5 text-muted-foreground"
                      >
                        {file.filePath}
                      </p>
                    </div>
                    <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground/60">
                      {showPrimaryRoleBadge && isPrimaryFile ? (
                        <span
                          id={selectorId(roleIdPrefix ?? fileRowIdPrefix, "primary", file.id)}
                          className="rounded-full border border-emerald-500/30 bg-emerald-500/15 px-1.5 py-0.5 text-emerald-700 dark:text-emerald-300"
                        >
                          {t("mediaFile.primary")}
                        </span>
                      ) : null}
                      {isAdditionalFile ? (
                        <span
                          id={selectorId(roleIdPrefix ?? fileRowIdPrefix, "additional", file.id)}
                          className="rounded-full border border-sky-500/30 bg-sky-500/15 px-1.5 py-0.5 text-sky-700 dark:text-sky-300"
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
                    <MediaInfoBadges file={file} />
                    <ExternalSubtitleSection
                      downloads={downloads}
                      onChanged={onRefreshSubtitles}
                    />
                  </div>
                  <div className="flex shrink-0 flex-col items-start gap-3 lg:items-end lg:self-start lg:pl-6">
                    <div className="text-left sm:text-right">
                      <div
                        className="text-3xl font-semibold tracking-tight text-foreground sm:text-4xl"
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
                              className={cn(
                                boxedTextActionButtonBaseClass,
                                boxedActionButtonToneClass.search,
                              )}
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
                              className={cn(
                                boxedTextActionButtonBaseClass,
                                boxedActionButtonToneClass.search,
                              )}
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
                            className={cn(
                              boxedActionButtonBaseClass,
                              boxedActionButtonToneClass.delete,
                            )}
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
