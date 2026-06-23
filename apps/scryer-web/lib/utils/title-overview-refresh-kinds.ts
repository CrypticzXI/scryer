import type { ActivityKind } from "@/lib/types/activity";

const TITLE_OVERVIEW_REFRESH_KIND_VALUES = [
  "movie_downloaded",
  "series_episode_imported",
  "file_analyzed",
  "file_upgraded",
  "subtitle_downloaded",
  "import_rejected",
] as const satisfies readonly ActivityKind[];

export const TITLE_OVERVIEW_REFRESH_KINDS: ReadonlySet<string> = new Set(
  TITLE_OVERVIEW_REFRESH_KIND_VALUES,
);
