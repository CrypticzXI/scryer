import * as React from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useLocation } from "react-router-dom";
import { useTranslate } from "@/lib/context/translate-context";
import { useUiDateTimeFormat } from "@/lib/context/ui-settings-context";
import {
  persistOverviewScrollValue,
  readOverviewSavedScroll,
  useOverviewElementScrollRestoration,
} from "@/lib/hooks/use-overview-window-scroll-restoration";
import { Button } from "@/components/ui/button";
import {
  ArrowDown,
  ArrowUp,
  ChevronsUpDown,
  Eye,
  EyeOff,
  Loader2,
  Search,
  Trash2,
  Zap,
} from "lucide-react";
import { SearchResultBuckets } from "@/components/common/release-search-results";
import { TitlePosterSlot } from "@/components/title-poster-slot";
import { releaseSupportsAdditionalFileQueue } from "@/lib/utils/release-queue-scope";
import { selectPosterVariantUrl } from "@/lib/utils/poster-images";
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
  titleOverviewOpenButtonId,
  titleOverviewRowId,
  titleOverviewSearchButtonId,
} from "@/lib/utils/dom-ids";
import {
  bytesToReadable,
  formatTitleDate,
  resolveDisplayedQualityLabel,
  resolveOverviewTargetView,
  TitleEpisodeProgressBar,
  TitleTableActionButton,
  TitleTableEmptyState,
  TitleTableLazyTooltipActionButton,
  TitleTableLoadingState,
  DEFAULT_TITLE_TABLE_VISIBLE_COLUMNS,
  type TitleTableSortDirection,
  type TitleTableSortKey,
  type TitleTableVisibleColumns,
  useTitleTableVirtualizerRebuild,
} from "./title-table-shared";

type CompactTitleTableProps = {
  view: string;
  titles: TitleRecord[];
  titleLoading: boolean;
  catalogHasMoreTitles?: boolean;
  catalogLoadingMoreTitles?: boolean;
  catalogPagingEnabled?: boolean;
  onCatalogEndReached?: () => Promise<void> | void;
  sortKey: TitleTableSortKey;
  sortDirection: TitleTableSortDirection;
  onSortChange: (key: TitleTableSortKey) => void;
  visibleColumns?: TitleTableVisibleColumns;
  resolvedProfileName: string | null;
  qualityProfiles: ParsedQualityProfile[];
  qualityProfilesLoading: boolean;
  onOpenOverview: (
    targetView: ViewId,
    overviewTarget: OverviewTitleTarget,
  ) => void;
  selectedTitleId?: string | null;
  contextPanelId?: string;
  onSelectTitle?: (title: TitleRecord) => void;
  onDelete: (title: TitleRecord) => void;
  onAutoQueue: (title: TitleRecord) => Promise<void> | void;
  onToggleMonitored?: (
    title: TitleRecord,
    monitored: boolean,
  ) => Promise<void> | void;
  onInteractiveSearch: (title: TitleRecord) => Promise<Release[]> | Release[];
  onQueueFromInteractive: (title: TitleRecord, release: Release) => void;
  onQueueAdditionalFromInteractive?: (
    title: TitleRecord,
    release: Release,
  ) => Promise<void> | void;
  isDeletingById: Record<string, boolean>;
  isTogglingMonitoredById?: Record<string, boolean>;
  selectedTitleIds: ReadonlySet<string>;
  onToggleSelected: (titleId: string) => void;
  onToggleSelectAll: (checked: boolean) => void;
  bulkActionBusy: boolean;
  showScanLibraryAction?: boolean;
  showConfigureRootsAction?: boolean;
  configureRootsReason?: "missing" | "invalid";
  configureRootsHref?: string;
  onScanLibrary?: () => Promise<void> | void;
  scanLibraryLoading?: boolean;
  scanLibraryDisabled?: boolean;
  scanLibraryNotice?: string | null;
};

export function CompactTitleTable({
  view,
  titles,
  titleLoading,
  catalogHasMoreTitles = false,
  catalogLoadingMoreTitles = false,
  catalogPagingEnabled = true,
  onCatalogEndReached,
  sortKey,
  sortDirection,
  onSortChange,
  visibleColumns = DEFAULT_TITLE_TABLE_VISIBLE_COLUMNS,
  resolvedProfileName,
  qualityProfiles,
  qualityProfilesLoading,
  onOpenOverview,
  selectedTitleId,
  contextPanelId,
  onSelectTitle,
  onDelete,
  onAutoQueue,
  onToggleMonitored,
  onInteractiveSearch,
  onQueueFromInteractive,
  onQueueAdditionalFromInteractive,
  isDeletingById,
  isTogglingMonitoredById,
  selectedTitleIds,
  onToggleSelected,
  onToggleSelectAll,
  bulkActionBusy,
  showScanLibraryAction = false,
  showConfigureRootsAction = false,
  configureRootsReason = "missing",
  configureRootsHref,
  onScanLibrary,
  scanLibraryLoading = false,
  scanLibraryDisabled = false,
  scanLibraryNotice,
}: CompactTitleTableProps) {
  "use no memo";
  const location = useLocation();
  const t = useTranslate();
  const dateTimeFormat = useUiDateTimeFormat();
  const isMovieView = view === "movies";
  const overviewTargetView: ViewId = resolveOverviewTargetView(view);
  const selectedPaneMode =
    selectedTitleId !== null && onSelectTitle !== undefined;
  const showLibraryColumn = !selectedPaneMode && visibleColumns.library;
  const showMonitoredColumn = selectedPaneMode || visibleColumns.monitored;
  const showQualityColumn = !selectedPaneMode && visibleColumns.quality;
  const showEpisodesColumn =
    !selectedPaneMode && !isMovieView && visibleColumns.episodes;
  const showSizeColumn = selectedPaneMode || visibleColumns.size;
  const showAddedColumn = !selectedPaneMode && visibleColumns.added;
  const showActionsColumn = !selectedPaneMode && visibleColumns.actions;
  const columnCount = selectedPaneMode
    ? 3
    : 2 +
      (showLibraryColumn ? 1 : 0) +
      (showMonitoredColumn ? 1 : 0) +
      (showQualityColumn ? 1 : 0) +
      (showEpisodesColumn ? 1 : 0) +
      (showSizeColumn ? 1 : 0) +
      (showAddedColumn ? 1 : 0) +
      (showActionsColumn ? 1 : 0);
  const selectedVisibleCount = titles.filter((title) =>
    selectedTitleIds.has(title.id),
  ).length;
  const allVisibleSelected =
    titles.length > 0 && selectedVisibleCount === titles.length;
  const selectAllState = allVisibleSelected
    ? true
    : selectedVisibleCount > 0
      ? "indeterminate"
      : false;
  const titleTableColGroup = selectedPaneMode ? (
    <colgroup>
      <col />
      <col style={{ width: "44px" }} />
      <col style={{ width: "76px" }} />
    </colgroup>
  ) : (
    <colgroup>
      <col style={{ width: "3rem" }} />
      <col />
      {showLibraryColumn ? <col style={{ width: "8rem" }} /> : null}
      {showMonitoredColumn ? <col style={{ width: "5.5rem" }} /> : null}
      {showQualityColumn ? <col style={{ width: "9rem" }} /> : null}
      {showEpisodesColumn ? <col style={{ width: "8.5rem" }} /> : null}
      {showSizeColumn ? <col style={{ width: "7.5rem" }} /> : null}
      {showAddedColumn ? <col style={{ width: "7.5rem" }} /> : null}
      {showActionsColumn ? <col style={{ width: "10rem" }} /> : null}
    </colgroup>
  );
  const visibleColumnSignature = selectedPaneMode
    ? "selected-pane"
    : [
        showLibraryColumn && "library",
        showMonitoredColumn && "monitored",
        showQualityColumn && "quality",
        showEpisodesColumn && "episodes",
        showSizeColumn && "size",
        showAddedColumn && "added",
        showActionsColumn && "actions",
      ]
        .filter(Boolean)
        .join(":");

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

  const titleTableScrollRef = React.useRef<HTMLDivElement>(null);
  const sortedTitles = titles;
  const scrollStorageKeySuffix = selectedPaneMode
    ? "compact-selected"
    : "compact";
  const initialScrollOffset = React.useMemo(
    () =>
      readOverviewSavedScroll(location.pathname, scrollStorageKeySuffix) ?? 0,
    [location.pathname, scrollStorageKeySuffix],
  );

  const titleVirtualizer = useVirtualizer({
    count: sortedTitles.length,
    getScrollElement: () => titleTableScrollRef.current,
    getItemKey: (index) => sortedTitles[index]?.id ?? index,
    estimateSize: () => (selectedPaneMode ? 68 : 48),
    initialOffset: initialScrollOffset,
    overscan: 8,
  });
  const getTitleTableMaxScrollTop = useTitleTableVirtualizerRebuild({
    itemCount: sortedTitles.length,
    loading: titleLoading,
    rebuildKey: `${
      selectedPaneMode ? "selected-pane" : "full-table"
    }:${visibleColumnSignature}`,
    scrollRef: titleTableScrollRef,
    titleVirtualizer,
  });
  const restoreTitleTableScroll = React.useCallback(
    (nextTop: number) => {
      titleVirtualizer.scrollToOffset(nextTop);
    },
    [titleVirtualizer],
  );
  useOverviewElementScrollRestoration({
    enabled: true,
    ready: !titleLoading && titles.length > 0,
    storageKeySuffix: scrollStorageKeySuffix,
    scrollRef: titleTableScrollRef,
    getMaxScrollTop: getTitleTableMaxScrollTop,
    restoreScrollTop: restoreTitleTableScroll,
  });

  const selectedTitleScrollKey = selectedTitleId
    ? `${selectedTitleId}:${sortKey}:${sortDirection}`
    : null;
  const autoScrolledSelectedTitleKeyRef = React.useRef<string | null>(null);
  React.useEffect(() => {
    if (!selectedTitleId || !selectedTitleScrollKey) {
      autoScrolledSelectedTitleKeyRef.current = null;
      return;
    }
    if (
      autoScrolledSelectedTitleKeyRef.current === selectedTitleScrollKey ||
      titleLoading ||
      sortedTitles.length === 0
    ) {
      return;
    }

    const selectedIndex = sortedTitles.findIndex(
      (title) => title.id === selectedTitleId,
    );
    if (selectedIndex < 0) {
      return;
    }

    autoScrolledSelectedTitleKeyRef.current = selectedTitleScrollKey;
    const frameId = window.requestAnimationFrame(() => {
      titleVirtualizer.scrollToIndex(selectedIndex, { align: "center" });
    });
    return () => {
      window.cancelAnimationFrame(frameId);
    };
  }, [
    selectedTitleId,
    selectedTitleScrollKey,
    sortedTitles,
    titleLoading,
    titleVirtualizer,
  ]);

  React.useEffect(() => {
    const element = titleTableScrollRef.current;
    if (
      !element ||
      !catalogPagingEnabled ||
      !catalogHasMoreTitles ||
      catalogLoadingMoreTitles ||
      !onCatalogEndReached
    ) {
      return;
    }

    const maybeLoadNextPage = () => {
      if (element.clientHeight <= 0) {
        return;
      }
      const remaining =
        element.scrollHeight - (element.scrollTop + element.clientHeight);
      if (remaining <= 1200) {
        void onCatalogEndReached();
      }
    };

    maybeLoadNextPage();
    element.addEventListener("scroll", maybeLoadNextPage, { passive: true });
    return () => {
      element.removeEventListener("scroll", maybeLoadNextPage);
    };
  }, [
    catalogPagingEnabled,
    catalogHasMoreTitles,
    catalogLoadingMoreTitles,
    onCatalogEndReached,
    titles.length,
  ]);

  const handleOpenOverview = React.useCallback(
    (item: OverviewTitleTarget) => {
      persistOverviewScrollValue(
        location.pathname,
        scrollStorageKeySuffix,
        titleVirtualizer.scrollOffset ?? titleTableScrollRef.current?.scrollTop,
      );
      onOpenOverview(overviewTargetView, item);
    },
    [
      location.pathname,
      onOpenOverview,
      overviewTargetView,
      scrollStorageKeySuffix,
      titleVirtualizer.scrollOffset,
    ],
  );

  const handleActivateTitle = React.useCallback(
    (item: TitleRecord) => {
      if (onSelectTitle) {
        persistOverviewScrollValue(
          location.pathname,
          scrollStorageKeySuffix,
          titleVirtualizer.scrollOffset ??
            titleTableScrollRef.current?.scrollTop,
        );
        onSelectTitle(item);
        return;
      }
      handleOpenOverview(item);
    },
    [
      handleOpenOverview,
      location.pathname,
      onSelectTitle,
      scrollStorageKeySuffix,
      titleVirtualizer.scrollOffset,
    ],
  );

  const isInteractiveTitleRowTarget = React.useCallback(
    (target: EventTarget | null) =>
      target instanceof Element &&
      target.closest(
        'a[href], button, input, select, textarea, [role="button"], [role="checkbox"], [role="menuitem"]',
      ) !== null,
    [],
  );

  const handleTitleRowClick = React.useCallback(
    (event: React.MouseEvent<HTMLTableRowElement>, item: TitleRecord) => {
      if (!onSelectTitle || isInteractiveTitleRowTarget(event.target)) {
        return;
      }
      handleActivateTitle(item);
    },
    [handleActivateTitle, isInteractiveTitleRowTarget, onSelectTitle],
  );

  const handleTitleRowKeyDown = React.useCallback(
    (event: React.KeyboardEvent<HTMLTableRowElement>, item: TitleRecord) => {
      if (!onSelectTitle || isInteractiveTitleRowTarget(event.target)) {
        return;
      }

      if (event.key !== "Enter" && event.key !== " ") {
        return;
      }

      event.preventDefault();
      handleActivateTitle(item);
    },
    [handleActivateTitle, isInteractiveTitleRowTarget, onSelectTitle],
  );

  const handleSort = React.useCallback(
    (nextKey: TitleTableSortKey) => {
      onSortChange(nextKey);
    },
    [onSortChange],
  );

  const renderSortIcon = React.useCallback(
    (key: TitleTableSortKey) => {
      if (sortKey !== key) {
        return (
          <ChevronsUpDown className="h-3.5 w-3.5 text-[var(--scry-muted3)]" />
        );
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
      setAutoQueueLoadingByTitle((previous) => ({
        ...previous,
        [titleId]: true,
      }));
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
    [
      expandedInteractiveRows,
      handleRunInteractiveSearch,
      interactiveSearchResultsByTitle,
    ],
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
    const isSelected = selectedTitleId === item.id;
    const addedLabel =
      formatTitleDate(item.createdAt, dateTimeFormat) ?? t("label.unknown");

    if (selectedPaneMode) {
      const posterUrl = selectPosterVariantUrl(item.posterUrl, "w70");
      const yearLabel = item.year ? String(item.year) : null;
      const libraryLabel = item.libraryName ?? item.libraryId ?? null;
      const qualityLabel = qualityProfilesLoading
        ? null
        : resolveDisplayedQualityLabel(
            item,
            qualityProfiles,
            resolvedProfileName,
            t("label.unknown"),
          );
      const totalEpisodes = item.episodesTotal ?? item.episodesMonitored ?? 0;
      const episodeLabel =
        !isMovieView && totalEpisodes > 0
          ? `${item.episodesOwned ?? 0}/${totalEpisodes} ${t("title.table.episodes")}`
          : null;
      const hasSubline = Boolean(
        yearLabel || episodeLabel || qualityLabel || libraryLabel,
      );
      const contextPanelControlsId = selectedPaneMode
        ? contextPanelId
        : undefined;
      const selectedContextPanelControlsId = isSelected
        ? contextPanelControlsId
        : undefined;

      return (
        <TableRow
          id={titleOverviewRowId(item.id)}
          data-ui="compact-title-table-row"
          data-selected={isSelected ? "true" : undefined}
          aria-selected={selectedPaneMode ? isSelected : undefined}
          aria-current={isSelected ? "true" : undefined}
          aria-controls={selectedContextPanelControlsId}
          aria-label={
            selectedPaneMode
              ? t("title.selectTitle", { name: item.name })
              : undefined
          }
          aria-keyshortcuts={selectedPaneMode ? "Enter Space" : undefined}
          tabIndex={selectedPaneMode ? 0 : undefined}
          onClick={(event) => handleTitleRowClick(event, item)}
          onKeyDown={(event) => handleTitleRowKeyDown(event, item)}
          className={cn(
            "h-[68px] border-b border-[var(--scry-line2)] bg-[var(--scry-card2)] transition-colors hover:bg-[var(--scry-hover)]",
            isSelected &&
              "bg-[rgba(var(--scry-accent-rgb),0.12)] shadow-[inset_3px_0_0_var(--scry-accent-ring)]",
          )}
        >
          <TableCell className="align-middle overflow-hidden py-2 pl-3 pr-2">
            <button
              id={titleOverviewOpenButtonId(item.id)}
              type="button"
              onClick={() => handleActivateTitle(item)}
              data-ui="title-name"
              aria-current={isSelected ? "true" : undefined}
              aria-controls={selectedContextPanelControlsId}
              tabIndex={selectedPaneMode ? -1 : undefined}
              className="flex w-full min-w-0 items-center gap-2.5 overflow-hidden rounded-[8px] p-1 text-left hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            >
              <span className="h-12 w-8 shrink-0 overflow-hidden rounded-[6px] border border-[var(--scry-border2)] bg-[var(--scry-soft)]">
                <TitlePosterSlot
                  src={posterUrl}
                  sourceSrc={item.posterSourceUrl}
                  metadataFetchedAt={item.metadataFetchedAt}
                  createdAt={item.createdAt}
                  alt={t("media.posterAlt", { name: item.name })}
                  className="h-full w-full object-cover"
                  placeholderClassName="flex h-full w-full items-center justify-center px-1 text-center text-[8px] text-muted-foreground"
                  emptyLabel={t("label.noArt")}
                  loading="lazy"
                  decoding="async"
                />
              </span>
              <span className="min-w-0 flex-1">
                <span className="block truncate text-[13px] font-semibold text-[var(--scry-ink2)]">
                  {item.name}
                </span>
                {hasSubline ? (
                  <span className="mt-1 flex min-w-0 items-center gap-1.5 overflow-hidden text-[10.5px] font-medium text-[var(--scry-muted3)]">
                    {yearLabel ? (
                      <span className="shrink-0 tabular-nums">{yearLabel}</span>
                    ) : null}
                    {episodeLabel ? (
                      <span className="shrink-0 rounded-[5px] bg-[var(--scry-chip)] px-1.5 py-0.5 text-[10px] font-semibold text-[var(--scry-muted2)]">
                        {episodeLabel}
                      </span>
                    ) : null}
                    {qualityLabel ? (
                      <span className="max-w-[5.5rem] shrink-0 truncate rounded-[5px] bg-sky-500/15 px-1.5 py-0.5 text-[10px] font-semibold text-sky-300">
                        {qualityLabel}
                      </span>
                    ) : null}
                    {libraryLabel ? (
                      <span className="inline-flex min-w-0 items-center gap-1.5">
                        <span
                          aria-hidden="true"
                          className="size-1.5 shrink-0 rounded-full bg-[var(--scry-accent)]"
                        />
                        <span className="min-w-0 truncate">{libraryLabel}</span>
                      </span>
                    ) : null}
                  </span>
                ) : (
                  <span className="mt-1 block text-[11px] text-[var(--scry-muted3)]">
                    {t("label.unknown")}
                  </span>
                )}
              </span>
            </button>
          </TableCell>
          <TableCell className="text-center align-middle">
            <span
              className="inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-[7px] border border-[var(--scry-line3)] bg-[var(--scry-inset)]"
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
          <TableCell className="align-middle whitespace-nowrap py-2 text-right text-[12px] font-semibold tabular-nums">
            {bytesToReadable(item.sizeBytes)}
          </TableCell>
        </TableRow>
      );
    }

    const contextPanelControlsId = selectedPaneMode ? contextPanelId : undefined;
    const selectedContextPanelControlsId = isSelected
      ? contextPanelControlsId
      : undefined;

    return (
      <React.Fragment key={item.id}>
        <TableRow
          id={titleOverviewRowId(item.id)}
          data-ui="compact-title-table-row"
          data-selected={isSelected ? "true" : undefined}
          aria-selected={selectedPaneMode ? isSelected : undefined}
          aria-current={isSelected ? "true" : undefined}
          aria-controls={selectedContextPanelControlsId}
          aria-label={
            selectedPaneMode
              ? t("title.selectTitle", { name: item.name })
              : undefined
          }
          aria-keyshortcuts={selectedPaneMode ? "Enter Space" : undefined}
          tabIndex={selectedPaneMode ? 0 : undefined}
          onClick={(event) => handleTitleRowClick(event, item)}
          onKeyDown={(event) => handleTitleRowKeyDown(event, item)}
          className={cn(
            "h-12 transition-colors hover:bg-muted/35",
            isSelected &&
              "bg-[rgba(var(--scry-accent-rgb),0.12)] shadow-[inset_3px_0_0_var(--scry-accent-ring)]",
          )}
        >
          <TableCell className="align-middle">
            <Checkbox
              checked={selectedTitleIds.has(item.id)}
              onCheckedChange={() => onToggleSelected(item.id)}
              aria-label={t("title.selectTitle", { name: item.name })}
              disabled={bulkActionBusy}
              className="mx-auto size-5 rounded-md [&_svg]:size-4"
            />
          </TableCell>
          <TableCell className="align-middle overflow-hidden py-1.5">
            <button
              id={titleOverviewOpenButtonId(item.id)}
              type="button"
              onClick={() => handleActivateTitle(item)}
              data-ui="title-name"
              aria-current={isSelected ? "true" : undefined}
              aria-controls={selectedContextPanelControlsId}
              tabIndex={selectedPaneMode ? -1 : undefined}
              className="block w-full overflow-hidden text-left text-[13px] font-medium hover:text-foreground hover:underline"
            >
              <span className="block truncate">{item.name}</span>
            </button>
          </TableCell>
          {showLibraryColumn ? (
            <TableCell className="align-middle overflow-hidden py-1.5 text-[12px] text-muted-foreground">
              <span className="block truncate">
                {item.libraryName ?? item.libraryId}
              </span>
            </TableCell>
          ) : null}
          {showMonitoredColumn ? (
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
          ) : null}
          {showQualityColumn ? (
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
          ) : null}
          {showEpisodesColumn ? (
            <TableCell className="align-middle whitespace-nowrap py-1.5">
              <TitleEpisodeProgressBar item={item} t={t} compact />
            </TableCell>
          ) : null}
          {showSizeColumn ? (
            <TableCell className="align-middle whitespace-nowrap py-1.5 text-[13px]">
              {bytesToReadable(item.sizeBytes)}
            </TableCell>
          ) : null}
          {showAddedColumn ? (
            <TableCell className="align-middle whitespace-nowrap py-1.5 text-[12px] text-muted-foreground">
              {addedLabel}
            </TableCell>
          ) : null}
          {showActionsColumn ? (
            <TableCell className="text-center align-middle py-1.5">
              <div
                data-ui="row-actions"
                className="inline-flex items-center justify-end gap-1"
              >
                <TitleTableLazyTooltipActionButton
                  id={titleOverviewSearchButtonId(item.id)}
                  tone="auto"
                  label={t("label.search")}
                  tooltip={t("help.autoSearchTooltip")}
                  onClick={() => handleQueueExisting(item)}
                  disabled={autoQueueLoading || bulkActionBusy}
                  className="size-7 rounded-sm"
                >
                  {autoQueueLoading ? (
                    <Loader2 className="h-3.5 w-3.5 animate-spin text-emerald-500" />
                  ) : (
                    <Zap className="h-3.5 w-3.5" />
                  )}
                </TitleTableLazyTooltipActionButton>
                {selectedPaneMode ? null : (
                  <TitleTableLazyTooltipActionButton
                    tone="search"
                    label={t("label.interactiveSearch")}
                    tooltip={t("help.interactiveSearchTooltip")}
                    onClick={() => handleToggleInteractiveSearch(item)}
                    disabled={bulkActionBusy}
                    className="size-7 rounded-sm"
                  >
                    <Search className="h-3.5 w-3.5" />
                  </TitleTableLazyTooltipActionButton>
                )}
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
          ) : null}
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
                    onQueueAdditional={
                      onQueueAdditionalFromInteractive
                        ? (release) => {
                            if (bulkActionBusy) {
                              return;
                            }
                            return onQueueAdditionalFromInteractive(
                              item,
                              release,
                            );
                          }
                        : undefined
                    }
                    canQueueAdditional={(release) =>
                      releaseSupportsAdditionalFileQueue(release, item.facet)
                    }
                    disabled={bulkActionBusy}
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

  const titleTableHeader = selectedPaneMode ? (
    <TableHeader>
      <TableRow className="sticky top-0 z-10 h-9 border-b border-[var(--scry-line3)] bg-[var(--scry-surfD)]">
        {renderSortableHeader(
          "name",
          t("label.title"),
          "pl-3 text-[10.5px] font-bold uppercase tracking-normal text-[var(--scry-faint2)]",
          "uppercase tracking-normal text-[var(--scry-faint2)]",
        )}
        <TableHead
          className="whitespace-nowrap text-center text-[10.5px] font-bold uppercase tracking-normal text-[var(--scry-faint2)]"
          title={t("title.table.monitored")}
        >
          MON.
        </TableHead>
        {renderSortableHeader(
          "size",
          t("title.table.size"),
          "whitespace-nowrap pr-3 text-right text-[10.5px] font-bold uppercase tracking-normal text-[var(--scry-faint2)]",
          "justify-end text-right uppercase tracking-normal text-[var(--scry-faint2)]",
        )}
      </TableRow>
    </TableHeader>
  ) : (
    <TableHeader>
      <TableRow className="sticky top-0 z-10 border-b border-[var(--scry-border)] bg-[var(--scry-surfD)]">
        <TableHead className="w-12 text-center">
          <Checkbox
            checked={selectAllState}
            onCheckedChange={(checked) => onToggleSelectAll(checked === true)}
            aria-label={t("title.selectAllTitles")}
            disabled={bulkActionBusy}
            className="mx-auto size-5 rounded-md [&_svg]:size-4"
          />
        </TableHead>
        {renderSortableHeader("name", t("label.name"))}
        {showLibraryColumn
          ? renderSortableHeader(
              "library",
              t("title.table.library"),
              "whitespace-nowrap",
            )
          : null}
        {showMonitoredColumn
          ? renderSortableHeader(
              "monitored",
              t("title.table.monitored"),
              "text-center whitespace-nowrap",
              "justify-center text-center",
            )
          : null}
        {showQualityColumn
          ? renderSortableHeader(
              "quality",
              t("title.table.qualityTier"),
              "whitespace-nowrap",
            )
          : null}
        {showEpisodesColumn
          ? renderSortableHeader(
              "episodes",
              t("title.table.episodes"),
              "whitespace-nowrap",
            )
          : null}
        {showSizeColumn
          ? renderSortableHeader(
              "size",
              t("title.table.size"),
              "whitespace-nowrap",
            )
          : null}
        {showAddedColumn
          ? renderSortableHeader(
              "added",
              t("title.contextAdded"),
              "whitespace-nowrap",
            )
          : null}
        {showActionsColumn ? (
          <TableHead className="text-center whitespace-nowrap">
            {t("label.actions")}
          </TableHead>
        ) : null}
      </TableRow>
    </TableHeader>
  );

  const virtualItems = titleVirtualizer.getVirtualItems();
  const totalVirtualSize = titleVirtualizer.getTotalSize();
  const firstVirtualItem = virtualItems[0];
  const lastVirtualItem = virtualItems[virtualItems.length - 1];
  const topSpacerHeight = firstVirtualItem?.start ?? 0;
  const bottomSpacerHeight = lastVirtualItem
    ? Math.max(totalVirtualSize - lastVirtualItem.end, 0)
    : 0;

  return (
    <div className="flex h-full min-h-0 flex-col gap-3">
      <div
        data-slot="title-list-scroll"
        ref={titleTableScrollRef}
        className={cn(
          "relative flex-1 overflow-auto rounded-[12px] border border-[var(--scry-border2)] bg-[var(--scry-card2)]",
          selectedPaneMode
            ? "min-h-0 rounded-[12px] border-[var(--scry-border2)] bg-[var(--scry-card2)]"
            : "min-h-[22rem]",
        )}
      >
        <table
          data-ui="compact-title-table"
          data-view={view}
          className="w-full table-fixed caption-bottom text-sm"
        >
          {titleTableColGroup}
          {titleTableHeader}
          {sortedTitles.length > 0 ? (
            <TableBody className="[&_tr:last-child]:border-0">
              {topSpacerHeight > 0 ? (
                <tr aria-hidden>
                  <td
                    colSpan={columnCount}
                    style={{ height: topSpacerHeight, padding: 0 }}
                  />
                </tr>
              ) : null}
              {virtualItems.map((virtualRow) => {
                const item = sortedTitles[virtualRow.index];
                if (!item) {
                  return null;
                }
                return (
                  <React.Fragment key={virtualRow.key}>
                    {renderTitleRow(item)}
                  </React.Fragment>
                );
              })}
              {bottomSpacerHeight > 0 ? (
                <tr aria-hidden>
                  <td
                    colSpan={columnCount}
                    style={{
                      height: bottomSpacerHeight,
                      padding: 0,
                    }}
                  />
                </tr>
              ) : null}
            </TableBody>
          ) : titleLoading ? (
            <TableBody>
              <TitleTableLoadingState colSpan={columnCount} />
            </TableBody>
          ) : (
            <TableBody>
              <TitleTableEmptyState
                colSpan={columnCount}
                t={t}
                showScanAction={showScanLibraryAction}
                showConfigureRootsAction={showConfigureRootsAction}
                configureRootsReason={configureRootsReason}
                configureRootsHref={configureRootsHref}
                onScan={onScanLibrary}
                scanLoading={scanLibraryLoading}
                scanDisabled={scanLibraryDisabled}
                scanNotice={scanLibraryNotice}
              />
            </TableBody>
          )}
        </table>
      </div>
    </div>
  );
}
