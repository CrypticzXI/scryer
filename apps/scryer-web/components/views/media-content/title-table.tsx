import * as React from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useLocation } from "react-router-dom";
import { useTranslate } from "@/lib/context/translate-context";
import {
  persistOverviewScrollValue,
  readOverviewSavedScroll,
  useOverviewElementScrollRestoration,
} from "@/lib/hooks/use-overview-window-scroll-restoration";
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
import { TitlePosterSlot } from "@/components/title-poster-slot";
import { SearchResultBuckets } from "@/components/common/release-search-results";
import { releaseSupportsAdditionalFileQueue } from "@/lib/utils/release-queue-scope";
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
import { selectPosterVariantUrl } from "@/lib/utils/poster-images";
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
  type TitleTableSortDirection,
  type TitleTableSortKey,
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
  resolvedProfileName: string | null;
  qualityProfiles: ParsedQualityProfile[];
  qualityProfilesLoading: boolean;
  onOpenOverview: (
    targetView: ViewId,
    overviewTarget: OverviewTitleTarget,
  ) => void;
  selectedTitleId?: string | null;
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
  resolvedProfileName,
  qualityProfiles,
  qualityProfilesLoading,
  onOpenOverview,
  selectedTitleId,
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
}: TitleTableProps) {
  "use no memo";
  const location = useLocation();
  const t = useTranslate();
  const isMovieView = view === "movies";
  const overviewTargetView: ViewId = resolveOverviewTargetView(view);
  const columnCount = isMovieView ? 9 : 10;
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
      <col style={{ width: "4.25rem" }} />
      <col />
      <col style={{ width: "7.25rem" }} />
      <col style={{ width: "5.25rem" }} />
      <col style={{ width: "8rem" }} />
      {!isMovieView ? <col style={{ width: "9.5rem" }} /> : null}
      <col style={{ width: "6.75rem" }} />
      <col style={{ width: "7.5rem" }} />
      <col style={{ width: "11rem" }} />
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

  const titleTableScrollRef = React.useRef<HTMLDivElement>(null);
  const sortedTitles = titles;
  const initialScrollOffset = React.useMemo(
    () => readOverviewSavedScroll(location.pathname, "poster-table") ?? 0,
    [location.pathname],
  );

  const titleVirtualizer = useVirtualizer({
    count: sortedTitles.length,
    getScrollElement: () => titleTableScrollRef.current,
    getItemKey: (index) => sortedTitles[index]?.id ?? index,
    estimateSize: () => 76,
    initialOffset: initialScrollOffset,
    overscan: 5,
  });
  const getTitleTableMaxScrollTop = useTitleTableVirtualizerRebuild({
    itemCount: sortedTitles.length,
    loading: titleLoading,
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
    storageKeySuffix: "poster-table",
    scrollRef: titleTableScrollRef,
    getMaxScrollTop: getTitleTableMaxScrollTop,
    restoreScrollTop: restoreTitleTableScroll,
  });

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
    catalogHasMoreTitles,
    catalogLoadingMoreTitles,
    onCatalogEndReached,
    titles.length,
  ]);

  const handleOpenOverview = React.useCallback(
    (item: OverviewTitleTarget) => {
      persistOverviewScrollValue(
        location.pathname,
        "poster-table",
        titleVirtualizer.scrollOffset ?? titleTableScrollRef.current?.scrollTop,
      );
      onOpenOverview(overviewTargetView, item);
    },
    [
      location.pathname,
      onOpenOverview,
      overviewTargetView,
      titleVirtualizer.scrollOffset,
    ],
  );

  const handleActivateTitle = React.useCallback(
    (item: TitleRecord) => {
      if (onSelectTitle) {
        persistOverviewScrollValue(
          location.pathname,
          "poster-table",
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
      titleVirtualizer.scrollOffset,
    ],
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
    const posterActionButtonClassName = "size-8 [&_svg]:size-4";
    const posterActionIconClassName = "h-4 w-4";
    const isSelected = selectedTitleId === item.id;
    const addedLabel = formatTitleDate(item.createdAt) ?? t("label.unknown");

    return (
      <React.Fragment key={item.id}>
        <TableRow
          id={titleOverviewRowId(item.id)}
          data-ui="title-table-row"
          data-selected={isSelected ? "true" : undefined}
          className={cn(
            "h-[4.75rem] transition-colors hover:bg-muted/35",
            isSelected && "bg-primary/10 ring-1 ring-inset ring-primary/30",
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
          <TableCell className="align-middle">
            <button
              type="button"
              onClick={() => handleActivateTitle(item)}
              data-ui="poster-link"
              className="inline-block text-left"
              aria-label={t("media.posterAlt", { name: item.name })}
            >
              <div
                data-ui="poster-thumb"
                className="h-14 w-10 overflow-hidden rounded-[6px] border border-border bg-muted"
              >
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
              id={titleOverviewOpenButtonId(item.id)}
              type="button"
              onClick={() => handleActivateTitle(item)}
              data-ui="title-name"
              className="block w-full overflow-hidden text-left text-[14px] font-semibold leading-5 hover:text-foreground hover:underline"
            >
              <span className="block truncate">{item.name}</span>
            </button>
          </TableCell>
          <TableCell className="align-middle">
            <span className="inline-flex max-w-full rounded border border-border bg-muted/60 px-1.5 py-0.5 text-[11px] font-medium leading-4 text-muted-foreground">
              <span className="truncate">
                {item.libraryName ?? item.libraryId}
              </span>
            </span>
          </TableCell>
          <TableCell className="text-center align-middle">
            <span
              className="inline-flex h-6 w-6 shrink-0 items-center justify-center"
              title={`${t("title.table.monitored")}: ${item.name}`}
              aria-label={`${t("title.table.monitored")}: ${item.name}`}
            >
              {item.monitored ? (
                <Eye className="h-4 w-4 text-emerald-600 dark:text-emerald-300" />
              ) : (
                <EyeOff className="h-4 w-4 text-rose-600 dark:text-rose-300" />
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
            <TableCell className="align-middle whitespace-nowrap">
              <TitleEpisodeProgressBar item={item} t={t} />
            </TableCell>
          ) : null}
          <TableCell className="align-middle whitespace-nowrap">
            {bytesToReadable(item.sizeBytes)}
          </TableCell>
          <TableCell className="align-middle whitespace-nowrap">
            {addedLabel}
          </TableCell>
          <TableCell className="text-center align-middle">
            <div
              data-ui="row-actions"
              className="inline-flex items-center justify-end gap-1.5"
            >
              <TitleTableLazyTooltipActionButton
                id={titleOverviewSearchButtonId(item.id)}
                tone="auto"
                label={t("label.search")}
                tooltip={t("help.autoSearchTooltip")}
                onClick={() => handleQueueExisting(item)}
                disabled={autoQueueLoading || bulkActionBusy}
                className={posterActionButtonClassName}
              >
                {autoQueueLoading ? (
                  <Loader2
                    className={cn(
                      posterActionIconClassName,
                      "animate-spin text-emerald-500",
                    )}
                  />
                ) : (
                  <Zap className={posterActionIconClassName} />
                )}
              </TitleTableLazyTooltipActionButton>
              <TitleTableLazyTooltipActionButton
                tone="search"
                label={t("label.interactiveSearch")}
                tooltip={t("help.interactiveSearchTooltip")}
                onClick={() => handleToggleInteractiveSearch(item)}
                disabled={bulkActionBusy}
                className={posterActionButtonClassName}
              >
                <Search className={posterActionIconClassName} />
              </TitleTableLazyTooltipActionButton>
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
                  className={posterActionButtonClassName}
                >
                  {monitorToggleLoading ? (
                    <Loader2
                      className={cn(posterActionIconClassName, "animate-spin")}
                    />
                  ) : item.monitored ? (
                    <EyeOff className={posterActionIconClassName} />
                  ) : (
                    <Eye className={posterActionIconClassName} />
                  )}
                </TitleTableActionButton>
              ) : null}
              <TitleTableActionButton
                tone="delete"
                label={t("label.delete")}
                onClick={() => onDelete(item)}
                disabled={deleteLoading || bulkActionBusy}
                className={posterActionButtonClassName}
              >
                {deleteLoading ? (
                  <Loader2
                    className={cn(posterActionIconClassName, "animate-spin")}
                  />
                ) : (
                  <Trash2 className={posterActionIconClassName} />
                )}
              </TitleTableActionButton>
            </div>
          </TableCell>
        </TableRow>
        {isPanelOpen ? (
          <TableRow data-ui="title-table-panel-row">
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
      <TableRow className="sticky top-0 z-10 bg-background">
        <TableHead className="w-12 text-center">
          <Checkbox
            checked={selectAllState}
            onCheckedChange={(checked) => onToggleSelectAll(checked === true)}
            aria-label={t("title.selectAllTitles")}
            disabled={bulkActionBusy}
            className="mx-auto size-5 rounded-md [&_svg]:size-4"
          />
        </TableHead>
        <TableHead className="w-14" />
        {renderSortableHeader("name", t("label.name"))}
        <TableHead className="whitespace-nowrap">Library</TableHead>
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
        {renderSortableHeader(
          "size",
          t("title.table.size"),
          "whitespace-nowrap",
        )}
        <TableHead className="whitespace-nowrap">
          {t("title.contextAdded")}
        </TableHead>
        <TableHead className="text-center whitespace-nowrap">
          {t("label.actions")}
        </TableHead>
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
    <div
      ref={titleTableScrollRef}
      className="relative h-full min-h-[22rem] w-full overflow-auto rounded-lg border border-border bg-background/40"
    >
      <table
        data-ui="title-table"
        data-view={view}
        className="min-w-[76rem] table-fixed caption-bottom text-sm"
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
