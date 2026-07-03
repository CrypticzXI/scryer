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
import { TitlePosterSlot } from "@/components/title-poster-slot";
import { SearchResultBuckets } from "@/components/common/release-search-results";
import { releaseSupportsAdditionalFileQueue } from "@/lib/utils/release-queue-scope";
import { Checkbox } from "@/components/ui/checkbox";
import { ActionTooltip } from "@/components/ui/tooltip";
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
  titleOverviewInteractiveSearchButtonId,
  titleOverviewInteractiveSearchPanelId,
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
  TitleCollectionEmptyState,
  TitleTableEmptyState,
  TitleTableTooltipActionButton,
  TitleTableLoadingState,
  TITLE_TABLE_ACTION_BUTTON_CLASS,
  TITLE_TABLE_HEADER_CELL_CLASS,
  TITLE_TABLE_HEADER_ROW_CLASS,
  TITLE_TABLE_INTERACTIVE_PANEL_BODY_CLASS,
  TITLE_TABLE_INTERACTIVE_PANEL_ESTIMATED_HEIGHT,
  TITLE_TABLE_ROW_CLASS,
  DEFAULT_TITLE_TABLE_VISIBLE_COLUMNS,
  type TitleTableSortDirection,
  type TitleTableSortKey,
  type TitleTableVisibleColumns,
  useTitleTableVirtualizerRebuild,
} from "./title-table-shared";

type TitleTableProps = {
  view: string;
  titles: TitleRecord[];
  titleLoading: boolean;
  catalogHasMoreTitles?: boolean;
  catalogLoadingMoreTitles?: boolean;
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
  selectedPaneMode?: boolean;
  contextPanelId?: string;
  onSelectTitle?: (title: TitleRecord) => void;
  onDelete: (title: TitleRecord) => void;
  onAutoQueue: (title: TitleRecord) => void;
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
  selectionMode?: boolean;
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

export function TitleTable({
  view,
  titles,
  titleLoading,
  catalogHasMoreTitles = false,
  catalogLoadingMoreTitles = false,
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
  selectedPaneMode = false,
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
  selectionMode = false,
  bulkActionBusy,
  showScanLibraryAction = false,
  showConfigureRootsAction = false,
  configureRootsReason = "missing",
  configureRootsHref,
  onScanLibrary,
  scanLibraryLoading = false,
  scanLibraryDisabled = false,
  scanLibraryNotice,
}: TitleTableProps) {
  "use no memo";
  const location = useLocation();
  const t = useTranslate();
  const dateTimeFormat = useUiDateTimeFormat();
  const isMovieView = view === "movies";
  const overviewTargetView: ViewId = resolveOverviewTargetView(view);
  const [tableWidth, setTableWidth] = React.useState<number | null>(null);
  // Drop lower-priority columns as the table narrows so the Name column stays
  // usable and the table never overflows (which would cut off row highlights).
  const columnWidthBudget = tableWidth ?? Number.POSITIVE_INFINITY;
  const showActionsColumn = visibleColumns.actions;
  const showMonitoredColumn =
    visibleColumns.monitored && columnWidthBudget >= 480;
  const showQualityColumn = visibleColumns.quality && columnWidthBudget >= 600;
  const showEpisodesColumn =
    !isMovieView && visibleColumns.episodes && columnWidthBudget >= 640;
  const showSizeColumn = visibleColumns.size && columnWidthBudget >= 700;
  const showLibraryColumn = visibleColumns.library && columnWidthBudget >= 810;
  const showAddedColumn = visibleColumns.added && columnWidthBudget >= 930;
  const columnCount =
    2 +
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
  const titleTableColGroup = (
    <colgroup>
      <col style={{ width: "3rem" }} />
      <col />
      {showLibraryColumn ? <col style={{ width: "7rem" }} /> : null}
      {showMonitoredColumn ? <col style={{ width: "5.25rem" }} /> : null}
      {showQualityColumn ? <col style={{ width: "8rem" }} /> : null}
      {showEpisodesColumn ? <col style={{ width: "9.5rem" }} /> : null}
      {showSizeColumn ? <col style={{ width: "7rem" }} /> : null}
      {showAddedColumn ? <col style={{ width: "7.5rem" }} /> : null}
      {showActionsColumn ? <col style={{ width: "11.5rem" }} /> : null}
    </colgroup>
  );
  const visibleColumnSignature = [
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
  React.useLayoutEffect(() => {
    const element = titleTableScrollRef.current;
    if (!element || typeof ResizeObserver === "undefined") {
      return;
    }
    const updateWidth = () => {
      const next = Math.round(element.getBoundingClientRect().width);
      setTableWidth((current) => (current === next ? current : next));
    };
    updateWidth();
    const observer = new ResizeObserver(updateWidth);
    observer.observe(element);
    return () => observer.disconnect();
  }, []);
  const sortedTitles = titles;
  const scrollStorageKeySuffix = selectedPaneMode
    ? "poster-table-selected"
    : "poster-table";
  const initialScrollOffset = React.useMemo(
    () =>
      readOverviewSavedScroll(location.pathname, scrollStorageKeySuffix) ?? 0,
    [location.pathname, scrollStorageKeySuffix],
  );
  const expandedInteractiveRowSignature = React.useMemo(
    () => Array.from(expandedInteractiveRows).sort().join("|"),
    [expandedInteractiveRows],
  );

  const titleVirtualizer = useVirtualizer({
    count: sortedTitles.length,
    getScrollElement: () => titleTableScrollRef.current,
    getItemKey: (index) => sortedTitles[index]?.id ?? index,
    estimateSize: (index) => {
      const titleId = sortedTitles[index]?.id;
      return (
        100 +
        (titleId && expandedInteractiveRows.has(titleId)
          ? TITLE_TABLE_INTERACTIVE_PANEL_ESTIMATED_HEIGHT
          : 0)
      );
    },
    initialOffset: initialScrollOffset,
    overscan: 5,
  });
  const getTitleTableMaxScrollTop = useTitleTableVirtualizerRebuild({
    itemCount: sortedTitles.length,
    loading: titleLoading,
    rebuildKey: `${
      selectedPaneMode ? "selected-pane" : "full-table"
    }:${visibleColumnSignature}:${expandedInteractiveRowSignature}`,
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

    element.addEventListener("scroll", maybeLoadNextPage, { passive: true });
    return () => {
      element.removeEventListener("scroll", maybeLoadNextPage);
    };
  }, [
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
      if (selectionMode) {
        onToggleSelected(item.id);
        return;
      }
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
      onToggleSelected,
      scrollStorageKeySuffix,
      selectionMode,
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
      if (isInteractiveTitleRowTarget(event.target)) {
        return;
      }
      if (selectionMode || onSelectTitle) {
        handleActivateTitle(item);
      }
    },
    [handleActivateTitle, isInteractiveTitleRowTarget, onSelectTitle, selectionMode],
  );

  const handleTitleRowKeyDown = React.useCallback(
    (event: React.KeyboardEvent<HTMLTableRowElement>, item: TitleRecord) => {
      if (isInteractiveTitleRowTarget(event.target)) {
        return;
      }
      if (!selectionMode && !onSelectTitle) {
        return;
      }

      if (event.key !== "Enter" && event.key !== " ") {
        return;
      }

      event.preventDefault();
      handleActivateTitle(item);
    },
    [handleActivateTitle, isInteractiveTitleRowTarget, onSelectTitle, selectionMode],
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
            "inline-flex w-full items-center gap-1 text-left text-[11px] font-bold uppercase tracking-[0.05em] text-[var(--scry-faint2)] transition-colors hover:text-[var(--scry-muted2)]",
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
      setInteractiveSearchLoadingByTitle((prev) => ({
        ...prev,
        [titleId]: true,
      }));
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
    const posterThumbUrl = selectPosterVariantUrl(item.posterUrl, "w70");
    const posterActionIconClassName = "h-[18px] w-[18px]";
    const isSelected = selectedTitleId === item.id;
    const isRowHighlighted =
      isSelected || (selectionMode && selectedTitleIds.has(item.id));
    const contextPanelControlsId = onSelectTitle ? contextPanelId : undefined;
    const selectedContextPanelControlsId = isSelected
      ? contextPanelControlsId
      : undefined;
    const addedLabel =
      formatTitleDate(item.createdAt, dateTimeFormat) ?? t("label.unknown");

    return (
      <React.Fragment key={item.id}>
        <TableRow
          id={titleOverviewRowId(item.id)}
          data-ui="title-table-row"
          data-selected={isRowHighlighted ? "true" : undefined}
          aria-selected={onSelectTitle ? isSelected : undefined}
          aria-current={isSelected ? "true" : undefined}
          aria-controls={selectedContextPanelControlsId}
          aria-label={
            onSelectTitle ? t("title.selectTitle", { name: item.name }) : undefined
          }
          aria-keyshortcuts={onSelectTitle ? "Enter Space" : undefined}
          tabIndex={onSelectTitle ? 0 : undefined}
          onClick={(event) => handleTitleRowClick(event, item)}
          onKeyDown={(event) => handleTitleRowKeyDown(event, item)}
          className={cn(
            "h-[100px]",
            TITLE_TABLE_ROW_CLASS,
            selectionMode && "cursor-pointer",
          )}
        >
          <TableCell className="px-0 text-center align-middle">
            <Checkbox
              checked={selectedTitleIds.has(item.id)}
              onCheckedChange={() => onToggleSelected(item.id)}
              aria-label={t("title.selectTitle", { name: item.name })}
              disabled={bulkActionBusy}
              size="table"
              className="mx-auto"
            />
          </TableCell>
          <TableCell className="align-middle overflow-hidden">
            <button
              id={titleOverviewOpenButtonId(item.id)}
              type="button"
              onClick={() => handleActivateTitle(item)}
              data-ui="title-name"
              aria-current={isSelected ? "true" : undefined}
              aria-controls={selectedContextPanelControlsId}
              tabIndex={onSelectTitle ? -1 : undefined}
              className="flex w-full min-w-0 items-center gap-3 overflow-hidden text-left text-[14px] font-semibold leading-5 text-[var(--scry-ink3)] hover:text-foreground"
            >
              <span
                data-ui="poster-thumb"
                className="relative h-[71px] w-12 shrink-0 overflow-hidden rounded-[7px] border border-[var(--scry-border2)] bg-[var(--scry-soft)]"
              >
                <TitlePosterSlot
                  src={posterThumbUrl}
                  sourceSrc={item.posterSourceUrl}
                  metadataFetchedAt={item.metadataFetchedAt}
                  createdAt={item.createdAt}
                  alt=""
                  className="h-full w-full object-cover"
                  placeholderClassName="flex h-full w-full items-center justify-center text-[10px] text-muted-foreground"
                  emptyLabel={t("label.noArt")}
                  loading="lazy"
                />
                <span
                  aria-hidden="true"
                  className="pointer-events-none absolute inset-0 bg-[linear-gradient(180deg,transparent_60%,rgba(4,6,12,0.55))]"
                />
              </span>
              <span className="block min-w-0 truncate">{item.name}</span>
            </button>
          </TableCell>
          {showLibraryColumn ? (
            <TableCell className="overflow-hidden text-center align-middle text-[12.5px] text-[var(--scry-muted)]">
              <span className="block truncate">
                {item.libraryName ?? item.libraryId}
              </span>
            </TableCell>
          ) : null}
          {showMonitoredColumn ? (
            <TableCell className="text-center align-middle">
              <ActionTooltip content={`${t("title.table.monitored")}: ${item.name}`}>
                <span
                  className="inline-flex h-6 w-6 shrink-0 items-center justify-center"
                  aria-label={`${t("title.table.monitored")}: ${item.name}`}
                >
                  {item.monitored ? (
                    <Eye className="size-[17px] text-[var(--scry-success-text-soft)]" />
                  ) : (
                    <EyeOff className="size-[17px] text-[var(--scry-faint2)]" />
                  )}
                </span>
              </ActionTooltip>
            </TableCell>
          ) : null}
          {showQualityColumn ? (
            <TableCell className="whitespace-nowrap text-center align-middle text-[12.5px] text-[var(--scry-text4)]">
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
            <TableCell className="whitespace-nowrap text-center align-middle">
              <TitleEpisodeProgressBar item={item} t={t} />
            </TableCell>
          ) : null}
          {showSizeColumn ? (
            <TableCell className="whitespace-nowrap text-center align-middle font-[var(--font-code)] text-[12.5px] tabular-nums text-[var(--scry-text4)]">
              {bytesToReadable(item.sizeBytes)}
            </TableCell>
          ) : null}
          {showAddedColumn ? (
            <TableCell className="whitespace-nowrap text-center align-middle text-[12px] text-[var(--scry-muted)]">
              {addedLabel}
            </TableCell>
          ) : null}
          {showActionsColumn ? (
            <TableCell className="overflow-hidden px-2 text-center align-middle">
              <div
                data-ui="row-actions"
                className={cn(
                  "flex w-full min-w-0 items-center justify-center gap-1.5",
                  selectionMode && "pointer-events-none opacity-40",
                )}
              >
                <TitleTableTooltipActionButton
                  id={titleOverviewSearchButtonId(item.id)}
                  tone="auto"
                  label={t("label.search")}
                  tooltip={t("help.autoSearchTooltip")}
                  onClick={() => handleQueueExisting(item)}
                  disabled={autoQueueLoading || bulkActionBusy}
                  className={TITLE_TABLE_ACTION_BUTTON_CLASS}
                >
                  {autoQueueLoading ? (
                    <Loader2
                      className={cn(
                        posterActionIconClassName,
                        "animate-spin text-[var(--scry-accent-text)]",
                      )}
                    />
                  ) : (
                    <Zap className={posterActionIconClassName} />
                  )}
                </TitleTableTooltipActionButton>
                <TitleTableTooltipActionButton
                  id={titleOverviewInteractiveSearchButtonId(item.id)}
                  tone="accent"
                  label={t("label.interactiveSearch")}
                  tooltip={t("help.interactiveSearchTooltip")}
                  onClick={() => handleToggleInteractiveSearch(item)}
                  disabled={bulkActionBusy}
                  className={TITLE_TABLE_ACTION_BUTTON_CLASS}
                >
                  <Search className={posterActionIconClassName} />
                </TitleTableTooltipActionButton>
                {onToggleMonitored ? (
                  <TitleTableTooltipActionButton
                    tone="search"
                    label={t(
                      item.monitored
                        ? "title.unmonitorAction"
                        : "title.monitorAction",
                    )}
                    onClick={() => onToggleMonitored(item, !item.monitored)}
                    disabled={monitorToggleLoading || bulkActionBusy}
                    className={TITLE_TABLE_ACTION_BUTTON_CLASS}
                  >
                    {monitorToggleLoading ? (
                      <Loader2
                        className={cn(
                          posterActionIconClassName,
                          "animate-spin",
                        )}
                      />
                    ) : item.monitored ? (
                      <EyeOff className={posterActionIconClassName} />
                    ) : (
                      <Eye className={posterActionIconClassName} />
                    )}
                  </TitleTableTooltipActionButton>
                ) : null}
                <TitleTableTooltipActionButton
                  tone="delete"
                  label={t("label.delete")}
                  onClick={() => onDelete(item)}
                  disabled={deleteLoading || bulkActionBusy}
                  className={TITLE_TABLE_ACTION_BUTTON_CLASS}
                >
                  {deleteLoading ? (
                    <Loader2
                      className={cn(posterActionIconClassName, "animate-spin")}
                    />
                  ) : (
                    <Trash2 className={posterActionIconClassName} />
                  )}
                </TitleTableTooltipActionButton>
              </div>
            </TableCell>
          ) : null}
        </TableRow>
        {isPanelOpen ? (
          <TableRow
            id={titleOverviewInteractiveSearchPanelId(item.id)}
            data-ui="title-table-panel-row"
          >
            <TableCell
              colSpan={columnCount}
              className="border-t border-border bg-popover/40 p-0"
            >
              <div className={TITLE_TABLE_INTERACTIVE_PANEL_BODY_CLASS}>
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
                    <Loader2 className="h-5 w-5 animate-spin text-[var(--scry-accent-text)]" />
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
                    onQueue={(release) => onQueueFromInteractive(item, release)}
                    onQueueAdditional={
                      onQueueAdditionalFromInteractive
                        ? (release) =>
                            onQueueAdditionalFromInteractive(item, release)
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

  const titleTableHeader = (
    <TableHeader>
      <TableRow className={TITLE_TABLE_HEADER_ROW_CLASS}>
        <TableHead className="w-12 text-center">
          <Checkbox
            checked={selectAllState}
            onCheckedChange={(checked) => onToggleSelectAll(checked === true)}
            aria-label={t("title.selectAllTitles")}
            disabled={bulkActionBusy}
            size="table"
            className="mx-auto"
          />
        </TableHead>
        {renderSortableHeader(
          "name",
          t("label.name"),
          TITLE_TABLE_HEADER_CELL_CLASS,
        )}
        {showLibraryColumn
          ? renderSortableHeader(
              "library",
              t("title.table.library"),
              cn("whitespace-nowrap text-center", TITLE_TABLE_HEADER_CELL_CLASS),
              "justify-center text-center uppercase tracking-[0.05em] text-[var(--scry-faint2)]",
            )
          : null}
        {showMonitoredColumn
          ? renderSortableHeader(
              "monitored",
              t("title.table.monitored"),
              cn("text-center whitespace-nowrap", TITLE_TABLE_HEADER_CELL_CLASS),
              "justify-center text-center uppercase tracking-[0.05em] text-[var(--scry-faint2)]",
            )
          : null}
        {showQualityColumn
          ? renderSortableHeader(
              "quality",
              t("title.table.qualityTier"),
              cn("whitespace-nowrap text-center", TITLE_TABLE_HEADER_CELL_CLASS),
              "justify-center text-center uppercase tracking-[0.05em] text-[var(--scry-faint2)]",
            )
          : null}
        {showEpisodesColumn
          ? renderSortableHeader(
              "episodes",
              t("title.table.episodes"),
              cn("whitespace-nowrap text-center", TITLE_TABLE_HEADER_CELL_CLASS),
              "justify-center text-center uppercase tracking-[0.05em] text-[var(--scry-faint2)]",
            )
          : null}
        {showSizeColumn
          ? renderSortableHeader(
              "size",
              t("title.table.size"),
              cn("whitespace-nowrap text-center", TITLE_TABLE_HEADER_CELL_CLASS),
              "justify-center text-center uppercase tracking-[0.05em] text-[var(--scry-faint2)]",
            )
          : null}
        {showAddedColumn
          ? renderSortableHeader(
              "added",
              t("title.contextAdded"),
              cn("whitespace-nowrap text-center", TITLE_TABLE_HEADER_CELL_CLASS),
              "justify-center text-center uppercase tracking-[0.05em] text-[var(--scry-faint2)]",
            )
          : null}
        {showActionsColumn ? (
          <TableHead
            className={cn(
              "whitespace-nowrap px-2 text-center",
              TITLE_TABLE_HEADER_CELL_CLASS,
            )}
          >
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

  if (
    !titleLoading &&
    sortedTitles.length === 0 &&
    showConfigureRootsAction &&
    configureRootsHref
  ) {
    return (
      <div
        data-slot="title-list-root-config-empty"
        className="flex h-full min-h-[22rem] w-full items-start justify-center px-4 pt-12"
      >
        <TitleCollectionEmptyState
          t={t}
          showConfigureRootsAction={showConfigureRootsAction}
          configureRootsReason={configureRootsReason}
          configureRootsHref={configureRootsHref}
        />
      </div>
    );
  }

  return (
    <div
      data-slot="title-list-scroll"
      ref={titleTableScrollRef}
      className={cn(
        "relative h-full min-h-[22rem] w-full overflow-x-hidden overflow-y-auto rounded-[14px] border border-[var(--scry-border)] bg-[var(--scry-surfD)]",
      )}
    >
      <table
        data-ui="title-table"
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
  );
}
