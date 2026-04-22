import * as React from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useTranslate } from "@/lib/context/translate-context";
import { Button } from "@/components/ui/button";
import { ArrowDown, ArrowUp, Eye, EyeOff, Loader2, Search, Trash2, Zap } from "lucide-react";
import {
  HoverCard,
  HoverCardContent,
  HoverCardTrigger,
} from "@/components/ui/hover-card";
import { TitlePosterSlot } from "@/components/title-poster-slot";
import { SearchResultBuckets } from "@/components/common/release-search-results";
import {
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import type { OverviewTitleTarget, ViewId } from "@/components/root/types";
import type { Release, TitleRecord } from "@/lib/types";
import type { ParsedQualityProfile } from "@/lib/types/quality-profiles";
import { selectPosterVariantUrl } from "@/lib/utils/poster-images";
import { cn } from "@/lib/utils";
import {
  boxedActionButtonBaseClass,
  boxedActionButtonToneClass,
  type BoxedActionButtonTone,
} from "@/lib/utils/action-button-styles";

const QP_TAG_PREFIX = "scryer:quality-profile:";

function formatProfileLabel(value: string | null | undefined): string | null {
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

function bytesToReadable(raw: number | null | undefined) {
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

function formatEpisodeProgress(
  ownedEpisodes: number | null | undefined,
  monitoredEpisodes: number | null | undefined,
) {
  if (typeof monitoredEpisodes !== "number") {
    return "—";
  }

  if (monitoredEpisodes <= 0) {
    return "0 / 0";
  }

  const owned = typeof ownedEpisodes === "number" && ownedEpisodes >= 0 ? ownedEpisodes : 0;
  return `${owned} / ${monitoredEpisodes}`;
}

type SortKey = "name" | "monitored" | "quality" | "episodes" | "status" | "size";
type SortDirection = "asc" | "desc";

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

function compareMaybeText(left: string | null | undefined, right: string | null | undefined) {
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

function compareNumbers(left: number | null | undefined, right: number | null | undefined) {
  const normalizedLeft = left ?? Number.NEGATIVE_INFINITY;
  const normalizedRight = right ?? Number.NEGATIVE_INFINITY;
  return normalizedLeft - normalizedRight;
}

function compareEpisodeProgressValues(left: TitleRecord, right: TitleRecord) {
  const leftOwned = left.episodesOwned ?? 0;
  const rightOwned = right.episodesOwned ?? 0;
  const leftTarget = left.episodesMonitored ?? left.episodesTotal ?? 0;
  const rightTarget = right.episodesMonitored ?? right.episodesTotal ?? 0;
  const leftRatio = leftTarget > 0 ? leftOwned / leftTarget : Number.NEGATIVE_INFINITY;
  const rightRatio = rightTarget > 0 ? rightOwned / rightTarget : Number.NEGATIVE_INFINITY;

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

type TitleTableProps = {
  view: string;
  titles: TitleRecord[];
  titleLoading: boolean;
  resolvedProfileName: string | null;
  qualityProfiles: ParsedQualityProfile[];
  qualityProfilesLoading: boolean;
  onOpenOverview: (targetView: ViewId, overviewTarget: OverviewTitleTarget) => void;
  onDelete: (title: TitleRecord) => void;
  onAutoQueue: (title: TitleRecord) => void;
  onToggleMonitored?: (title: TitleRecord, monitored: boolean) => Promise<void> | void;
  onInteractiveSearch: (title: TitleRecord) => Promise<Release[]> | Release[];
  onQueueFromInteractive: (title: TitleRecord, release: Release) => void;
  isDeletingById: Record<string, boolean>;
  isTogglingMonitoredById?: Record<string, boolean>;
};

function resolveTitleProfileName(
  item: TitleRecord,
  profiles: ParsedQualityProfile[],
  fallback: string | null,
): string | null {
  const tag = item.tags?.find((t) => t.startsWith(QP_TAG_PREFIX));
  if (tag) {
    const id = tag.slice(QP_TAG_PREFIX.length);
    const match = profiles.find((p) => p.id === id);
    if (match) return match.name;
    return formatProfileLabel(id);
  }
  return formatProfileLabel(fallback) ?? fallback;
}

function resolveDisplayedQualityLabel(
  item: TitleRecord,
  profiles: ParsedQualityProfile[],
  fallback: string | null,
  unknownLabel: string,
) {
  return resolveTitleProfileName(item, profiles, fallback) || unknownLabel;
}

function StatusBadge({ status, t }: { status?: string | null; t: (key: string) => string }) {
  const normalized = status?.toLowerCase() ?? "";
  if (normalized === "ended") {
    return <span className="rounded bg-zinc-700/60 px-2 py-0.5 text-xs text-zinc-300">{t("title.ended")}</span>;
  }
  if (normalized === "upcoming") {
    return <span className="rounded bg-blue-900/50 px-2 py-0.5 text-xs text-blue-300">{t("title.upcoming")}</span>;
  }
  if (normalized === "continuing") {
    return <span className="rounded bg-emerald-900/50 px-2 py-0.5 text-xs text-emerald-300">{t("title.continuing")}</span>;
  }
  return null;
}

function TitleTableActionButton({
  label,
  tone,
  className,
  children,
  ...props
}: React.ComponentProps<typeof Button> & {
  label: string;
  tone: BoxedActionButtonTone;
}) {
  return (
    <Button
      type="button"
      size="icon-sm"
      variant="secondary"
      title={label}
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

export function TitleTable({
  view,
  titles,
  titleLoading,
  resolvedProfileName,
  qualityProfiles,
  qualityProfilesLoading,
  onOpenOverview,
  onDelete,
  onAutoQueue,
  onToggleMonitored,
  onInteractiveSearch,
  onQueueFromInteractive,
  isDeletingById,
  isTogglingMonitoredById,
}: TitleTableProps) {
  "use no memo";
  const t = useTranslate();
  const isMovieView = view === "movies";
  const overviewTargetView: ViewId = isMovieView ? "movies" : view === "anime" ? "anime" : "series";
  const columnCount = isMovieView ? 6 : 7;
  const titleTableColGroup = (
    <colgroup>
      <col style={{ width: "5.5rem" }} />
      <col />
      <col style={{ width: "10rem" }} />
      {!isMovieView ? <col style={{ width: "8rem" }} /> : null}
      {isMovieView ? <col style={{ width: "8rem" }} /> : null}
      <col style={{ width: "7rem" }} />
      <col style={{ width: "12.5rem" }} />
    </colgroup>
  );

  const [expandedInteractiveRows, setExpandedInteractiveRows] = React.useState(new Set<string>());
  const [interactiveSearchResultsByTitle, setInteractiveSearchResultsByTitle] = React.useState<
    Record<string, Release[]>
  >({});
  const [interactiveSearchLoadingByTitle, setInteractiveSearchLoadingByTitle] = React.useState<
    Record<string, boolean>
  >({});
  const [autoQueueLoadingByTitle, setAutoQueueLoadingByTitle] = React.useState<Record<string, boolean>>({});
  const [sortKey, setSortKey] = React.useState<SortKey>("name");
  const [sortDirection, setSortDirection] = React.useState<SortDirection>("asc");

  const titleTableScrollRef = React.useRef<HTMLDivElement>(null);
  const sortedTitles = React.useMemo(() => {
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
              resolveDisplayedQualityLabel(left, qualityProfiles, resolvedProfileName, t("label.unknown")),
              resolveDisplayedQualityLabel(right, qualityProfiles, resolvedProfileName, t("label.unknown")),
            );
          case "episodes":
            return compareEpisodeProgressValues(left, right);
          case "status":
            return compareMaybeText(getStatusSortLabel(left), getStatusSortLabel(right));
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
  }, [qualityProfiles, qualityProfilesLoading, resolvedProfileName, sortDirection, sortKey, t, titles]);

  const titleVirtualizer = useVirtualizer({
    count: sortedTitles.length,
    getScrollElement: () => titleTableScrollRef.current,
    estimateSize: () => 96,
    overscan: 5,
  });

  const defaultSortDirectionFor = React.useCallback((key: SortKey): SortDirection => {
    switch (key) {
      case "monitored":
      case "episodes":
      case "size":
        return "desc";
      default:
        return "asc";
    }
  }, []);

  const handleSort = React.useCallback((nextKey: SortKey) => {
    if (sortKey === nextKey) {
      setSortDirection((currentDirection) => (currentDirection === "asc" ? "desc" : "asc"));
      return;
    }

    setSortKey(nextKey);
    setSortDirection(defaultSortDirectionFor(nextKey));
  }, [defaultSortDirectionFor, sortKey]);

  const renderSortIcon = React.useCallback((key: SortKey) => {
    if (sortKey !== key) {
      return null;
    }
    return sortDirection === "asc"
      ? <ArrowUp className="h-3.5 w-3.5" />
      : <ArrowDown className="h-3.5 w-3.5" />;
  }, [sortDirection, sortKey]);

  const renderSortableHeader = React.useCallback((
    key: SortKey,
    label: string,
    className?: string,
    buttonClassName?: string,
  ) => (
    <TableHead
      className={className}
      aria-sort={
        sortKey === key
          ? sortDirection === "asc"
            ? "ascending"
            : "descending"
          : "none"
      }
    >
      <button
        type="button"
        className={cn(
          "inline-flex w-full items-center gap-1 text-left font-medium text-foreground transition-colors hover:text-foreground/80",
          buttonClassName,
        )}
        onClick={() => handleSort(key)}
      >
        <span>{label}</span>
        {renderSortIcon(key)}
      </button>
    </TableHead>
  ), [handleSort, renderSortIcon, sortDirection, sortKey]);

  const handleQueueExisting = React.useCallback(
    (title: TitleRecord) => {
      const titleId = title.id;
      setAutoQueueLoadingByTitle((prev) => ({ ...prev, [titleId]: true }));
      void Promise.resolve(onAutoQueue(title)).finally(() => {
        setAutoQueueLoadingByTitle((prev) => {
          if (!prev[titleId]) return prev;
          const next = { ...prev };
          delete next[titleId];
          return next;
        });
      });
    },
    [onAutoQueue],
  );

  const handleRunInteractiveSearch = React.useCallback(
    (title: TitleRecord) => {
      const titleId = title.id;
      setInteractiveSearchLoadingByTitle((prev) => ({ ...prev, [titleId]: true }));
      void Promise.resolve(onInteractiveSearch(title))
        .then((results) => {
          setInteractiveSearchResultsByTitle((prev) => ({
            ...prev,
            [titleId]: results ?? [],
          }));
        })
        .finally(() => {
          setInteractiveSearchLoadingByTitle((prev) => {
            if (!prev[titleId]) return prev;
            const next = { ...prev };
            delete next[titleId];
            return next;
          });
        });
    },
    [onInteractiveSearch],
  );

  const handleToggleInteractiveSearch = React.useCallback(
    (title: TitleRecord) => {
      const titleId = title.id;
      const isOpen = expandedInteractiveRows.has(titleId);
      setExpandedInteractiveRows((prev) => {
        const next = new Set(prev);
        if (next.has(titleId)) {
          next.delete(titleId);
        } else {
          next.add(titleId);
        }
        return next;
      });
      if (!isOpen && !Object.prototype.hasOwnProperty.call(interactiveSearchResultsByTitle, titleId)) {
        handleRunInteractiveSearch(title);
      }
    },
    [expandedInteractiveRows, handleRunInteractiveSearch, interactiveSearchResultsByTitle],
  );

  const renderTitleRow = (item: TitleRecord) => {
    const isPanelOpen = expandedInteractiveRows.has(item.id);
    const interactiveSearchResults = interactiveSearchResultsByTitle[item.id] ?? [];
    const interactiveSearchLoading = interactiveSearchLoadingByTitle[item.id] === true;
    const autoQueueLoading = autoQueueLoadingByTitle[item.id] === true;
    const deleteLoading = isDeletingById[item.id] === true;
    const monitorToggleLoading = isTogglingMonitoredById?.[item.id] === true;
    const posterThumbUrl = selectPosterVariantUrl(item.posterUrl, "w70");

    return (
      <React.Fragment key={item.id}>
        <TableRow data-ui="title-table-row" className="h-24">
          <TableCell className="align-middle">
            <button
              type="button"
              onClick={() => onOpenOverview(overviewTargetView, item)}
              data-ui="poster-link"
              className="inline-block text-left"
              aria-label={t("media.posterAlt", { name: item.name })}
            >
              <div data-ui="poster-thumb" className="h-20 w-14 overflow-hidden rounded border border-border bg-muted">
                <TitlePosterSlot
                  src={posterThumbUrl}
                  sourceSrc={item.posterSourceUrl}
                  metadataFetchedAt={item.metadataFetchedAt}
                  createdAt={item.createdAt}
                  alt={t("media.posterAlt", { name: item.name })}
                  className="h-full w-full object-cover"
                  placeholderClassName="flex h-full w-full items-center justify-center text-[10px] text-muted-foreground"
                  emptyLabel={t("label.noArt")}
                  loading="lazy"
                />
              </div>
            </button>
          </TableCell>
          <TableCell className="align-middle overflow-hidden">
            <button
              type="button"
              onClick={() => onOpenOverview(overviewTargetView, item)}
              data-ui="title-name"
              className="block w-full overflow-hidden text-left text-xl font-bold hover:text-foreground hover:underline"
            >
              <span className="block truncate">{item.name}</span>
            </button>
          </TableCell>
          <TableCell className="text-center align-middle">
            <span
              className="inline-flex h-6 w-6 shrink-0 items-center justify-center"
              title={`${t("title.table.monitored")}: ${item.name}`}
              aria-label={`${t("title.table.monitored")}: ${item.name}`}
            >
              {item.monitored ? (
                <Eye className="h-5 w-5 text-emerald-600 dark:text-emerald-300" />
              ) : (
                <EyeOff className="h-5 w-5 text-rose-600 dark:text-rose-300" />
              )}
            </span>
          </TableCell>
          <TableCell className="align-middle whitespace-nowrap">
            {qualityProfilesLoading
              ? null
              : resolveDisplayedQualityLabel(
                  item,
                  qualityProfiles,
                  resolvedProfileName,
                  t("label.unknown"),
                )}
          </TableCell>
          {!isMovieView ? (
              <TableCell className="align-middle whitespace-nowrap tabular-nums">
              {formatEpisodeProgress(
                item.episodesOwned,
                item.episodesMonitored,
              )}
            </TableCell>
          ) : null}
          {!isMovieView ? (
            <TableCell className="align-middle whitespace-nowrap">
              <StatusBadge status={item.contentStatus} t={t} />
            </TableCell>
          ) : null}
          {isMovieView ? <TableCell className="align-middle whitespace-nowrap">{bytesToReadable(item.sizeBytes)}</TableCell> : null}
          <TableCell className="text-center align-middle">
            <div data-ui="row-actions" className="inline-flex items-center justify-end gap-2">
              <HoverCard openDelay={3000} closeDelay={75}>
                <HoverCardTrigger asChild>
                  <TitleTableActionButton
                    tone="auto"
                    label={t("label.search")}
                    onClick={() => handleQueueExisting(item)}
                    disabled={autoQueueLoading}
                  >
                    {autoQueueLoading ? (
                      <Loader2 className="h-4 w-4 animate-spin text-emerald-500" />
                    ) : (
                      <Zap className="h-4 w-4" />
                    )}
                  </TitleTableActionButton>
                </HoverCardTrigger>
                <HoverCardContent>
                  <p className="max-w-[18rem] whitespace-normal break-words text-sm">
                    {t("help.autoSearchTooltip")}
                  </p>
                </HoverCardContent>
              </HoverCard>
              <HoverCard openDelay={3000} closeDelay={75}>
                <HoverCardTrigger asChild>
                  <TitleTableActionButton
                    tone="search"
                    label={t("label.interactiveSearch")}
                    onClick={() => handleToggleInteractiveSearch(item)}
                  >
                    <Search className="h-4 w-4" />
                  </TitleTableActionButton>
                </HoverCardTrigger>
                <HoverCardContent>
                  <p className="max-w-[18rem] whitespace-normal break-words text-sm">
                    {t("help.interactiveSearchTooltip")}
                  </p>
                </HoverCardContent>
              </HoverCard>
              {onToggleMonitored ? (
                <TitleTableActionButton
                  tone={item.monitored ? "disabled" : "enabled"}
                  label={t(item.monitored ? "title.unmonitorAction" : "title.monitorAction")}
                  onClick={() => onToggleMonitored(item, !item.monitored)}
                  disabled={monitorToggleLoading}
                >
                  {monitorToggleLoading ? (
                    <Loader2 className="h-4 w-4 animate-spin" />
                  ) : item.monitored ? (
                    <EyeOff className="h-4 w-4" />
                  ) : (
                    <Eye className="h-4 w-4" />
                  )}
                </TitleTableActionButton>
              ) : null}
              <TitleTableActionButton
                tone="delete"
                label={t("label.delete")}
                onClick={() => onDelete(item)}
                disabled={deleteLoading}
              >
                {deleteLoading ? (
                  <Loader2 className="h-4 w-4 animate-spin" />
                ) : (
                  <Trash2 className="h-4 w-4" />
                )}
              </TitleTableActionButton>
            </div>
          </TableCell>
        </TableRow>
        {isPanelOpen ? (
          <TableRow data-ui="title-table-panel-row">
            <TableCell colSpan={columnCount} className="border-t border-border bg-popover/40 p-0">
              <div className="px-4 py-3">
                <div className="mb-2 flex items-center justify-between gap-3">
                  <p className="text-sm text-card-foreground">
                    {t("nzb.searchResultsFor", { name: item.name })}
                  </p>
                  <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    onClick={() => handleRunInteractiveSearch(item)}
                    disabled={interactiveSearchLoading}
                    aria-label={t("label.search")}
                  >
                    <Search className="h-4 w-4" />
                    <span className="ml-1">
                      {interactiveSearchLoading ? t("label.searching") : t("label.refresh")}
                    </span>
                  </Button>
                </div>
                {interactiveSearchLoading ? (
                  <div className="flex items-center gap-3 py-3">
                    <Loader2 className="h-5 w-5 animate-spin text-emerald-500" />
                    <p className="text-sm text-muted-foreground">{t("label.searching")}</p>
                  </div>
                ) : interactiveSearchResults.length === 0 ? (
                  <p className="text-sm text-muted-foreground">{t("nzb.noResultsYet")}</p>
                ) : (
                  <SearchResultBuckets
                    results={interactiveSearchResults}
                    onQueue={(release) => onQueueFromInteractive(item, release)}
                    requireCandidateToken
                  />
                )}
              </div>
            </TableCell>
          </TableRow>
        ) : null}
      </React.Fragment>
    );
  };

  const titleTableHeader = (
    <TableHeader>
      <TableRow className="sticky top-0 z-10 bg-background">
        <TableHead className="w-14" />
        {renderSortableHeader("name", t("label.name"))}
        {renderSortableHeader(
          "monitored",
          t("title.table.monitored"),
          "text-center whitespace-nowrap",
          "justify-center text-center",
        )}
        {renderSortableHeader("quality", t("title.table.qualityTier"), "w-48 whitespace-nowrap")}
        {!isMovieView ? renderSortableHeader("episodes", t("title.table.episodes"), "whitespace-nowrap") : null}
        {!isMovieView ? renderSortableHeader("status", t("title.table.status"), "whitespace-nowrap") : null}
        {isMovieView ? renderSortableHeader("size", t("title.table.size"), "whitespace-nowrap") : null}
        <TableHead className="text-center whitespace-nowrap">{t("label.actions")}</TableHead>
      </TableRow>
    </TableHeader>
  );

  const virtualItems = titleVirtualizer.getVirtualItems();


  return (
    <div
      ref={titleTableScrollRef}
      className="relative w-full"
      style={{ maxHeight: "70vh", overflow: "auto" }}
    >
      <table data-ui="title-table" data-view={view} className="w-full table-fixed caption-bottom text-sm">
        {titleTableColGroup}
        {titleTableHeader}
        {virtualItems.length > 0 ? (
          <>
            {virtualItems[0].start > 0 ? (
              <tbody aria-hidden>
                <tr><td style={{ height: virtualItems[0].start, padding: 0 }} /></tr>
              </tbody>
            ) : null}
            {virtualItems.map((virtualRow) => {
              const item = sortedTitles[virtualRow.index];
              return (
                <tbody
                  key={virtualRow.key}
                  ref={titleVirtualizer.measureElement}
                  data-index={virtualRow.index}
                  className="[&_tr:last-child]:border-0"
                >
                  {renderTitleRow(item)}
                </tbody>
              );
            })}
            {virtualItems[virtualItems.length - 1].end < titleVirtualizer.getTotalSize() ? (
              <tbody aria-hidden>
                <tr>
                  <td
                    style={{
                      height: titleVirtualizer.getTotalSize() - virtualItems[virtualItems.length - 1].end,
                      padding: 0,
                    }}
                  />
                </tr>
              </tbody>
            ) : null}
          </>
        ) : !titleLoading ? (
          <TableBody>
            <TableRow>
              <TableCell colSpan={columnCount} className="text-muted-foreground">
                {t("title.noManaged")}
              </TableCell>
            </TableRow>
          </TableBody>
        ) : null}
      </table>
    </div>
  );
}
