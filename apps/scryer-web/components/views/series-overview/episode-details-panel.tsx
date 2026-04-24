import * as React from "react";
import { HardDrive, Search, Trash2 } from "lucide-react";
import { useTranslate } from "@/lib/context/translate-context";
import type {
  CollectionEpisode,
  EpisodeMediaFile,
  InterstitialMovieMetadata,
} from "@/components/containers/series-overview-container";
import { MediaInfoBadges } from "@/components/common/media-info-badges";
import { InterstitialMoviePanel } from "./interstitial-movie-panel";
import { deriveMediaFileQualityLabel, formatDate, formatFileSize } from "./helpers";
import type { SubtitleDownloadRecord } from "@/lib/types/subtitles";
import { ExternalSubtitleSection } from "@/components/common/external-subtitle-section";
import { SubtitleSearchModal } from "@/components/views/subtitle-search-modal";
import { Button } from "@/components/ui/button";
import { boxedActionButtonBaseClass, boxedActionButtonToneClass } from "@/lib/utils/action-button-styles";

export function EpisodeDetailsPanel({
  episode,
  mediaFiles,
  subtitleDownloads = [],
  onRefreshSubtitles,
  linkedMovie,
  onDeleteFile,
}: {
  episode: CollectionEpisode;
  mediaFiles: EpisodeMediaFile[];
  subtitleDownloads?: SubtitleDownloadRecord[];
  onRefreshSubtitles?: () => Promise<void> | void;
  linkedMovie?: InterstitialMovieMetadata | null;
  onDeleteFile?: (fileId: string) => void;
}) {
  const t = useTranslate();
  const [subtitleSearchTarget, setSubtitleSearchTarget] = React.useState<{
    mediaFileId: string;
    filePath: string;
  } | null>(null);
  const subtitleDownloadsByMediaFile = subtitleDownloads.reduce<Record<string, SubtitleDownloadRecord[]>>(
    (grouped, download) => {
      (grouped[download.mediaFileId] ??= []).push(download);
      return grouped;
    },
    {},
  );
  return (
    <div className="space-y-3">
      {episode.overview ? (
        <div>
          <p className="mb-1 text-xs font-medium text-muted-foreground">{t("episode.overview")}</p>
          <p className="text-sm leading-relaxed text-muted-foreground">{episode.overview}</p>
        </div>
      ) : null}
      {linkedMovie ? (
        <div>
          <p className="mb-2 text-xs font-medium text-muted-foreground">{t("title.movieDetails")}</p>
          <div className="rounded-xl border border-border/70 bg-card/40 p-3">
            <InterstitialMoviePanel
              movie={linkedMovie}
              hasFile={mediaFiles.length > 0}
              monitored={episode.monitored}
            />
          </div>
        </div>
      ) : null}
      <div>
        <p className="mb-1 text-xs font-medium text-muted-foreground">{t("episode.fileOnDisk")}</p>
        {mediaFiles.length > 0 ? (
          <div className="space-y-2">
            {mediaFiles.map((file) => {
              const qualityLabel = deriveMediaFileQualityLabel(file);
              const downloads = subtitleDownloadsByMediaFile[file.id] ?? [];
              return (
                <div key={file.id} className="space-y-1.5 rounded bg-card/60 px-3 py-2 text-sm">
                  <div className="flex flex-wrap items-center gap-3">
                    <HardDrive className="h-3.5 w-3.5 shrink-0 text-muted-foreground/60" />
                    <span className="min-w-0 break-all font-mono text-xs text-muted-foreground">{file.filePath}</span>
                    {qualityLabel ? (
                      <span className="rounded border border-emerald-500/40 dark:border-emerald-500/30 bg-emerald-500/20 dark:bg-emerald-500/15 px-1.5 py-0.5 text-[10px] font-medium text-emerald-700 dark:text-emerald-300">
                        {qualityLabel}
                      </span>
                    ) : null}
                    <span className="text-xs text-muted-foreground/60">{formatFileSize(Number(file.sizeBytes))}</span>
                    <span className="text-xs text-muted-foreground/60">{formatDate(file.createdAt)}</span>
                    {file.acquisitionScore != null ? (
                      <span className="text-xs text-muted-foreground/60" title={file.scoringLog ?? undefined}>
                        {t("mediaFile.score", { score: file.acquisitionScore })}
                      </span>
                    ) : null}
                    <Button
                      type="button"
                      size="sm"
                      variant="secondary"
                      className={`ml-auto h-8 border px-3 ${boxedActionButtonToneClass.search}`}
                      onClick={() =>
                        setSubtitleSearchTarget({
                          mediaFileId: file.id,
                          filePath: file.filePath,
                        })
                      }
                    >
                      <Search className="mr-1.5 h-3.5 w-3.5" />
                      {t("subtitle.search")}
                    </Button>
                    {onDeleteFile ? (
                      <Button
                        type="button"
                        size="icon-sm"
                        variant="secondary"
                        onClick={() => onDeleteFile(file.id)}
                        className={`${boxedActionButtonBaseClass} ${boxedActionButtonToneClass.delete}`}
                        title={t("mediaFile.delete")}
                        aria-label={t("mediaFile.delete")}
                      >
                        <Trash2 className="h-4 w-4" />
                      </Button>
                    ) : null}
                  </div>
                  <MediaInfoBadges file={file} />
                  <ExternalSubtitleSection downloads={downloads} />
                </div>
              );
            })}
          </div>
        ) : (
          <p className="text-sm italic text-muted-foreground/60">{t("episode.noFile")}</p>
        )}
      </div>
      {subtitleSearchTarget ? (
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
            void onRefreshSubtitles?.();
          }}
        />
      ) : null}
    </div>
  );
}
