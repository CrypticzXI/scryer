import * as React from "react";
import { IconButton } from "@/components/ui/icon-button";
import { useTranslate } from "@/lib/context/translate-context";
import type { Release } from "@/lib/types";
import type { BoxedActionButtonTone } from "@/lib/utils/action-button-styles";
import type {
  EpisodeMediaFile,
  TitleReleaseBlocklistEntry,
} from "@/components/containers/series-overview-container";
import type { ExternalSubtitleRecord } from "@/lib/types/subtitles";

export type TranslateFn = ReturnType<typeof useTranslate>;

export const EMPTY_EPISODE_FILES: EpisodeMediaFile[] = [];
export const EMPTY_RELEASES: Release[] = [];
export const EMPTY_SUBTITLE_DOWNLOADS: ExternalSubtitleRecord[] = [];
export const EMPTY_BLOCKLIST_ENTRIES: TitleReleaseBlocklistEntry[] = [];

export function EpisodeTableActionButton({
  label,
  tone,
  showTitleAttribute = true,
  className,
  children,
  ...props
}: Omit<React.ComponentProps<typeof IconButton>, "tone"> & {
  label: string;
  tone: Extract<BoxedActionButtonTone, "auto" | "search">;
  showTitleAttribute?: boolean;
}) {
  return (
    <IconButton
      label={label}
      tone={tone}
      showTitleAttribute={showTitleAttribute}
      className={className}
      {...props}
    >
      {children}
    </IconButton>
  );
}
