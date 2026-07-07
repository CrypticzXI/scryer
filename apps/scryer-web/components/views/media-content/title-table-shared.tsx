import * as React from "react";
import { Loader2 } from "lucide-react";
import { Link } from "react-router-dom";
import { Button } from "@/components/ui/button";
import { IconButton } from "@/components/ui/icon-button";
import { Progress } from "@/components/ui/progress";
import { TableCell, TableRow } from "@/components/ui/table";
import type { ViewId, Translate } from "@/components/root/types";
import type { TitleRecord } from "@/lib/types";
import type { UiDateTimeFormat } from "@/lib/types/settings";
import type { ParsedQualityProfile } from "@/lib/types/quality-profiles";
import { formatUiDate } from "@/lib/utils/date-format";
import {
  compactRatingNumber,
  externalRatingLabelForAliases,
} from "@/lib/utils/title-ratings";
import { cn } from "@/lib/utils";
import type { BoxedActionButtonTone } from "@/lib/utils/action-button-styles";

const QP_TAG_PREFIX = "scryer:quality-profile:";

export type TitleTableSortKey =
  | "name"
  | "library"
  | "monitored"
  | "quality"
  | "episodes"
  | "status"
  | "size"
  | "added"
  | "year"
  | "runtime"
  | "root"
  | "popularity"
  | "resolution"
  | "hdr"
  | "audioCodec"
  | "ratingScryer"
  | "ratingImdb"
  | "ratingRottenTomatoes"
  | "ratingPopcornmeter"
  | "ratingMetacritic"
  | "ratingMetacriticUser"
  | "ratingLetterboxd"
  | "ratingTmdb"
  | "ratingTvdb"
  | "ratingTrakt"
  | "ratingMyanimelist"
  | "ratingAnilist"
  | "ratingAnidb"
  | "ratingMdblist";

export type TitleTableColumnKey =
  | "library"
  | "monitored"
  | "quality"
  | "episodes"
  | "size"
  | "added"
  | "year"
  | "runtime"
  | "status"
  | "root"
  | "popularity"
  | "resolution"
  | "hdr"
  | "audioCodec"
  | "ratingScryer"
  | "ratingImdb"
  | "ratingRottenTomatoes"
  | "ratingPopcornmeter"
  | "ratingMetacritic"
  | "ratingMetacriticUser"
  | "ratingLetterboxd"
  | "ratingTmdb"
  | "ratingTvdb"
  | "ratingTrakt"
  | "ratingMyanimelist"
  | "ratingAnilist"
  | "ratingAnidb"
  | "ratingMdblist"
  | "actions";

export type TitleTableVisibleColumns = Record<TitleTableColumnKey, boolean>;

export const DEFAULT_TITLE_TABLE_VISIBLE_COLUMNS: TitleTableVisibleColumns = {
  library: true,
  monitored: true,
  quality: true,
  episodes: true,
  size: true,
  added: false,
  year: false,
  runtime: false,
  status: false,
  root: false,
  popularity: false,
  resolution: false,
  hdr: false,
  audioCodec: false,
  ratingScryer: false,
  ratingImdb: false,
  ratingRottenTomatoes: false,
  ratingPopcornmeter: false,
  ratingMetacritic: false,
  ratingMetacriticUser: false,
  ratingLetterboxd: false,
  ratingTmdb: false,
  ratingTvdb: false,
  ratingTrakt: false,
  ratingMyanimelist: false,
  ratingAnilist: false,
  ratingAnidb: false,
  ratingMdblist: false,
  actions: true,
};

export const TITLE_TABLE_COLUMN_KEYS: readonly TitleTableColumnKey[] = [
  "library",
  "monitored",
  "quality",
  "episodes",
  "size",
  "added",
  "year",
  "runtime",
  "status",
  "root",
  "popularity",
  "resolution",
  "hdr",
  "audioCodec",
  "ratingScryer",
  "ratingImdb",
  "ratingRottenTomatoes",
  "ratingPopcornmeter",
  "ratingMetacritic",
  "ratingMetacriticUser",
  "ratingLetterboxd",
  "ratingTmdb",
  "ratingTvdb",
  "ratingTrakt",
  "ratingMyanimelist",
  "ratingAnilist",
  "ratingAnidb",
  "ratingMdblist",
  "actions",
];

export const MOVIE_TITLE_TABLE_ONLY_COLUMNS = new Set<TitleTableColumnKey>([
  "year",
  "resolution",
  "hdr",
  "audioCodec",
  "popularity",
]);

export const SERIES_TITLE_TABLE_ONLY_COLUMNS = new Set<TitleTableColumnKey>([
  "status",
]);

export const ANIME_TITLE_TABLE_RATING_COLUMNS: readonly TitleTableColumnKey[] = [
  "ratingImdb",
  "ratingTmdb",
  "ratingTvdb",
  "ratingTrakt",
  "ratingMyanimelist",
  "ratingAnilist",
  "ratingAnidb",
  "ratingMdblist",
];

export const MOVIE_TITLE_TABLE_RATING_COLUMNS: readonly TitleTableColumnKey[] = [
  "ratingImdb",
  "ratingRottenTomatoes",
  "ratingPopcornmeter",
  "ratingMetacritic",
  "ratingMetacriticUser",
  "ratingLetterboxd",
  "ratingTmdb",
  "ratingTvdb",
  "ratingTrakt",
  "ratingMdblist",
];

export const SHARED_TITLE_TABLE_RATING_COLUMNS: readonly TitleTableColumnKey[] =
  MOVIE_TITLE_TABLE_RATING_COLUMNS.filter(
    (key) => key !== "ratingLetterboxd",
  );

export function titleTableSupportedRatingColumnsForView(
  view: ViewId,
): readonly TitleTableColumnKey[] {
  if (view === "anime") {
    return ANIME_TITLE_TABLE_RATING_COLUMNS;
  }
  return view === "movies"
    ? MOVIE_TITLE_TABLE_RATING_COLUMNS
    : SHARED_TITLE_TABLE_RATING_COLUMNS;
}

export function isTitleTableColumnSupportedForView(
  key: TitleTableColumnKey,
  view: ViewId,
): boolean {
  if (isTitleTableRatingColumn(key)) {
    return titleTableSupportedRatingColumnsForView(view).includes(key);
  }
  if (view !== "movies" && MOVIE_TITLE_TABLE_ONLY_COLUMNS.has(key)) {
    return false;
  }
  if (view === "movies" && SERIES_TITLE_TABLE_ONLY_COLUMNS.has(key)) {
    return false;
  }
  if (key === "episodes" && view === "movies") {
    return false;
  }
  return true;
}

export const TITLE_TABLE_HEADER_ROW_CLASS =
  "sticky top-0 z-10 h-11 border-b border-[var(--scry-border)] bg-[var(--scry-surfD)]";

export const TITLE_TABLE_HEADER_CELL_CLASS =
  "text-[11px] font-bold uppercase tracking-[0.05em] text-[var(--scry-faint2)]";

export const TITLE_TABLE_ROW_CLASS =
  "border-b border-[var(--scry-line2)] transition-colors hover:bg-[var(--scry-rowHover)]";

export const TITLE_TABLE_ACTION_BUTTON_CLASS = "h-9 w-9 rounded-[8px]";

export const COMPACT_TITLE_TABLE_ACTION_BUTTON_CLASS = "size-7 rounded-[7px]";

export const TITLE_TABLE_INTERACTIVE_PANEL_ESTIMATED_HEIGHT = 448;

export const TITLE_TABLE_INTERACTIVE_PANEL_BODY_CLASS =
  "max-h-[min(28rem,calc(100vh-14rem))] overflow-y-auto overscroll-contain px-4 py-3";

export type TitleTableSortDirection = "asc" | "desc";

type TitleTableVirtualizer = {
  getTotalSize: () => number;
  measure: () => void;
  scrollOffset?: number | null;
  scrollToOffset: (offset: number) => void;
};

export function useTitleTableVirtualizerRebuild<TElement extends HTMLElement>({
  itemCount,
  loading,
  rebuildKey,
  scrollRef,
  titleVirtualizer,
}: {
  itemCount: number;
  loading: boolean;
  rebuildKey?: React.Key;
  scrollRef: React.RefObject<TElement | null>;
  titleVirtualizer: TitleTableVirtualizer;
}) {
  const getMaxScrollTop = React.useCallback(
    (element: HTMLElement) => {
      const totalSize = titleVirtualizer.getTotalSize();
      if (totalSize <= 0 || element.clientHeight <= 0) {
        return Math.max(0, element.scrollHeight - element.clientHeight);
      }

      return Math.max(totalSize - element.clientHeight, 0);
    },
    [titleVirtualizer],
  );

  React.useLayoutEffect(() => {
    if (typeof window === "undefined") {
      return;
    }

    let secondFrameId: number | undefined;
    const rebuildVirtualTable = () => {
      titleVirtualizer.measure();

      const element = scrollRef.current;
      if (!element) {
        return;
      }

      const maxScrollTop = getMaxScrollTop(element);
      const virtualOffset = titleVirtualizer.scrollOffset;
      const currentOffset =
        typeof virtualOffset === "number" ? virtualOffset : element.scrollTop;
      if (currentOffset > maxScrollTop || element.scrollTop > maxScrollTop) {
        titleVirtualizer.scrollToOffset(maxScrollTop);
      }
    };
    rebuildVirtualTable();
    const firstFrameId = window.requestAnimationFrame(() => {
      rebuildVirtualTable();
      secondFrameId = window.requestAnimationFrame(rebuildVirtualTable);
    });
    const timeoutId = window.setTimeout(rebuildVirtualTable, 80);
    const resizeObserver =
      typeof ResizeObserver !== "undefined"
        ? new ResizeObserver(rebuildVirtualTable)
        : null;

    const element = scrollRef.current;
    if (element) {
      resizeObserver?.observe(element);
    }

    return () => {
      window.cancelAnimationFrame(firstFrameId);
      if (secondFrameId !== undefined) {
        window.cancelAnimationFrame(secondFrameId);
      }
      window.clearTimeout(timeoutId);
      resizeObserver?.disconnect();
    };
  }, [
    getMaxScrollTop,
    itemCount,
    loading,
    rebuildKey,
    scrollRef,
    titleVirtualizer,
  ]);

  return getMaxScrollTop;
}

export function resolveOverviewTargetView(view: string): ViewId {
  if (view === "movies") {
    return "movies";
  }
  if (view === "anime") {
    return "anime";
  }
  return "series";
}

export function formatProfileLabel(
  value: string | null | undefined,
): string | null {
  const trimmed = value?.trim();
  if (!trimmed) {
    return null;
  }
  if (trimmed.toLowerCase() === "4k") {
    return "4K";
  }
  if (/^\d{3,4}p$/i.test(trimmed)) {
    return trimmed.toUpperCase();
  }
  return trimmed;
}

export function bytesToReadable(raw: number | null | undefined) {
  if (!raw || raw <= 0) {
    return "—";
  }
  if (raw > 1024 * 1024 * 1024) {
    return `${(raw / (1024 * 1024 * 1024)).toFixed(2)} GB`;
  }
  if (raw > 1024 * 1024) {
    return `${(raw / (1024 * 1024)).toFixed(2)} MB`;
  }
  if (raw > 1024) {
    return `${(raw / 1024).toFixed(2)} KB`;
  }
  return `${raw} B`;
}

export function formatRuntimeMinutes(
  minutes: number | null | undefined,
): string {
  if (!minutes || minutes <= 0) {
    return "—";
  }
  const hours = Math.floor(minutes / 60);
  const remainingMinutes = minutes % 60;
  if (hours <= 0) {
    return `${remainingMinutes}m`;
  }
  if (remainingMinutes <= 0) {
    return `${hours}h`;
  }
  return `${hours}h ${remainingMinutes}m`;
}

export function formatCatalogPopularity(value: number | null | undefined) {
  if (typeof value !== "number" || !Number.isFinite(value)) {
    return "—";
  }
  if (Math.abs(value) >= 100) {
    return Math.round(value).toString();
  }
  return compactRatingNumber(value);
}

export function formatResolutionLabel(value: string | null | undefined) {
  const trimmed = value?.trim();
  if (!trimmed) {
    return "—";
  }
  const normalized = trimmed.toLowerCase();
  if (normalized === "4k" || normalized === "uhd") {
    return "2160p";
  }
  if (/^\d{3,4}p$/.test(normalized)) {
    return normalized;
  }
  return trimmed.toUpperCase();
}

export function formatHdrLabel(value: string | null | undefined) {
  const trimmed = value?.trim();
  if (!trimmed) {
    return "—";
  }
  const normalized = trimmed.toLowerCase().replace(/[\s_.-]+/g, "");
  if (normalized === "sdr") {
    return "SDR";
  }
  if (normalized.includes("dolbyvision") || normalized === "dv") {
    return "Dolby Vision";
  }
  if (normalized.includes("hdr10plus") || normalized.includes("hdr10+")) {
    return "HDR10+";
  }
  if (normalized.includes("hdr10")) {
    return "HDR10";
  }
  if (normalized.includes("hdr")) {
    return "HDR";
  }
  return trimmed;
}

export function formatAudioCodecLabel(value: string | null | undefined) {
  const trimmed = value?.trim();
  if (!trimmed) {
    return "—";
  }
  const normalized = trimmed.toLowerCase().replace(/[\s_.-]+/g, "");
  switch (normalized) {
    case "truehd":
      return "TrueHD";
    case "dtshdma":
    case "dtshdmasteraudio":
      return "DTS-HD MA";
    case "dtshd":
      return "DTS-HD";
    case "dtsx":
      return "DTS:X";
    case "eac3":
    case "eac3joc":
    case "ddplus":
      return "E-AC-3";
    case "ac3":
      return "AC-3";
    case "aac":
      return "AAC";
    case "flac":
      return "FLAC";
    case "opus":
      return "Opus";
    case "mp3":
      return "MP3";
    case "pcm":
      return "PCM";
    default:
      return trimmed;
  }
}

type RatingColumnDefinition = {
  label: string;
  aliases: readonly string[];
};

const RATING_COLUMN_DEFINITIONS: Partial<
  Record<TitleTableColumnKey, RatingColumnDefinition>
> = {
  ratingImdb: { label: "IMDb", aliases: ["imdb"] },
  ratingRottenTomatoes: {
    label: "Rotten Tomatoes",
    aliases: ["rottentomatoes", "tomatoes"],
  },
  ratingPopcornmeter: {
    label: "Popcornmeter",
    aliases: ["popcornmeter", "popcorn", "audience"],
  },
  ratingMetacritic: { label: "Metacritic", aliases: ["metacritic"] },
  ratingMetacriticUser: {
    label: "Metacritic User",
    aliases: ["metacriticuser", "mcuser"],
  },
  ratingLetterboxd: { label: "Letterboxd", aliases: ["letterboxd"] },
  ratingTmdb: { label: "TMDB", aliases: ["tmdb"] },
  ratingTvdb: { label: "TVDB", aliases: ["tvdb", "thetvdb"] },
  ratingTrakt: { label: "Trakt", aliases: ["trakt"] },
  ratingMyanimelist: {
    label: "MyAnimeList",
    aliases: ["myanimelist", "mal", "myanimelistnet"],
  },
  ratingAnilist: { label: "AniList", aliases: ["anilist"] },
  ratingAnidb: { label: "AniDB", aliases: ["anidb"] },
  ratingMdblist: { label: "MDBList", aliases: ["mdblist"] },
};

export function isTitleTableRatingColumn(key: TitleTableColumnKey): boolean {
  return key.startsWith("rating");
}

export function titleTableRatingColumnWidthRem(
  key: TitleTableColumnKey,
): number {
  switch (key) {
    case "ratingRottenTomatoes":
      return 11.5;
    case "ratingPopcornmeter":
      return 10.75;
    case "ratingMetacritic":
      return 9.25;
    case "ratingMetacriticUser":
      return 12.25;
    case "ratingLetterboxd":
      return 9.25;
    case "ratingMyanimelist":
      return 10;
    case "ratingAnilist":
    case "ratingMdblist":
      return 7.25;
    case "ratingImdb":
    case "ratingTmdb":
    case "ratingTvdb":
    case "ratingTrakt":
    case "ratingAnidb":
    case "ratingScryer":
    default:
      return 6.25;
  }
}

export function titleTableRatingColumnLabel(
  key: TitleTableColumnKey,
): string {
  if (key === "ratingScryer") {
    return "Scryer Rating";
  }
  return RATING_COLUMN_DEFINITIONS[key]?.label ?? key;
}

export function titleTableRatingColumnValue(
  item: TitleRecord,
  key: TitleTableColumnKey,
): string {
  const ratings = item.ratings;
  if (!ratings) {
    return "—";
  }
  if (key === "ratingScryer") {
    return typeof ratings.rating === "number" && Number.isFinite(ratings.rating)
      ? compactRatingNumber(ratings.rating)
      : "—";
  }
  const definition = RATING_COLUMN_DEFINITIONS[key];
  if (!definition) {
    return "—";
  }
  return (
    externalRatingLabelForAliases(ratings.externalRatings, definition.aliases) ??
    "—"
  );
}

export function formatEpisodeProgress(
  ownedEpisodes: number | null | undefined,
  targetEpisodes: number | null | undefined,
) {
  if (typeof targetEpisodes !== "number") {
    return "—";
  }

  if (targetEpisodes <= 0) {
    return "0 / 0";
  }

  const owned =
    typeof ownedEpisodes === "number" && ownedEpisodes >= 0 ? ownedEpisodes : 0;
  return `${owned} / ${targetEpisodes}`;
}

export type EpisodeProgressPresentation = {
  text: string;
  percent: number;
  indicatorClassName: string;
  assistiveText: string;
};

function normalizeEpisodeProgressCounts(item: TitleRecord) {
  const monitored =
    typeof item.episodesMonitored === "number"
      ? Math.max(0, item.episodesMonitored)
      : null;
  const total =
    typeof item.episodesTotal === "number" && item.episodesTotal >= 0
      ? item.episodesTotal
      : null;

  if (monitored == null && total == null) {
    return null;
  }

  const owned =
    typeof item.episodesOwned === "number" && item.episodesOwned > 0
      ? item.episodesOwned
      : 0;
  const target = total ?? monitored ?? 0;

  return {
    monitored: monitored ?? 0,
    owned,
    total,
    target,
    displayedOwned: target > 0 ? Math.min(owned, target) : 0,
  };
}

function episodeProgressIndicatorClass(item: TitleRecord, percent: number) {
  if (percent >= 100) {
    return item.contentStatus?.trim().toLowerCase() === "ended"
      ? "bg-[var(--scry-success-solid)]"
      : "bg-[var(--scry-info-solid)]";
  }

  const missingMonitoredEpisodes =
    typeof item.episodesMonitored === "number" &&
    item.episodesMonitored > 0 &&
    (item.episodesOwned ?? 0) < item.episodesMonitored;

  return missingMonitoredEpisodes
    ? "bg-[var(--scry-danger-solid)]"
    : "bg-slate-500 dark:bg-slate-500";
}

function collectionEpisodeProgressIndicatorClass(
  missingMonitoredEpisodes: boolean,
  percent: number,
) {
  if (percent >= 100) {
    return "bg-[var(--scry-success-solid)]";
  }

  return missingMonitoredEpisodes
    ? "bg-[var(--scry-danger-solid)]"
    : "bg-slate-500 dark:bg-slate-500";
}

function buildEpisodeProgressAssistiveText(item: TitleRecord, t: Translate) {
  const counts = normalizeEpisodeProgressCounts(item);
  if (!counts) {
    return null;
  }

  return counts.total == null
    ? t("title.table.episodeProgressTooltip", {
        owned: counts.displayedOwned,
        total: counts.target,
      })
    : t("title.table.episodeProgressTooltipWithTotal", {
        owned: counts.displayedOwned,
        total: counts.total,
        monitored: counts.monitored,
      });
}

export function getEpisodeProgressPresentation(
  item: TitleRecord,
  t: Translate,
): EpisodeProgressPresentation | null {
  const counts = normalizeEpisodeProgressCounts(item);
  if (!counts) {
    return null;
  }

  const text = formatEpisodeProgress(counts.displayedOwned, counts.target);
  const percent =
    counts.target > 0 ? (counts.displayedOwned / counts.target) * 100 : 0;
  const assistiveText = buildEpisodeProgressAssistiveText(item, t) ?? text;

  return {
    text,
    percent,
    indicatorClassName: episodeProgressIndicatorClass(item, percent),
    assistiveText,
  };
}

export function getCollectionEpisodeProgressPresentation({
  ownedEpisodes,
  totalEpisodes,
  monitoredEpisodes,
  t,
}: {
  ownedEpisodes: number | null | undefined;
  totalEpisodes: number | null | undefined;
  monitoredEpisodes?: number | null | undefined;
  t: Translate;
}): EpisodeProgressPresentation | null {
  if (typeof totalEpisodes !== "number") {
    return null;
  }

  const target = Math.max(0, totalEpisodes);
  const owned =
    typeof ownedEpisodes === "number" && ownedEpisodes >= 0 ? ownedEpisodes : 0;
  const displayedOwned = target > 0 ? Math.min(owned, target) : 0;
  const text = formatEpisodeProgress(displayedOwned, target);
  const percent = target > 0 ? (displayedOwned / target) * 100 : 0;
  const assistiveText = t("title.table.episodeProgressTooltipWithTotal", {
    owned: displayedOwned,
    total: target,
    monitored:
      typeof monitoredEpisodes === "number" && monitoredEpisodes >= 0
        ? monitoredEpisodes
        : 0,
  });
  const missingMonitoredEpisodes =
    typeof monitoredEpisodes === "number" &&
    monitoredEpisodes > 0 &&
    owned < monitoredEpisodes;

  return {
    text,
    percent,
    indicatorClassName: collectionEpisodeProgressIndicatorClass(
      missingMonitoredEpisodes,
      percent,
    ),
    assistiveText,
  };
}

export function EpisodeProgressBar({
  progress,
  compact = false,
  className,
}: {
  progress: EpisodeProgressPresentation | null;
  compact?: boolean;
  className?: string;
}) {
  if (!progress) {
    return <span className="tabular-nums text-muted-foreground">—</span>;
  }

  return (
    <div
      className={cn("relative", compact ? "w-[8rem]" : "w-[10rem]", className)}
    >
      <Progress
        value={progress.percent}
        className={cn(
          "border border-border/60 bg-muted/90 shadow-sm",
          compact ? "h-5 rounded-md" : "h-7 rounded-md",
        )}
        indicatorClassName={progress.indicatorClassName}
        aria-label={progress.assistiveText}
        aria-valuetext={progress.assistiveText}
        title={progress.assistiveText}
      />
      <span
        aria-hidden="true"
        className={cn(
          "pointer-events-none absolute inset-0 flex items-center justify-center tabular-nums font-semibold text-white [text-shadow:0_1px_1px_rgba(0,0,0,0.55)]",
          compact ? "text-[11px]" : "text-sm",
        )}
      >
        {progress.text}
      </span>
    </div>
  );
}

export function TitleEpisodeProgressBar({
  item,
  t,
  compact = false,
  className,
}: {
  item: TitleRecord;
  t: Translate;
  compact?: boolean;
  className?: string;
}) {
  const progress = getEpisodeProgressPresentation(item, t);
  return (
    <EpisodeProgressBar
      progress={progress}
      compact={compact}
      className={className}
    />
  );
}

function resolveTitleProfileName(
  item: TitleRecord,
  profiles: ParsedQualityProfile[],
  fallback: string | null,
): string | null {
  const tag = item.tags?.find((tagValue) => tagValue.startsWith(QP_TAG_PREFIX));
  if (tag) {
    const id = tag.slice(QP_TAG_PREFIX.length);
    const match = profiles.find((profile) => profile.id === id);
    if (match) {
      return match.name;
    }
    return formatProfileLabel(id);
  }

  return formatProfileLabel(fallback) ?? fallback;
}

export function resolveDisplayedQualityLabel(
  item: TitleRecord,
  profiles: ParsedQualityProfile[],
  fallback: string | null,
  unknownLabel: string,
) {
  return resolveTitleProfileName(item, profiles, fallback) || unknownLabel;
}

export function defaultSortDirectionForTitleKey(
  key: TitleTableSortKey,
): TitleTableSortDirection {
  if (key.startsWith("rating")) {
    return "desc";
  }
  switch (key) {
    case "monitored":
    case "episodes":
    case "size":
    case "added":
    case "year":
    case "runtime":
    case "popularity":
    case "resolution":
    case "hdr":
      return "desc";
    default:
      return "asc";
  }
}

export function formatTitleDate(
  value: string | null | undefined,
  dateTimeFormat: UiDateTimeFormat,
): string | null {
  if (!value) {
    return null;
  }

  const parsed = new Date(value);
  if (Number.isNaN(parsed.getTime())) {
    return null;
  }

  return formatUiDate(value, dateTimeFormat);
}

export function StatusBadge({
  status,
  t,
}: {
  status?: string | null;
  t: Translate;
}) {
  const normalized = status?.toLowerCase() ?? "";
  if (normalized === "ended") {
    return (
      <span className="rounded bg-zinc-700/60 px-2 py-0.5 text-xs text-zinc-300">
        {t("title.ended")}
      </span>
    );
  }
  if (normalized === "upcoming") {
    return (
      <span className="rounded bg-[var(--scry-info-bg-strong)] px-2 py-0.5 text-xs text-[var(--scry-info-text)]">
        {t("title.upcoming")}
      </span>
    );
  }
  if (normalized === "continuing") {
    return (
      <span className="rounded bg-[var(--scry-success-bg-strong)] px-2 py-0.5 text-xs text-[var(--scry-success-text)]">
        {t("title.continuing")}
      </span>
    );
  }
  return null;
}

type TitleTableActionButtonProps = Omit<
  React.ComponentProps<typeof IconButton>,
  "tone"
> & {
  label: string;
  tone: BoxedActionButtonTone;
  showTitleAttribute?: boolean;
};

export function TitleTableActionButton({
  label,
  tone,
  showTitleAttribute = true,
  className,
  children,
  ...props
}: TitleTableActionButtonProps) {
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

export function TitleTableTooltipActionButton({
  tooltip,
  tooltipClassName,
  showTitleAttribute = false,
  ...buttonProps
}: TitleTableActionButtonProps & {
  tooltip?: React.ReactNode;
  tooltipClassName?: string;
}) {
  return (
    <TitleTableActionButton
      {...buttonProps}
      tooltip={tooltip ?? buttonProps.label}
      tooltipClassName={tooltipClassName}
      showTitleAttribute={showTitleAttribute}
    />
  );
}

export function TitleTableEmptyState({
  colSpan,
  ...emptyStateProps
}: {
  colSpan: number;
} & TitleCollectionEmptyStateProps) {
  return (
    <TableRow>
      <TableCell colSpan={colSpan} className="py-8">
        <TitleCollectionEmptyState {...emptyStateProps} />
      </TableCell>
    </TableRow>
  );
}

export function TitleTableLoadingState({ colSpan }: { colSpan: number }) {
  return (
    <TableRow>
      <TableCell colSpan={colSpan} className="py-10">
        <TitleCollectionLoadingState />
      </TableCell>
    </TableRow>
  );
}

type TitleCollectionEmptyStateProps = {
  t: Translate;
  showScanAction?: boolean;
  showConfigureRootsAction?: boolean;
  configureRootsReason?: "missing" | "invalid";
  configureRootsHref?: string;
  scanLoading?: boolean;
  scanDisabled?: boolean;
  scanNotice?: string | null;
  onScan?: () => Promise<void> | void;
};

export function TitleCollectionLoadingState() {
  return (
    <div
      className="mx-auto flex max-w-sm items-center justify-center gap-3 rounded-xl border border-border/70 bg-card/60 px-5 py-5 text-center shadow-sm"
      aria-live="polite"
      aria-busy="true"
    >
      <Loader2 className="h-5 w-5 animate-spin text-primary" />
      <div className="text-left">
        <p className="text-sm font-medium text-foreground">
          Loading library...
        </p>
        <p className="text-sm text-muted-foreground">
          Checking your titles and library setup.
        </p>
      </div>
    </div>
  );
}

export function TitleCollectionEmptyState({
  t,
  showScanAction = false,
  showConfigureRootsAction = false,
  configureRootsReason = "missing",
  configureRootsHref,
  scanLoading = false,
  scanDisabled = false,
  scanNotice,
  onScan,
}: TitleCollectionEmptyStateProps) {
  return (
    <>
      {showConfigureRootsAction && configureRootsHref ? (
        <div className="mx-auto max-w-sm rounded-xl border border-border/70 bg-card/60 px-5 py-5 text-center shadow-sm">
          <p className="text-sm font-medium text-foreground">
            {configureRootsReason === "invalid"
              ? t("title.invalidRootFoldersTitle")
              : t("settings.rootFoldersEmpty")}
          </p>
          <p className="mt-1 text-sm text-muted-foreground">
            {configureRootsReason === "invalid"
              ? t("title.invalidRootFoldersHint")
              : t("title.configureRootFoldersHint")}
          </p>
          <Button asChild type="button" variant="primary" className="mt-4">
            <Link to={configureRootsHref}>
              {t("title.configureRootFoldersButton")}
            </Link>
          </Button>
        </div>
      ) : showScanAction && onScan ? (
        <div className="mx-auto max-w-sm rounded-xl border border-border/70 bg-card/60 px-5 py-5 text-center shadow-sm">
          <p className="text-sm font-medium text-foreground">
            {t("title.noManaged")}
          </p>
          <p className="mt-1 text-sm text-muted-foreground">
            {t("title.noFilesTrackedHint")}
          </p>
          <Button
            type="button"
            variant="primary"
            className="mt-4"
            onClick={() => {
              void onScan();
            }}
            disabled={scanDisabled || scanLoading}
          >
            {scanLoading ? (
              <Loader2 className="mr-1.5 h-4 w-4 animate-spin" />
            ) : null}
            {t("settings.libraryScanButton")}
          </Button>
          {scanNotice ? (
            <p className="mt-3 text-xs text-muted-foreground">{scanNotice}</p>
          ) : null}
        </div>
      ) : (
        <p className="text-muted-foreground">{t("title.noManaged")}</p>
      )}
    </>
  );
}
