import * as React from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useTranslate } from "@/lib/context/translate-context";
import { Button } from "@/components/ui/button";
import {
  ArrowDown,
  ArrowUp,
  Eye,
  EyeOff,
  Loader2,
  Search,
  Trash2,
  Zap,
} from "lucide-react";
import {
  HoverCard,
  HoverCardContent,
  HoverCardTrigger,
} from "@/components/ui/hover-card";
import { SearchResultBuckets } from "@/components/common/release-search-results";
import { Checkbox } from "@/components/ui/checkbox";
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
import { cn } from "@/lib/utils";
import {
  bytesToReadable,
  defaultSortDirectionForTitleKey,
  formatEpisodeProgress,
  resolveDisplayedQualityLabel,
  resolveOverviewTargetView,
  sortTitlesForTable,
  StatusBadge,
  TitleTableActionButton,
  type TitleTableSortDirection,
  type TitleTableSortKey,
} from "./title-table-shared";

type CompactTitleTableProps = {
  view: string;
  titles: TitleRecord[];
  titleLoading: boolean;
  resolvedProfileName: string | null;
  qualityProfiles: ParsedQualityProfile[];
  qualityProfilesLoading: boolean;
  onOpenOverview: (targetView: ViewId, overviewTarget: OverviewTitleTarget) => void;
  onDelete: (title: TitleRecord) => void;
  onAutoQueue: (title: TitleRecord) => Promise<void> | void;
  onToggleMonitored?: (title: TitleRecord, monitored: boolean) => Promise<void> | void;
  onInteractiveSearch: (title: TitleRecord) => Promise<Release[]> | Release[];
  onQueueFromInteractive: (title: TitleRecord, release: Release) => void;
  isDeletingById: Record<string, boolean>;
  isTogglingMonitoredById?: Record<string, boolean>;
  selectedTitleIds: ReadonlySet<string>;
  onToggleSelected: (titleId: string) => void;
  onToggleSelectAll: (checked: boolean) => void;
  onClearSelection: () => void;
  onBulkMonitor: (monitored: boolean) => Promise<void> | void;
  onBulkEdit: () => void;
  onBulkDelete: () => void;
  bulkActionBusy: boolean;
};

export function CompactTitleTable({
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
  selectedTitleIds,
  onToggleSelected,
  onToggleSelectAll,
  onClearSelection,
  onBulkMonitor,
  onBulkEdit,
  onBulkDelete,
  bulkActionBusy,
}: CompactTitleTableProps) {
  "use no memo";
  const t = useTranslate();
  const isMovieView = view === "movies";
  const overviewTargetView: ViewId = resolveOverviewTargetView(view);
  const columnCount = isMovieView ? 6 : 7;
  const selectedVisibleCount = titles.filter((title) =>
    selectedTitleIds.has(title.id),
  ).length;
  const allVisibleSelected =
    titles.length > 0 && selectedVisibleCount === titles.length;
  const selectAllState =
    allVisibleSelected
      ? true
      : selectedVisibleCount > 0
        ? "indeterminate"
        : false;

  const titleTableColGroup = (
    <colgroup>
      <col style={{ width: "2.5rem" }} />
      <col />
      <col style={{ width: "5.5rem" }} />
      <col style={{ width: "9rem" }} />
      {!isMovieView ? <col style={{ width: "7rem" }} /> : null}
      {!isMovieView ? <col style={{ width: "8rem" }} /> : null}
      {isMovieView ? <col style={{ width: "7rem" }} /> : null}
      <col style={{ width: "10rem" }} />
    </colgroup>
  );

  const [expandedInteractiveRows, setExpandedInteractiveRows] = React.useState(
    new Set<string>(),
  );
  const [interactiveSearchResultsByTitle, setInteractiveSearchResultsByTitle] =
    React.useState<Record<string, Release[]>>({});
  const [interactiveSearchLoadingByTitle, setInteractiveSearchLoadingByTitle] =
    React.useState<Record<string, boolean>>({});
  const [autoQueueLoadingByTitle, setAutoQueueLoadingByTitle] = React.useState<
    Record<string, boolean>
  >({});
  const [sortKey, setSortKey] = React.useState<TitleTableSortKey>("name");
  const [sortDirection, setSortDirection] =
    React.useState<TitleTableSortDirection>("asc");

  const titleTableScrollRef = React.useRef<HTMLDivElement>(null);
  const sortedTitles = React.useMemo(
    () =>
      sortTitlesForTable({
        titles,
        sortKey,
        sortDirection,
        qualityProfiles,
        resolvedProfileName,
        qualityProfilesLoading,
        t,
      }),
    [
      qualityProfiles,
      qualityProfilesLoading,
      resolvedProfileName,
      sortDirection,
      sortKey,
      t,
      titles,
    ],
  );

  const titleVirtualizer = useVirtualizer({
    count: sortedTitles.length,
    getScrollElement: () => titleTableScrollRef.current,
    estimateSize: () => 48,
    overscan: 8,
  });

  const handleSort = React.useCallback(
    (nextKey: TitleTableSortKey) => {
      if (sortKey === nextKey) {
        setSortDirection((currentDirection) =>
          currentDirection === "asc" ? "desc" : "asc",
        );
        return;
      }

      setSortKey(nextKey);
      setSortDirection(defaultSortDirectionForTitleKey(nextKey));
    },
    [sortKey],
  );

  const renderSortIcon = React.useCallback(
    (key: TitleTableSortKey) => {
      if (sortKey !== key) {
        return null;
      }
      return sortDirection === "asc" ? (
        <ArrowUp className="h-3.5 w-3.5" />
      ) : (
        <ArrowDown className="h-3.5 w-3.5" />
      );
    },
    [sortDirection, sortKey],
  );

  const renderSortableHeader = React.useCallback(
    (
      key: TitleTableSortKey,
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
    ),
    [handleSort, renderSortIcon, sortDirection, sortKey],
  );

  const handleQueueExisting = React.useCallback(
    (title: TitleRecord) => {
      if (bulkActionBusy) {
        return;
      }
      const titleId = title.id;
      setAutoQueueLoadingByTitle((previous) => ({ ...previous, [titleId]: true }));
      void Promise.resolve(onAutoQueue(title)).finally(() => {
        setAutoQueueLoadingByTitle((previous) => {
          if (!previous[titleId]) {
            return previous;
          }
          const next = { ...previous };
          delete next[titleId];
          return next;
        });
      });
    },
    [bulkActionBusy, onAutoQueue],
  );

  const handleRunInteractiveSearch = React.useCallback(
    (title: TitleRecord) => {
      if (bulkActionBusy) {
        return;
      }
      const titleId = title.id;
      setInteractiveSearchLoadingByTitle((previous) => ({
        ...previous,
        [titleId]: true,
      }));
      void Promise.resolve(onInteractiveSearch(title))
        .then((results) => {
          setInteractiveSearchResultsByTitle((previous) => ({
            ...previous,
            [titleId]: results ?? [],
          }));
        })
        .finally(() => {
          setInteractiveSearchLoadingByTitle((previous) => {
            if (!previous[titleId]) {
              return previous;
            }
            const next = { ...previous };
            delete next[titleId];
            return next;
          });
        });
    },
    [bulkActionBusy, onInteractiveSearch],
  );

  const handleToggleInteractiveSearch = React.useCallback(
    (title: TitleRecord) => {
      const titleId = title.id;
      const isOpen = expandedInteractiveRows.has(titleId);
      setExpandedInteractiveRows((previous) => {
        const next = new Set(previous);
        if (next.has(titleId)) {
          next.delete(titleId);
        } else {
          next.add(titleId);
        }
        return next;
      });
      if (
        !isOpen &&
        !Object.prototype.hasOwnProperty.call(
          interactiveSearchResultsByTitle,
          titleId,
        )
      ) {
        handleRunInteractiveSearch(title);
      }
    },
    [expandedInteractiveRows, handleRunInteractiveSearch, interactiveSearchResultsByTitle],
  );

  const renderTitleRow = (item: TitleRecord) => {
    const isPanelOpen = expandedInteractiveRows.has(item.id);
    const interactiveSearchResults =
      interactiveSearchResultsByTitle[item.id] ?? [];
    const interactiveSearchLoading =
      interactiveSearchLoadingByTitle[item.id] === true;
    const autoQueueLoading = autoQueueLoadingByTitle[item.id] === true;
    const deleteLoading = isDeletingById[item.id] === true;
    const monitorToggleLoading = isTogglingMonitoredById?.[item.id] === true;

    return (
      <React.Fragment key={item.id}>
        <TableRow data-ui="compact-title-table-row" className="h-12">
          <TableCell className="align-middle">
            <Checkbox
              checked={selectedTitleIds.has(item.id)}
              onCheckedChange={() => onToggleSelected(item.id)}
              aria-label={t("title.selectTitle", { name: item.name })}
              disabled={bulkActionBusy}
            />
          </TableCell>
          <TableCell className="align-middle overflow-hidden py-1.5">
            <button
              type="button"
              onClick={() => onOpenOverview(overviewTargetView, item)}
              data-ui="title-name"
              className="block w-full overflow-hidden text-left text-[13px] font-medium hover:text-foreground hover:underline"
            >
              <span className="block truncate">{item.name}</span>
            </button>
          </TableCell>
          <TableCell className="text-center align-middle">
            <span
              className="inline-flex h-4 w-4 shrink-0 items-center justify-center"
              title={`${t("title.table.monitored")}: ${item.name}`}
              aria-label={`${t("title.table.monitored")}: ${item.name}`}
            >
              {item.monitored ? (
                <Eye className="h-3.5 w-3.5 text-emerald-600 dark:text-emerald-300" />
              ) : (
                <EyeOff className="h-3.5 w-3.5 text-rose-600 dark:text-rose-300" />
              )}
            </span>
          </TableCell>
          <TableCell className="align-middle whitespace-nowrap py-1.5 text-[13px]">
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
            <TableCell className="align-middle whitespace-nowrap py-1.5 text-[13px] tabular-nums">
              {formatEpisodeProgress(
                item.episodesOwned,
                item.episodesMonitored,
              )}
            </TableCell>
          ) : null}
          {!isMovieView ? (
            <TableCell className="align-middle whitespace-nowrap py-1.5">
              <StatusBadge status={item.contentStatus} t={t} />
            </TableCell>
          ) : null}
          {isMovieView ? (
            <TableCell className="align-middle whitespace-nowrap py-1.5 text-[13px]">
              {bytesToReadable(item.sizeBytes)}
            </TableCell>
          ) : null}
          <TableCell className="text-center align-middle py-1.5">
            <div
              data-ui="row-actions"
              className="inline-flex items-center justify-end gap-1"
            >
              <HoverCard openDelay={3000} closeDelay={75}>
                <HoverCardTrigger asChild>
                  <TitleTableActionButton
                    tone="auto"
                    label={t("label.search")}
                    onClick={() => handleQueueExisting(item)}
                    disabled={autoQueueLoading || bulkActionBusy}
                    className="size-7 rounded-sm"
                  >
                    {autoQueueLoading ? (
                      <Loader2 className="h-3.5 w-3.5 animate-spin text-emerald-500" />
                    ) : (
                      <Zap className="h-3.5 w-3.5" />
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
                    disabled={bulkActionBusy}
                    className="size-7 rounded-sm"
                  >
                    <Search className="h-3.5 w-3.5" />
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
                  label={t(
                    item.monitored
                      ? "title.unmonitorAction"
                      : "title.monitorAction",
                  )}
                  onClick={() => onToggleMonitored(item, !item.monitored)}
                  disabled={monitorToggleLoading || bulkActionBusy}
                  className="size-7 rounded-sm"
                >
                  {monitorToggleLoading ? (
                    <Loader2 className="h-3.5 w-3.5 animate-spin" />
                  ) : item.monitored ? (
                    <EyeOff className="h-3.5 w-3.5" />
                  ) : (
                    <Eye className="h-3.5 w-3.5" />
                  )}
                </TitleTableActionButton>
              ) : null}
              <TitleTableActionButton
                tone="delete"
                label={t("label.delete")}
                onClick={() => onDelete(item)}
                disabled={deleteLoading || bulkActionBusy}
                className="size-7 rounded-sm"
              >
                {deleteLoading ? (
                  <Loader2 className="h-3.5 w-3.5 animate-spin" />
                ) : (
                  <Trash2 className="h-3.5 w-3.5" />
                )}
              </TitleTableActionButton>
            </div>
          </TableCell>
        </TableRow>
        {isPanelOpen ? (
          <TableRow data-ui="compact-title-table-panel-row">
            <TableCell
              colSpan={columnCount}
              className="border-t border-border bg-popover/40 p-0"
            >
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
                    disabled={interactiveSearchLoading || bulkActionBusy}
                    aria-label={t("label.search")}
                  >
                    <Search className="h-4 w-4" />
                    <span className="ml-1">
                      {interactiveSearchLoading
                        ? t("label.searching")
                        : t("label.refresh")}
                    </span>
                  </Button>
                </div>
                {interactiveSearchLoading ? (
                  <div className="flex items-center gap-3 py-3">
                    <Loader2 className="h-5 w-5 animate-spin text-emerald-500" />
                    <p className="text-sm text-muted-foreground">
                      {t("label.searching")}
                    </p>
                  </div>
                ) : interactiveSearchResults.length === 0 ? (
                  <p className="text-sm text-muted-foreground">
                    {t("nzb.noResultsYet")}
                  </p>
                ) : (
                  <SearchResultBuckets
                    results={interactiveSearchResults}
                    onQueue={(release) => {
                      if (bulkActionBusy) {
                        return;
                      }
                      return onQueueFromInteractive(item, release);
                    }}
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
        <TableHead className="w-10 text-center">
          <Checkbox
            checked={selectAllState}
            onCheckedChange={(checked) => onToggleSelectAll(checked === true)}
            aria-label={t("title.selectAllTitles")}
            disabled={bulkActionBusy}
          />
        </TableHead>
        {renderSortableHeader("name", t("label.name"))}
        {renderSortableHeader(
          "monitored",
          t("title.table.monitored"),
          "text-center whitespace-nowrap",
          "justify-center text-center",
        )}
        {renderSortableHeader(
          "quality",
          t("title.table.qualityTier"),
          "whitespace-nowrap",
        )}
        {!isMovieView
          ? renderSortableHeader(
              "episodes",
              t("title.table.episodes"),
              "whitespace-nowrap",
            )
          : null}
        {!isMovieView
          ? renderSortableHeader(
              "status",
              t("title.table.status"),
              "whitespace-nowrap",
            )
          : null}
        {isMovieView
          ? renderSortableHeader(
              "size",
              t("title.table.size"),
              "whitespace-nowrap",
            )
          : null}
        <TableHead className="text-center whitespace-nowrap">
          {t("label.actions")}
        </TableHead>
      </TableRow>
    </TableHeader>
  );

  const virtualItems = titleVirtualizer.getVirtualItems();

  return (
    <div className="space-y-3">
      {selectedVisibleCount > 0 ? (
        <div className="flex flex-col gap-2 rounded-lg border border-border/70 bg-muted/20 px-3 py-3 sm:flex-row sm:items-center sm:justify-between">
          <p className="text-sm text-card-foreground">
            {t("title.bulkSelectionCount", { count: selectedVisibleCount })}
          </p>
          <div className="flex flex-wrap items-center gap-2">
            <Button
              type="button"
              size="sm"
              variant="outline"
              onClick={() => void onBulkMonitor(true)}
              disabled={bulkActionBusy}
            >
              {t("title.monitorAction")}
            </Button>
            <Button
              type="button"
              size="sm"
              variant="outline"
              onClick={() => void onBulkMonitor(false)}
              disabled={bulkActionBusy}
            >
              {t("title.unmonitorAction")}
            </Button>
            <Button
              type="button"
              size="sm"
              variant="outline"
              onClick={onBulkEdit}
              disabled={bulkActionBusy}
            >
              {t("label.edit")}
            </Button>
            <Button
              type="button"
              size="sm"
              variant="outline"
              onClick={onBulkDelete}
              disabled={bulkActionBusy}
            >
              {t("label.delete")}
            </Button>
            <Button
              type="button"
              size="sm"
              variant="ghost"
              onClick={onClearSelection}
              disabled={bulkActionBusy}
            >
              {t("label.clear")}
            </Button>
          </div>
        </div>
      ) : null}

      <div
        ref={titleTableScrollRef}
        className="relative w-full"
        style={{ maxHeight: "70vh", overflow: "auto" }}
      >
        <table
          data-ui="compact-title-table"
          data-view={view}
          className="w-full table-fixed caption-bottom text-sm"
        >
          {titleTableColGroup}
          {titleTableHeader}
          {virtualItems.length > 0 ? (
            <>
              {virtualItems[0].start > 0 ? (
                <tbody aria-hidden>
                  <tr>
                    <td style={{ height: virtualItems[0].start, padding: 0 }} />
                  </tr>
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
              {virtualItems[virtualItems.length - 1].end <
              titleVirtualizer.getTotalSize() ? (
                <tbody aria-hidden>
                  <tr>
                    <td
                      style={{
                        height:
                          titleVirtualizer.getTotalSize() -
                          virtualItems[virtualItems.length - 1].end,
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
    </div>
  );
}
