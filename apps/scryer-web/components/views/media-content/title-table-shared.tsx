import * as React from "react";
import { Loader2 } from "lucide-react";
import { Link } from "react-router-dom";
import { Button } from "@/components/ui/button";
import { Progress } from "@/components/ui/progress";
import { TableCell, TableRow } from "@/components/ui/table";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import type { ViewId, Translate } from "@/components/root/types";
import type { TitleRecord } from "@/lib/types";
import type { UiDateTimeFormat } from "@/lib/types/settings";
import type { ParsedQualityProfile } from "@/lib/types/quality-profiles";
import { formatUiDate } from "@/lib/utils/date-format";
import { cn } from "@/lib/utils";
import {
  boxedActionButtonToneClass,
  type BoxedActionButtonTone,
} from "@/lib/utils/action-button-styles";

const QP_TAG_PREFIX = "scryer:quality-profile:";

export type TitleTableSortKey =
  | "name"
  | "library"
  | "monitored"
  | "quality"
  | "episodes"
  | "status"
  | "size"
  | "added";

export type TitleTableColumnKey =
  | "library"
  | "monitored"
  | "quality"
  | "episodes"
  | "size"
  | "added"
  | "actions";

export type TitleTableVisibleColumns = Record<TitleTableColumnKey, boolean>;

export const DEFAULT_TITLE_TABLE_VISIBLE_COLUMNS: TitleTableVisibleColumns = {
  library: true,
  monitored: true,
  quality: true,
  episodes: true,
  size: true,
  added: false,
  actions: true,
};

export const TITLE_TABLE_COLUMN_KEYS: readonly TitleTableColumnKey[] = [
  "library",
  "monitored",
  "quality",
  "episodes",
  "size",
  "added",
  "actions",
];

export const TITLE_TABLE_HEADER_ROW_CLASS =
  "sticky top-0 z-10 h-11 border-b border-[var(--scry-border)] bg-[var(--scry-surfD)]";

export const TITLE_TABLE_HEADER_CELL_CLASS =
  "text-[11px] font-bold uppercase tracking-[0.05em] text-[var(--scry-faint2)]";

export const TITLE_TABLE_ROW_CLASS =
  "border-b border-[var(--scry-line2)] transition-colors hover:bg-[var(--scry-hover)]";

export const TITLE_TABLE_ACTION_BUTTON_CLASS = "h-9 w-9 rounded-[8px]";

export const COMPACT_TITLE_TABLE_ACTION_BUTTON_CLASS = "size-7 rounded-[7px]";

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
      ? "bg-emerald-600 dark:bg-emerald-600"
      : "bg-sky-600 dark:bg-sky-500";
  }

  return item.monitored
    ? "bg-rose-500 dark:bg-rose-400"
    : "bg-slate-500 dark:bg-slate-500";
}

function collectionEpisodeProgressIndicatorClass(
  monitored: boolean,
  percent: number,
) {
  if (percent >= 100) {
    return "bg-emerald-600 dark:bg-emerald-600";
  }

  return monitored
    ? "bg-rose-500 dark:bg-rose-400"
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
  monitored,
  t,
}: {
  ownedEpisodes: number | null | undefined;
  totalEpisodes: number | null | undefined;
  monitoredEpisodes?: number | null | undefined;
  monitored: boolean;
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

  return {
    text,
    percent,
    indicatorClassName: collectionEpisodeProgressIndicatorClass(
      monitored,
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
  switch (key) {
    case "monitored":
    case "episodes":
    case "size":
    case "added":
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
      <span className="rounded bg-blue-900/50 px-2 py-0.5 text-xs text-blue-300">
        {t("title.upcoming")}
      </span>
    );
  }
  if (normalized === "continuing") {
    return (
      <span className="rounded bg-emerald-900/50 px-2 py-0.5 text-xs text-emerald-300">
        {t("title.continuing")}
      </span>
    );
  }
  return null;
}

type TitleTableActionButtonProps = React.ComponentProps<typeof Button> & {
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
    <Button
      type="button"
      size="icon-sm"
      variant="secondary"
      title={showTitleAttribute ? label : undefined}
      aria-label={label}
      className={cn(
        "h-9 w-9 border shadow-none transition-colors hover:translate-y-0 hover:shadow-none",
        boxedActionButtonToneClass[tone],
        className,
      )}
      {...props}
    >
      {children}
    </Button>
  );
}

export function TitleTableLazyTooltipActionButton({
  tooltip,
  tooltipClassName,
  showTitleAttribute = false,
  ...buttonProps
}: TitleTableActionButtonProps & {
  tooltip: React.ReactNode;
  tooltipClassName?: string;
}) {
  const [active, setActive] = React.useState(false);
  const trigger = (
    <span
      className="inline-flex"
      onPointerEnter={() => setActive(true)}
      onPointerLeave={() => setActive(false)}
      onFocus={() => setActive(true)}
      onBlur={() => setActive(false)}
    >
      <TitleTableActionButton
        {...buttonProps}
        showTitleAttribute={showTitleAttribute}
      />
    </span>
  );

  if (!active) {
    return trigger;
  }

  return (
    <TooltipProvider>
      <Tooltip open={active} onOpenChange={setActive}>
        <TooltipTrigger asChild>{trigger}</TooltipTrigger>
        <TooltipContent
          side="top"
          sideOffset={8}
          className={cn(
            "max-w-[18rem] whitespace-normal break-words text-left text-sm leading-snug",
            tooltipClassName,
          )}
        >
          {tooltip}
        </TooltipContent>
      </Tooltip>
    </TooltipProvider>
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
