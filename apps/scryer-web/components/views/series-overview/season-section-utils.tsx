import * as React from "react";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { useTranslate } from "@/lib/context/translate-context";
import type { Release } from "@/lib/types";
import {
  boxedActionButtonBaseClass,
  boxedActionButtonToneClass,
  type BoxedActionButtonTone,
} from "@/lib/utils/action-button-styles";
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
}: React.ComponentProps<typeof Button> & {
  label: string;
  tone: Extract<BoxedActionButtonTone, "auto" | "search">;
  showTitleAttribute?: boolean;
}) {
  return (
    <Button
      type="button"
      size="icon-sm"
      variant="secondary"
      title={showTitleAttribute ? label : undefined}
      aria-label={label}
      className={cn(
        boxedActionButtonBaseClass,
        boxedActionButtonToneClass[tone],
        className,
      )}
      {...props}
    >
      {children}
    </Button>
  );
}
