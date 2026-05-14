import * as React from "react";
import { Loader2 } from "lucide-react";
import { Link } from "react-router-dom";
import { Button } from "@/components/ui/button";
import { Progress } from "@/components/ui/progress";
import { TableCell, TableRow } from "@/components/ui/table";
import type { ViewId, Translate } from "@/components/root/types";
import type { TitleRecord } from "@/lib/types";
import type { ParsedQualityProfile } from "@/lib/types/quality-profiles";
import { cn } from "@/lib/utils";
import {
  boxedActionButtonBaseClass,
  boxedActionButtonToneClass,
  type BoxedActionButtonTone,
} from "@/lib/utils/action-button-styles";

const QP_TAG_PREFIX = "scryer:quality-profile:";

export type TitleTableSortKey =
  | "name"
  | "monitored"
  | "quality"
  | "episodes"
  | "status"
  | "size";
export type TitleTableSortDirection = "asc" | "desc";

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
  const percent = counts.target > 0 ? (counts.displayedOwned / counts.target) * 100 : 0;
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

function normalizeTitleForUiSort(value: string) {
  const trimmed = value.trim();
  if (!trimmed) {
    return trimmed;
  }
  const withoutArticle = trimmed.replace(/^(a|an|the)\s+/i, "");
  return withoutArticle.trim() || trimmed;
}

function compareText(left: string, right: string) {
  return left.localeCompare(right, undefined, {
    sensitivity: "base",
    numeric: true,
  });
}

function compareTitleText(left: string, right: string) {
  const normalizedLeft = normalizeTitleForUiSort(left);
  const normalizedRight = normalizeTitleForUiSort(right);
  const normalizedDelta = compareText(normalizedLeft, normalizedRight);
  if (normalizedDelta !== 0) {
    return normalizedDelta;
  }
  return compareText(left, right);
}

function compareMaybeText(
  left: string | null | undefined,
  right: string | null | undefined,
) {
  const normalizedLeft = left?.trim() ?? "";
  const normalizedRight = right?.trim() ?? "";
  if (!normalizedLeft && !normalizedRight) {
    return 0;
  }
  if (!normalizedLeft) {
    return 1;
  }
  if (!normalizedRight) {
    return -1;
  }
  return compareText(normalizedLeft, normalizedRight);
}

function compareBooleans(left: boolean, right: boolean) {
  return Number(left) - Number(right);
}

function compareNumbers(
  left: number | null | undefined,
  right: number | null | undefined,
) {
  const normalizedLeft = left ?? Number.NEGATIVE_INFINITY;
  const normalizedRight = right ?? Number.NEGATIVE_INFINITY;
  return normalizedLeft - normalizedRight;
}

function compareEpisodeProgressValues(left: TitleRecord, right: TitleRecord) {
  const leftOwned = left.episodesOwned ?? 0;
  const rightOwned = right.episodesOwned ?? 0;
  const leftTarget = left.episodesTotal ?? left.episodesMonitored ?? 0;
  const rightTarget = right.episodesTotal ?? right.episodesMonitored ?? 0;
  const leftRatio =
    leftTarget > 0 ? leftOwned / leftTarget : Number.NEGATIVE_INFINITY;
  const rightRatio =
    rightTarget > 0 ? rightOwned / rightTarget : Number.NEGATIVE_INFINITY;

  const ratioDelta = leftRatio - rightRatio;
  if (ratioDelta !== 0) {
    return ratioDelta;
  }

  const ownedDelta = leftOwned - rightOwned;
  if (ownedDelta !== 0) {
    return ownedDelta;
  }

  return leftTarget - rightTarget;
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
    <div className={cn("relative", compact ? "w-[8rem]" : "w-[10rem]", className)}>
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
  return (
    resolveTitleProfileName(item, profiles, fallback) || unknownLabel
  );
}

export function sortTitlesForTable({
  titles,
  sortKey,
  sortDirection,
  qualityProfiles,
  resolvedProfileName,
  qualityProfilesLoading,
  t,
}: {
  titles: TitleRecord[];
  sortKey: TitleTableSortKey;
  sortDirection: TitleTableSortDirection;
  qualityProfiles: ParsedQualityProfile[];
  resolvedProfileName: string | null;
  qualityProfilesLoading: boolean;
  t: Translate;
}): TitleRecord[] {
  const factor = sortDirection === "asc" ? 1 : -1;
  const getStatusSortLabel = (item: TitleRecord) => {
    const normalized = item.contentStatus?.toLowerCase() ?? "";
    switch (normalized) {
      case "ended":
        return t("title.ended");
      case "upcoming":
        return t("title.upcoming");
      case "continuing":
        return t("title.continuing");
      default:
        return "";
    }
  };

  return [...titles].sort((left, right) => {
    const delta = (() => {
      switch (sortKey) {
        case "name":
          return compareTitleText(left.name, right.name);
        case "monitored":
          return compareBooleans(left.monitored, right.monitored);
        case "quality":
          if (qualityProfilesLoading) {
            return 0;
          }
          return compareMaybeText(
            resolveDisplayedQualityLabel(
              left,
              qualityProfiles,
              resolvedProfileName,
              t("label.unknown"),
            ),
            resolveDisplayedQualityLabel(
              right,
              qualityProfiles,
              resolvedProfileName,
              t("label.unknown"),
            ),
          );
        case "episodes":
          return compareEpisodeProgressValues(left, right);
        case "status":
          return compareMaybeText(
            getStatusSortLabel(left),
            getStatusSortLabel(right),
          );
        case "size":
          return compareNumbers(left.sizeBytes, right.sizeBytes);
        default:
          return 0;
      }
    })();

    if (delta !== 0) {
      return delta * factor;
    }

    return compareTitleText(left.name, right.name);
  });
}

export function defaultSortDirectionForTitleKey(
  key: TitleTableSortKey,
): TitleTableSortDirection {
  switch (key) {
    case "monitored":
    case "episodes":
    case "size":
      return "desc";
    default:
      return "asc";
  }
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

export function TitleTableActionButton({
  label,
  tone,
  showTitleAttribute = true,
  className,
  children,
  ...props
}: React.ComponentProps<typeof Button> & {
  label: string;
  tone: BoxedActionButtonTone;
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

export function TitleTableLoadingState({
  colSpan,
}: {
  colSpan: number;
}) {
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
        <p className="text-sm font-medium text-foreground">Loading library...</p>
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
            {t("settings.rootFoldersEmpty")}
          </p>
          <p className="mt-1 text-sm text-muted-foreground">
            {t("title.configureRootFoldersHint")}
          </p>
          <Button asChild type="button" variant="primary" className="mt-4">
            <Link to={configureRootsHref}>
              {t("title.configureRootFoldersButton")}
            </Link>
          </Button>
        </div>
      ) : showScanAction && onScan ? (
        <div className="mx-auto max-w-sm rounded-xl border border-border/70 bg-card/60 px-5 py-5 text-center shadow-sm">
          <p className="text-sm font-medium text-foreground">{t("title.noManaged")}</p>
          <p className="mt-1 text-sm text-muted-foreground">{t("title.noFilesTrackedHint")}</p>
          <Button
            type="button"
            variant="primary"
            className="mt-4"
            onClick={() => {
              void onScan();
            }}
            disabled={scanDisabled || scanLoading}
          >
            {scanLoading ? <Loader2 className="mr-1.5 h-4 w-4 animate-spin" /> : null}
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
