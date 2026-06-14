import * as React from "react";
import { useTranslate } from "@/lib/context/translate-context";
import type {
  CollectionEpisode,
  EpisodeMediaFile,
} from "@/components/containers/series-overview-container";
import { MediaFilesOnDiskPanel } from "@/components/common/media-files-on-disk-panel";
import type { ExternalSubtitleRecord } from "@/lib/types/subtitles";
import { selectorId } from "@/lib/utils/dom-ids";

function fullyQualifiedHttpUrl(raw: string | null | undefined): string | null {
  const trimmed = raw?.trim();
  if (!trimmed) {
    return null;
  }

  try {
    const url = new URL(trimmed);
    if ((url.protocol === "http:" || url.protocol === "https:") && url.hostname.trim()) {
      return url.toString();
    }
  } catch {
    return null;
  }

  return null;
}

export function EpisodeDetailsPanel({
  episode,
  mediaFiles,
  subtitleDownloads = [],
  onRefreshSubtitles,
  onDeleteFile,
}: {
  episode: CollectionEpisode;
  mediaFiles: EpisodeMediaFile[];
  subtitleDownloads?: ExternalSubtitleRecord[];
  onRefreshSubtitles?: () => Promise<void> | void;
  onDeleteFile?: (fileId: string) => void;
}) {
  const t = useTranslate();
  const episodeImageUrl = React.useMemo(() => fullyQualifiedHttpUrl(episode.imageUrl), [episode.imageUrl]);
  const episodeImageAlt = episode.title ?? episode.episodeLabel ?? "";
  return (
    <div id={selectorId("series-overview-episode-details", episode.id)} className="space-y-3">
      {episodeImageUrl || episode.overview ? (
        <div className="flex items-start gap-4">
          {episodeImageUrl ? (
            <img
              src={episodeImageUrl}
              alt={episodeImageAlt}
              loading="lazy"
              decoding="async"
              className="w-40 shrink-0 rounded border border-border/70 bg-muted [image-rendering:smooth] sm:w-48"
            />
          ) : null}
          {episode.overview ? (
            <div className="min-w-0 flex-1">
              <p className="mb-1 text-xs font-medium text-muted-foreground">{t("episode.overview")}</p>
              <p className="text-sm leading-relaxed text-muted-foreground">{episode.overview}</p>
            </div>
          ) : null}
        </div>
      ) : null}
      <MediaFilesOnDiskPanel<EpisodeMediaFile>
        title={t("episode.fileOnDisk")}
        emptyMessage={t("episode.noFile")}
        mediaFiles={mediaFiles}
        subtitleDownloads={subtitleDownloads}
        onRefreshSubtitles={onRefreshSubtitles}
        onDeleteFile={onDeleteFile}
      />
    </div>
  );
}
