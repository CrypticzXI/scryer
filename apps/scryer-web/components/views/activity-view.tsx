
import {
  ActivitySquare,
  ArrowDown,
  ArrowDownToLine,
  ArrowUp,
  CheckCircle2,
  ChevronLeft,
  ChevronRight,
  CircleOff,
  CircleAlert,
  Clock3,
  Filter,
  HardDrive,
  Loader2,
  Pause,
  Trash2,
  XCircle,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";
import {
  type UIEvent,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";

import { DownloadClientTypeLogo } from "@/components/common/download-client-type-logo";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { Card, CardContent } from "@/components/ui/card";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { QueueRowItem } from "@/components/views/activity/queue-row-item";
import { QueueTableRow } from "@/components/views/activity/queue-table-row";
import {
  Table,
  TableBody,
  TableCell,
  TableCheckboxHead,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import type {
  ActivitySortKey,
  DownloadActivityStatus,
  DownloadClientFilterOption,
  DownloadHistoryStatus,
  DownloadImportStatus,
  DownloadQueueItem,
  SortConfig,
} from "@/lib/types";
import { useTranslate } from "@/lib/context/translate-context";
import { useIsMobile } from "@/lib/hooks/use-mobile";
import { selectorId } from "@/lib/utils/dom-ids";
import { cn } from "@/lib/utils";
import { downloadQueueItemIdentityKey } from "@/lib/utils/download-queue";
import {
  activityStatusRank,
  type ActivityTab,
  canDeleteImportItem,
  canIgnoreImportItem,
  compareStrings,
  deriveQueueRowPresentation,
  downloadQueueItemRowSelectorKey,
  effectiveQueueItemProgress,
  parseByteCount,
  type QueueRowPresentation,
  queueStateLabels,
  type TranslateFn,
} from "@/lib/utils/activity-utils";

type ActivityViewState = {
  queueItems: DownloadQueueItem[];
  queueLoading: boolean;
  queueLoadingMore: boolean;
  queueError: string | null;
  requestManualImport: (item: DownloadQueueItem) => Promise<void>;
  requestAssignTitle: (item: DownloadQueueItem) => Promise<void>;
  requestIgnore: (item: DownloadQueueItem) => Promise<void>;
  requestMarkFailed: (item: DownloadQueueItem, skipReacquire: boolean) => Promise<void>;
  requestIgnoreItems: (items: DownloadQueueItem[]) => Promise<void>;
  requestPause: (item: DownloadQueueItem) => Promise<void>;
  requestResume: (item: DownloadQueueItem) => Promise<void>;
  requestDelete: (item: DownloadQueueItem) => Promise<void>;
  requestDeleteItems: (items: DownloadQueueItem[]) => Promise<void>;
  activeTab: ActivityTab;
  sortConfigByTab: Record<ActivityTab, SortConfig>;
  toggleSort: (tab: ActivityTab, key: ActivitySortKey) => void;
  activityScryerSubmittedOnly: boolean;
  toggleActivityScryerSubmittedOnly: () => void;
  historyScryerSubmittedOnly: boolean;
  toggleHistoryScryerSubmittedOnly: () => void;
  selectedImportStatuses: DownloadImportStatus[];
  toggleImportStatus: (status: DownloadImportStatus) => void;
  selectedActivityStatuses: DownloadActivityStatus[];
  toggleActivityStatus: (status: DownloadActivityStatus) => void;
  selectedHistoryStatuses: DownloadHistoryStatus[];
  toggleHistoryStatus: (status: DownloadHistoryStatus) => void;
  activityAvailableClients: DownloadClientFilterOption[];
  selectedActivityClientIds: string[];
  toggleActivityClientId: (clientId: string) => void;
  historyAvailableClients: DownloadClientFilterOption[];
  selectedHistoryClientIds: string[];
  toggleHistoryClientId: (clientId: string) => void;
  historyPage: number;
  historyTotalPages: number;
  goToPreviousHistoryPage: () => Promise<void>;
  goToNextHistoryPage: () => Promise<void>;
  historyHasPreviousPage: boolean;
  historyHasNextPage: boolean;
  visibleHasMore: boolean;
  requestMoreItems: () => Promise<void>;
};

type ActivityFilterChipOption<T extends string> = {
  value: T;
  labelKey: string;
  icon: LucideIcon;
  iconClassName?: string;
};

const importFilterOptions: ActivityFilterChipOption<DownloadImportStatus>[] = [
  {
    value: "IMPORTING",
    labelKey: "activity.importFilter.importing",
    icon: HardDrive,
    iconClassName: "text-[var(--scry-info-text-soft)]",
  },
  {
    value: "PENDING",
    labelKey: "activity.importFilter.pending",
    icon: Clock3,
    iconClassName: "text-[var(--scry-accent-text)]",
  },
  {
    value: "BLOCKED",
    labelKey: "activity.importFilter.blocked",
    icon: CircleAlert,
    iconClassName: "text-[var(--scry-warning-text)]",
  },
  {
    value: "FAILED",
    labelKey: "activity.importFilter.failed",
    icon: XCircle,
    iconClassName: "text-[var(--scry-danger-text-soft)]",
  },
];

const activityFilterOptions: ActivityFilterChipOption<DownloadActivityStatus>[] = [
  {
    value: "DOWNLOADING",
    labelKey: "activity.activityFilter.downloading",
    icon: ArrowDownToLine,
    iconClassName: "text-[var(--scry-info-text-soft)]",
  },
  {
    value: "QUEUED",
    labelKey: "activity.activityFilter.queued",
    icon: Clock3,
    iconClassName: "text-[var(--scry-warning-text)]",
  },
  {
    value: "PAUSED",
    labelKey: "activity.activityFilter.paused",
    icon: Pause,
    iconClassName: "text-[var(--scry-warning-text)]",
  },
  {
    value: "POST_PROCESSING",
    labelKey: "activity.activityFilter.postProcessing",
    icon: HardDrive,
    iconClassName: "text-[var(--scry-info-text-soft)]",
  },
];

const historyFilterOptions: ActivityFilterChipOption<DownloadHistoryStatus>[] = [
  {
    value: "SUCCESS",
    labelKey: "activity.historyFilter.success",
    icon: CheckCircle2,
    iconClassName: "text-[var(--scry-success-text-soft)]",
  },
  {
    value: "FAILED",
    labelKey: "activity.historyFilter.failed",
    icon: XCircle,
    iconClassName: "text-[var(--scry-danger-text-soft)]",
  },
];

function ActivityFilterSection<T extends string>({
  title,
  options,
  selectedValues,
  onToggle,
  t,
}: {
  title: string;
  options: ActivityFilterChipOption<T>[];
  selectedValues: string[];
  onToggle: (value: T) => void;
  t: TranslateFn;
}) {
  return (
    <div className="flex flex-col gap-2">
      <p className="text-xs font-medium text-muted-foreground">{title}</p>
      <div className="flex flex-col gap-1">
        {options.map((option) => {
          const Icon = option.icon;
          const isSelected = selectedValues.includes(option.value);
          return (
            <label
              key={option.value}
              className="flex cursor-pointer items-center gap-2 rounded-md px-1.5 py-1 text-sm hover:bg-accent/50"
            >
              <Checkbox
                checked={isSelected}
                size="compact"
                onCheckedChange={() => onToggle(option.value)}
              />
              <Icon
                className={cn(
                  "h-[14px] w-[14px] shrink-0",
                  option.iconClassName ?? "text-muted-foreground",
                )}
                aria-hidden="true"
              />
              <span>{t(option.labelKey)}</span>
            </label>
          );
        })}
      </div>
    </div>
  );
}

function ActivityClientFilterSection({
  title,
  options,
  selectedValues,
  onToggle,
}: {
  title: string;
  options: DownloadClientFilterOption[];
  selectedValues: string[];
  onToggle: (clientId: string) => void;
}) {
  return (
    <div className="flex flex-col gap-2">
      <p className="text-xs font-medium text-muted-foreground">{title}</p>
      <div className="flex flex-col gap-1">
        {options.map((option) => {
          const isSelected = selectedValues.includes(option.clientId);
          return (
            <label
              key={option.clientId}
              className="flex cursor-pointer items-center gap-2 rounded-md px-1.5 py-1 text-sm hover:bg-accent/50"
              title={`${option.clientName} • ${option.clientType}`}
            >
              <Checkbox
                checked={isSelected}
                size="compact"
                onCheckedChange={() => onToggle(option.clientId)}
              />
              <DownloadClientTypeLogo
                typeValue={option.clientType}
                className="h-[14px] w-[14px] shrink-0"
              />
              <span>{option.clientName || option.clientType}</span>
            </label>
          );
        })}
      </div>
    </div>
  );
}

function ActivityBooleanFilterSection({
  title,
  label,
  checked,
  onToggle,
}: {
  title: string;
  label: string;
  checked: boolean;
  onToggle: () => void;
}) {
  return (
    <div className="flex flex-col gap-2">
      <p className="text-xs font-medium text-muted-foreground">{title}</p>
      <label className="flex cursor-pointer items-center gap-2 rounded-md px-1.5 py-1 text-sm hover:bg-accent/50">
        <Checkbox
          checked={checked}
          size="compact"
          onCheckedChange={onToggle}
        />
        <span>{label}</span>
      </label>
    </div>
  );
}

function ActivityTableLoadingMask({ label }: { label: string }) {
  return (
    <div className="flex items-center justify-center py-16">
      <div className="inline-flex items-center gap-2 rounded-full border border-border/70 bg-background/90 px-4 py-2 text-sm text-muted-foreground shadow-sm backdrop-blur-sm">
        <Loader2 className="h-4 w-4 animate-spin" />
        <span>{label}</span>
      </div>
    </div>
  );
}

export function ActivityView({ state }: { state: ActivityViewState }) {
  const t = useTranslate();
  const isMobile = useIsMobile();
  const {
    queueItems,
    queueLoading,
    queueLoadingMore,
    queueError,
    requestManualImport,
    requestAssignTitle,
    requestIgnore,
    requestMarkFailed,
    requestIgnoreItems,
    requestPause,
    requestResume,
    requestDelete,
    requestDeleteItems,
    activeTab,
    sortConfigByTab,
    toggleSort,
    activityScryerSubmittedOnly,
    toggleActivityScryerSubmittedOnly,
    historyScryerSubmittedOnly,
    toggleHistoryScryerSubmittedOnly,
    selectedImportStatuses,
    toggleImportStatus,
    selectedActivityStatuses,
    toggleActivityStatus,
    selectedHistoryStatuses,
    toggleHistoryStatus,
    activityAvailableClients,
    selectedActivityClientIds,
    toggleActivityClientId,
    historyAvailableClients,
    selectedHistoryClientIds,
    toggleHistoryClientId,
    historyPage,
    historyTotalPages,
    goToPreviousHistoryPage,
    goToNextHistoryPage,
    historyHasPreviousPage,
    historyHasNextPage,
    visibleHasMore,
    requestMoreItems,
  } = state;
  const [actionLoadingId, setActionLoadingId] = useState<string | null>(null);
  const [deleteConfirmItem, setDeleteConfirmItem] = useState<DownloadQueueItem | null>(null);
  const [bulkDeleteConfirmItems, setBulkDeleteConfirmItems] = useState<DownloadQueueItem[]>([]);
  const [deleteInProgress, setDeleteInProgress] = useState(false);
  const [bulkActionInProgress, setBulkActionInProgress] = useState<"ignore" | "delete" | null>(
    null,
  );
  const [rowActionBusy, setRowActionBusy] = useState<Record<string, true>>({});
  const [expandedItemIds, setExpandedItemIds] = useState<Record<string, true>>({});
  const [selectedImportItemKeys, setSelectedImportItemKeys] = useState<Record<string, true>>({});
  const [filterPopoverOpen, setFilterPopoverOpen] = useState(false);
  const rowActionBusyRef = useRef<Record<string, true>>({});
  const scrollHeightClass = isMobile ? "max-h-[70vh]" : "max-h-[1700px]";

  const setRowBusy = useCallback((rowId: string, busy: boolean) => {
    rowActionBusyRef.current = busy
      ? { ...rowActionBusyRef.current, [rowId]: true }
      : Object.fromEntries(
          Object.entries(rowActionBusyRef.current).filter(([id]) => id !== rowId),
        );
    setRowActionBusy((current) => {
      if (!busy) {
        const { [rowId]: _removed, ...next } = current;
        return next;
      }
      if (current[rowId]) {
        return current;
      }
      return {
        ...current,
        [rowId]: true,
      };
    });
  }, []);

  const handleDelete = useCallback(async () => {
    if (!deleteConfirmItem) return;
    const rowId = downloadQueueItemIdentityKey(deleteConfirmItem);
    setRowBusy(rowId, true);
    setDeleteInProgress(true);
    try {
      await requestDelete(deleteConfirmItem);
    } finally {
      setDeleteInProgress(false);
      setRowBusy(rowId, false);
      setDeleteConfirmItem(null);
    }
  }, [deleteConfirmItem, requestDelete, setRowBusy]);

  const clearSelectedImportItems = useCallback((items: DownloadQueueItem[]) => {
    const keys = new Set(items.map(downloadQueueItemIdentityKey));
    setSelectedImportItemKeys((current) => {
      const next = Object.fromEntries(
        Object.entries(current).filter(([key]) => !keys.has(key)),
      );
      return Object.keys(next).length === Object.keys(current).length ? current : next;
    });
  }, []);

  const handleBulkIgnore = useCallback(async (items: DownloadQueueItem[]) => {
    if (items.length === 0) {
      return;
    }

    const rowIds = items.map(downloadQueueItemIdentityKey);
    rowIds.forEach((rowId) => setRowBusy(rowId, true));
    setBulkActionInProgress("ignore");
    try {
      await requestIgnoreItems(items);
      clearSelectedImportItems(items);
    } finally {
      rowIds.forEach((rowId) => setRowBusy(rowId, false));
      setBulkActionInProgress(null);
    }
  }, [clearSelectedImportItems, requestIgnoreItems, setRowBusy]);

  const handleBulkDelete = useCallback(async () => {
    if (bulkDeleteConfirmItems.length === 0) {
      return;
    }

    const items = bulkDeleteConfirmItems;
    const rowIds = items.map(downloadQueueItemIdentityKey);
    rowIds.forEach((rowId) => setRowBusy(rowId, true));
    setBulkActionInProgress("delete");
    setDeleteInProgress(true);
    try {
      await requestDeleteItems(items);
      clearSelectedImportItems(items);
      setBulkDeleteConfirmItems([]);
    } finally {
      setDeleteInProgress(false);
      setBulkActionInProgress(null);
      rowIds.forEach((rowId) => setRowBusy(rowId, false));
    }
  }, [
    bulkDeleteConfirmItems,
    clearSelectedImportItems,
    requestDeleteItems,
    setRowBusy,
  ]);

  const toggleExpandedDetails = useCallback((rowId: string) => {
    setExpandedItemIds((current) => {
      if (current[rowId]) {
        const { [rowId]: _removed, ...next } = current;
        return next;
      }

      return {
        ...current,
        [rowId]: true,
      };
    });
  }, []);

  const handleResultsScroll = useCallback(
    (event: UIEvent<HTMLDivElement>) => {
      if (activeTab !== "import" || queueLoadingMore || !visibleHasMore || queueLoading) {
        return;
      }

      const element = event.currentTarget;
      if (element.scrollHeight - element.scrollTop - element.clientHeight <= 160) {
        void requestMoreItems();
      }
    },
    [activeTab, queueLoading, queueLoadingMore, requestMoreItems, visibleHasMore],
  );

  const emptyStateLabel =
    activeTab === "import"
      ? t("activity.importEmpty")
      : activeTab === "history"
        ? t("activity.historyEmpty")
        : t("activity.activityEmpty");
  const activeSortConfig = sortConfigByTab[activeTab];

  const handleSort = useCallback(
    (nextKey: ActivitySortKey) => {
      toggleSort(activeTab, nextKey);
    },
    [activeTab, toggleSort],
  );

  const renderSortIcon = useCallback(
    (key: ActivitySortKey) => {
      if (activeSortConfig.key !== key) {
        return null;
      }

      return activeSortConfig.direction === "ASC" ? (
        <ArrowUp className="h-3.5 w-3.5" />
      ) : (
        <ArrowDown className="h-3.5 w-3.5" />
      );
    },
    [activeSortConfig.direction, activeSortConfig.key],
  );

  const renderSortableHeader = useCallback(
    (key: ActivitySortKey, label: string, className?: string) => (
      <TableHead
        className={className}
        aria-sort={
          activeSortConfig.key === key
            ? activeSortConfig.direction === "ASC"
              ? "ascending"
              : "descending"
            : "none"
        }
      >
        <button
          type="button"
          className="inline-flex w-full items-center gap-1 text-left font-medium text-foreground transition-colors hover:text-foreground/80"
          onClick={() => handleSort(key)}
        >
          <span>{label}</span>
          {renderSortIcon(key)}
        </button>
      </TableHead>
    ),
    [activeSortConfig.direction, activeSortConfig.key, handleSort, renderSortIcon],
  );

  const sortedQueueItems = useMemo(() => {
    if (activeTab === "history") {
      return queueItems;
    }

    const directionMultiplier = activeSortConfig.direction === "ASC" ? 1 : -1;
    const items = [...queueItems];

    items.sort((leftItem, rightItem) => {
      let comparison = 0;

      switch (activeSortConfig.key) {
        case "TITLE": {
          const leftTitle = leftItem.titleName.trim() || leftItem.downloadClientItemId.trim();
          const rightTitle = rightItem.titleName.trim() || rightItem.downloadClientItemId.trim();
          comparison = compareStrings(leftTitle, rightTitle);
          break;
        }
        case "CLIENT": {
          const leftClient = leftItem.clientName.trim() || leftItem.clientType.trim();
          const rightClient = rightItem.clientName.trim() || rightItem.clientType.trim();
          comparison = compareStrings(leftClient, rightClient);
          if (comparison === 0) {
            comparison = compareStrings(leftItem.clientType, rightItem.clientType);
          }
          break;
        }
        case "STATUS": {
          comparison =
            activityStatusRank(activeTab, leftItem.displayState) -
            activityStatusRank(activeTab, rightItem.displayState);
          if (comparison === 0) {
            const leftStatus = t(queueStateLabels[leftItem.displayState.toLowerCase()] ?? "queue.state.unknown");
            const rightStatus = t(
              queueStateLabels[rightItem.displayState.toLowerCase()] ?? "queue.state.unknown",
            );
            comparison = compareStrings(leftStatus, rightStatus);
          }
          break;
        }
        case "PROGRESS": {
          comparison =
            effectiveQueueItemProgress(leftItem) - effectiveQueueItemProgress(rightItem);
          break;
        }
        case "SIZE": {
          const leftSize = parseByteCount(leftItem.sizeBytes) ?? 0;
          const rightSize = parseByteCount(rightItem.sizeBytes) ?? 0;
          comparison = leftSize - rightSize;
          break;
        }
      }

      if (comparison === 0) {
        const leftTitle = leftItem.titleName.trim() || leftItem.downloadClientItemId.trim();
        const rightTitle = rightItem.titleName.trim() || rightItem.downloadClientItemId.trim();
        comparison = compareStrings(leftTitle, rightTitle);
      }

      return comparison * directionMultiplier;
    });

    return items;
  }, [activeSortConfig.direction, activeSortConfig.key, activeTab, queueItems, t]);

  const visibleImportItems = useMemo(
    () => (activeTab === "import" ? sortedQueueItems : []),
    [activeTab, sortedQueueItems],
  );
  const selectedImportItems = useMemo(
    () =>
      visibleImportItems.filter((item) => selectedImportItemKeys[downloadQueueItemIdentityKey(item)]),
    [selectedImportItemKeys, visibleImportItems],
  );
  const selectedImportCount = selectedImportItems.length;
  const visibleImportKeys = useMemo(
    () => visibleImportItems.map(downloadQueueItemIdentityKey),
    [visibleImportItems],
  );
  const allVisibleImportItemsSelected =
    visibleImportKeys.length > 0 &&
    visibleImportKeys.every((key) => selectedImportItemKeys[key]);
  const someVisibleImportItemsSelected =
    !allVisibleImportItemsSelected &&
    visibleImportKeys.some((key) => selectedImportItemKeys[key]);
  const selectedIgnoreItems = useMemo(
    () =>
      selectedImportItems.filter((item) => {
        const rowId = downloadQueueItemIdentityKey(item);
        return (
          canIgnoreImportItem(item) &&
          !rowActionBusy[rowId] &&
          actionLoadingId !== rowId
        );
      }),
    [actionLoadingId, rowActionBusy, selectedImportItems],
  );
  const selectedDeleteItems = useMemo(
    () =>
      selectedImportItems.filter((item) => {
        const rowId = downloadQueueItemIdentityKey(item);
        return (
          canDeleteImportItem(item) &&
          !rowActionBusy[rowId] &&
          actionLoadingId !== rowId
        );
      }),
    [actionLoadingId, rowActionBusy, selectedImportItems],
  );

  useEffect(() => {
    if (activeTab !== "import") {
      setSelectedImportItemKeys({});
      return;
    }

    const visibleKeys = new Set(visibleImportKeys);
    setSelectedImportItemKeys((current) => {
      const next = Object.fromEntries(
        Object.entries(current).filter(([key]) => visibleKeys.has(key)),
      );
      return Object.keys(next).length === Object.keys(current).length ? current : next;
    });
  }, [activeTab, visibleImportKeys]);

  const toggleImportItemSelected = useCallback((item: DownloadQueueItem) => {
    const rowId = downloadQueueItemIdentityKey(item);
    setSelectedImportItemKeys((current) => {
      if (current[rowId]) {
        const { [rowId]: _removed, ...next } = current;
        return next;
      }
      return {
        ...current,
        [rowId]: true,
      };
    });
  }, []);

  const toggleAllVisibleImportItemsSelected = useCallback(() => {
    setSelectedImportItemKeys((current) => {
      if (visibleImportKeys.length === 0) {
        return current;
      }

      const allSelected = visibleImportKeys.every((key) => current[key]);
      if (allSelected) {
        const visibleKeySet = new Set(visibleImportKeys);
        return Object.fromEntries(
          Object.entries(current).filter(([key]) => !visibleKeySet.has(key)),
        );
      }

      const next = { ...current };
      for (const key of visibleImportKeys) {
        next[key] = true;
      }
      return next;
    });
  }, [visibleImportKeys]);

  const renderFilterPopoverContent = useCallback(() => {
    if (activeTab === "import") {
      return (
        <ActivityFilterSection
          title={t("queue.status")}
          options={importFilterOptions}
          selectedValues={selectedImportStatuses}
          onToggle={(value) => toggleImportStatus(value as DownloadImportStatus)}
          t={t}
        />
      );
    }

    if (activeTab === "history") {
      return (
        <div className="flex flex-col gap-4">
          <ActivityFilterSection
            title={t("queue.status")}
            options={historyFilterOptions}
            selectedValues={selectedHistoryStatuses}
            onToggle={(value) => toggleHistoryStatus(value as DownloadHistoryStatus)}
            t={t}
          />
          <ActivityBooleanFilterSection
            title={t("queue.source")}
            label={t("activity.scryerSubmitted")}
            checked={historyScryerSubmittedOnly}
            onToggle={toggleHistoryScryerSubmittedOnly}
          />
          {historyAvailableClients.length > 0 ? (
            <ActivityClientFilterSection
              title={t("queue.client")}
              options={historyAvailableClients}
              selectedValues={selectedHistoryClientIds}
              onToggle={toggleHistoryClientId}
            />
          ) : null}
        </div>
      );
    }

    return (
      <div className="flex flex-col gap-4">
        <ActivityFilterSection
          title={t("queue.status")}
          options={activityFilterOptions}
          selectedValues={selectedActivityStatuses}
          onToggle={(value) => toggleActivityStatus(value as DownloadActivityStatus)}
          t={t}
        />
        <ActivityBooleanFilterSection
          title={t("queue.source")}
          label={t("activity.scryerSubmitted")}
          checked={activityScryerSubmittedOnly}
          onToggle={toggleActivityScryerSubmittedOnly}
        />
        {activityAvailableClients.length > 0 ? (
          <ActivityClientFilterSection
            title={t("queue.client")}
            options={activityAvailableClients}
            selectedValues={selectedActivityClientIds}
            onToggle={toggleActivityClientId}
          />
        ) : null}
      </div>
    );
  }, [
    activeTab,
    activityAvailableClients,
    activityScryerSubmittedOnly,
    selectedActivityClientIds,
    selectedActivityStatuses,
    historyScryerSubmittedOnly,
    selectedHistoryClientIds,
    selectedHistoryStatuses,
    selectedImportStatuses,
    t,
    toggleActivityClientId,
    toggleActivityScryerSubmittedOnly,
    toggleActivityStatus,
    toggleHistoryClientId,
    toggleHistoryScryerSubmittedOnly,
    toggleHistoryStatus,
    toggleImportStatus,
    historyAvailableClients,
  ]);

  const buildQueueRowProps = useCallback(
    (queueItem: DownloadQueueItem) => {
      const rowId = downloadQueueItemIdentityKey(queueItem);
      const row: QueueRowPresentation = deriveQueueRowPresentation(queueItem, t);
      const rowSelectorKey = selectorId(
        downloadQueueItemRowSelectorKey(queueItem, rowId),
      );
      const isActionLoading = actionLoadingId === rowId;
      const isRowBusy = rowActionBusy[rowId] ?? false;
      const isManualImportPending = row.displayStateKey.toLowerCase() === "importing";
      const isDeletePending = row.displayStateKey.toLowerCase() === "removing";
      const isRowBlocked =
        isRowBusy || isManualImportPending || isDeletePending || isActionLoading;
      const isDeleteConfirming =
        deleteConfirmItem !== null &&
        downloadQueueItemIdentityKey(deleteConfirmItem) === rowId;
      const isRowFullyBusy = isRowBlocked || isDeleteConfirming;
      const isExpanded = Boolean(expandedItemIds[rowId]);
      const detailId = `activity-queue-details-${rowId}`;
      const rowActionVisualClass = isRowFullyBusy
        ? "pointer-events-none opacity-45 grayscale"
        : "";
      const isImportSelected = Boolean(selectedImportItemKeys[rowId]);

      return {
        queueItem,
        row,
        activeTab,
        rowId,
        rowSelectorKey,
        detailId,
        isActionLoading,
        isRowBlocked,
        isRowFullyBusy,
        isManualImportPending,
        isExpanded,
        isImportSelected,
        rowActionVisualClass,
        t,
        onToggleImportSelected: () => toggleImportItemSelected(queueItem),
        onToggleExpanded: () => toggleExpandedDetails(rowId),
        onPause: () => {
          setActionLoadingId(rowId);
          setRowBusy(rowId, true);
          void requestPause(queueItem).finally(() => {
            setRowBusy(rowId, false);
            setActionLoadingId((c) => (c === rowId ? null : c));
          });
        },
        onResume: () => {
          setActionLoadingId(rowId);
          setRowBusy(rowId, true);
          void requestResume(queueItem).finally(() => {
            setRowBusy(rowId, false);
            setActionLoadingId((c) => (c === rowId ? null : c));
          });
        },
        onManualImport: () => {
          setRowBusy(rowId, true);
          void requestManualImport(queueItem).finally(() => {
            setRowBusy(rowId, false);
          });
        },
        onAssignTitle: () => {
          setActionLoadingId(rowId);
          setRowBusy(rowId, true);
          void requestAssignTitle(queueItem).finally(() => {
            setRowBusy(rowId, false);
            setActionLoadingId((current) => (current === rowId ? null : current));
          });
        },
        onIgnore: () => {
          setActionLoadingId(rowId);
          setRowBusy(rowId, true);
          void requestIgnore(queueItem).finally(() => {
            setRowBusy(rowId, false);
            setActionLoadingId((current) => (current === rowId ? null : current));
          });
        },
        onMarkFailedSearchAgain: () => {
          setActionLoadingId(rowId);
          setRowBusy(rowId, true);
          void requestMarkFailed(queueItem, false).finally(() => {
            setRowBusy(rowId, false);
            setActionLoadingId((current) => (current === rowId ? null : current));
          });
        },
        onMarkFailedOnly: () => {
          setActionLoadingId(rowId);
          setRowBusy(rowId, true);
          void requestMarkFailed(queueItem, true).finally(() => {
            setRowBusy(rowId, false);
            setActionLoadingId((current) => (current === rowId ? null : current));
          });
        },
        onRequestDelete: () => {
          setRowBusy(rowId, true);
          setDeleteConfirmItem(queueItem);
        },
      };
    },
    [
      activeTab,
      actionLoadingId,
      deleteConfirmItem,
      expandedItemIds,
      requestAssignTitle,
      requestIgnore,
      requestManualImport,
      requestMarkFailed,
      requestPause,
      requestResume,
      rowActionBusy,
      selectedImportItemKeys,
      setRowBusy,
      t,
      toggleExpandedDetails,
      toggleImportItemSelected,
    ],
  );

  const renderMobileQueueCards = (
    items: DownloadQueueItem[],
    showHistorySpinner = false,
  ) => (
    <div className="space-y-3">
      {items.map((queueItem) => {
        const rowProps = buildQueueRowProps(queueItem);
        return <QueueRowItem key={rowProps.rowId} {...rowProps} />;
      })}
      {showHistorySpinner ? (
        <div className="flex items-center justify-center py-3 text-sm text-muted-foreground">
          <Loader2 className="mr-2 h-4 w-4 animate-spin" />
          {t("label.loading")}
        </div>
      ) : null}
    </div>
  );

  const renderDesktopQueueRows = (
    items: DownloadQueueItem[],
    showHistorySpinner = false,
  ) => (
    <>
      {items.map((queueItem) => {
        const rowProps = buildQueueRowProps(queueItem);
        return <QueueTableRow key={rowProps.rowId} {...rowProps} />;
      })}
      {showHistorySpinner ? (
        <TableRow>
          <TableCell
            colSpan={activeTab === "activity" ? 6 : activeTab === "import" ? 7 : 5}
            className="py-4 text-center text-sm text-muted-foreground"
          >
            <span className="inline-flex items-center">
              <Loader2 className="mr-2 h-4 w-4 animate-spin" />
              {t("label.loading")}
            </span>
          </TableCell>
        </TableRow>
      ) : null}
    </>
  );
  const activeActivityLabel =
    activeTab === "import"
      ? t("activity.import")
      : activeTab === "history"
        ? t("activity.history")
        : t("activity.activity");

  return (
    <>
      <ConfirmDialog
        open={deleteConfirmItem !== null}
        title={t("queue.deleteConfirmTitle")}
        description={t("queue.deleteConfirmDescription")}
        confirmLabel={t("label.delete")}
        cancelLabel={t("label.cancel")}
        isBusy={deleteInProgress}
        onConfirm={handleDelete}
        onCancel={() => {
          if (deleteConfirmItem) {
            setRowBusy(downloadQueueItemIdentityKey(deleteConfirmItem), false);
          }
          setDeleteConfirmItem(null);
        }}
      />
      <ConfirmDialog
        open={bulkDeleteConfirmItems.length > 0}
        title={t("queue.bulkDeleteConfirmTitle")}
        description={t("queue.bulkDeleteConfirmDescription", {
          count: bulkDeleteConfirmItems.length,
        })}
        confirmLabel={t("label.delete")}
        cancelLabel={t("label.cancel")}
        isBusy={deleteInProgress}
        onConfirm={handleBulkDelete}
        onCancel={() => {
          setBulkDeleteConfirmItems([]);
        }}
      />
      <div className="min-w-0 flex-1 overflow-y-auto bg-transparent">
        <div className="mx-auto flex min-h-0 w-full max-w-none flex-1 flex-col px-4 py-5 sm:px-6 md:px-[30px] md:py-[26px] md:pb-[60px]">
          <div className="mb-4 flex items-center gap-1.5 text-[12.5px] text-[var(--scry-faint)]">
            <span>{t("nav.group.automation")}</span>
            <ChevronRight className="h-3.5 w-3.5" />
            <span className="font-semibold text-[var(--scry-accent-text)]">
              {activeActivityLabel}
            </span>
          </div>
          <div className="mb-3 flex flex-wrap items-start justify-between gap-3">
            <div className="flex min-w-0 items-center gap-4">
              <div className="flex h-[46px] w-[46px] shrink-0 items-center justify-center rounded-[13px] border border-[var(--scry-baccent)] bg-[linear-gradient(135deg,rgba(var(--scry-accent-rgb),0.35),rgba(123,91,255,0.22))] text-[var(--scry-accent-text)]">
                <ActivitySquare className="h-[23px] w-[23px]" />
              </div>
              <div className="min-w-0">
                <h1 className="text-[25px] font-bold tracking-normal text-[var(--scry-ink2)]">
                  {activeActivityLabel}
                </h1>
              </div>
            </div>
          </div>
        <Card
          id={selectorId("activity-view", activeTab)}
          className="min-h-0 flex-1 rounded-none border-0 bg-transparent shadow-none"
        >
          <CardContent className="space-y-3 p-0">
          {queueError ? (
            <p className="rounded border border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] p-2 text-sm text-[var(--scry-danger-text)]">
              {queueError}
            </p>
          ) : null}
          <div
            className={cn(
              "flex flex-col gap-3 sm:flex-row sm:items-center",
              activeTab === "import" && selectedImportCount > 0
                ? "sm:justify-between"
                : "sm:justify-end",
            )}
          >
            {activeTab === "import" && selectedImportCount > 0 ? (
              <div className="flex flex-wrap items-center gap-2 rounded-lg border border-border/70 bg-card/60 px-3 py-2">
                <span className="text-sm text-muted-foreground">
                  {t("activity.selectedImportCount", { count: selectedImportCount })}
                </span>
                <Button
                  type="button"
                  size="sm"
                  variant="secondary"
                  disabled={bulkActionInProgress !== null || selectedIgnoreItems.length === 0}
                  onClick={() => {
                    void handleBulkIgnore(selectedIgnoreItems);
                  }}
                >
                  {bulkActionInProgress === "ignore" ? (
                    <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                  ) : (
                    <CircleOff className="mr-2 h-4 w-4" />
                  )}
                  {t("queue.ignore")}
                </Button>
                <Button
                  type="button"
                  size="sm"
                  variant="destructive"
                  disabled={bulkActionInProgress !== null || selectedDeleteItems.length === 0}
                  onClick={() => {
                    setBulkDeleteConfirmItems(selectedDeleteItems);
                  }}
                >
                  {bulkActionInProgress === "delete" ? (
                    <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                  ) : (
                    <Trash2 className="mr-2 h-4 w-4" />
                  )}
                  {t("label.delete")}
                </Button>
              </div>
            ) : null}
            <Popover open={filterPopoverOpen} onOpenChange={setFilterPopoverOpen}>
              <PopoverTrigger asChild>
                <Button
                  id={selectorId("activity", activeTab, "filter-button")}
                  type="button"
                  variant="outline"
                  size="sm"
                  className="inline-flex items-center gap-2"
                  aria-label={t("activity.filterBarLabel")}
                >
                  <Filter className="h-4 w-4" />
                  <span>{t("label.filters")}</span>
                </Button>
              </PopoverTrigger>
              <PopoverContent align="end" className="w-72 p-4">
                {renderFilterPopoverContent()}
              </PopoverContent>
            </Popover>
          </div>

          {isMobile ? (
            sortedQueueItems.length === 0 && !queueLoading ? (
              <p className="text-sm text-muted-foreground">{emptyStateLabel}</p>
            ) : sortedQueueItems.length === 0 ? (
              <div className={`${scrollHeightClass} overflow-y-auto pr-1`}>
                <div className="rounded-xl border border-border/60 bg-card/30">
                  <ActivityTableLoadingMask label={t("label.loading")} />
                </div>
              </div>
            ) : (
                <div
                  onScroll={handleResultsScroll}
                  className={`${scrollHeightClass} overflow-y-auto pr-1`}
                >
                  {renderMobileQueueCards(sortedQueueItems, queueLoadingMore)}
                </div>
              )
          ) : (
            <div
              onScroll={handleResultsScroll}
              className={`${scrollHeightClass} overflow-y-auto rounded-xl border border-border/60`}
            >
              <Table overflow="clip" layout="fixed" density="dense">
                <TableHeader>
                  <TableRow>
                    {activeTab === "import" ? (
                      <TableCheckboxHead>
                        <Checkbox
                          checked={
                            allVisibleImportItemsSelected
                              ? true
                              : someVisibleImportItemsSelected
                                ? "indeterminate"
                                : false
                          }
                          disabled={visibleImportKeys.length === 0}
                          aria-label={t("activity.selectAllImportItems")}
                          onCheckedChange={toggleAllVisibleImportItemsSelected}
                          size="table"
                          className="mx-auto"
                        />
                      </TableCheckboxHead>
                    ) : null}
                    {renderSortableHeader(
                      "TITLE",
                      t("queue.title"),
                      "w-[32%]",
                    )}
                    {renderSortableHeader(
                      "CLIENT",
                      t("queue.client"),
                      "w-[13%]",
                    )}
                    {renderSortableHeader(
                      "STATUS",
                      t("queue.status"),
                      "w-[15%]",
                    )}
                    {activeTab === "activity" || activeTab === "import"
                      ? renderSortableHeader(
                          "PROGRESS",
                          t("queue.progress"),
                          "w-[16%]",
                        )
                      : null}
                    {renderSortableHeader(
                      "SIZE",
                      t("queue.size"),
                      "w-28 text-center [&_button]:justify-center [&_button]:text-center",
                    )}
                    <TableHead className="w-52 text-center">
                      {t("label.actions")}
                    </TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {sortedQueueItems.length === 0 ? (
                    <TableRow>
                      <TableCell
                        colSpan={
                          activeTab === "activity"
                            ? 6
                            : activeTab === "import"
                              ? 7
                              : 5
                        }
                        className={
                          queueLoading
                            ? "p-0"
                            : "text-sm text-muted-foreground"
                        }
                      >
                        {queueLoading ? (
                          <ActivityTableLoadingMask label={t("label.loading")} />
                        ) : (
                          emptyStateLabel
                        )}
                      </TableCell>
                    </TableRow>
                  ) : (
                    renderDesktopQueueRows(sortedQueueItems, queueLoadingMore)
                  )}
                </TableBody>
              </Table>
            </div>
          )}
          {activeTab === "history" && historyTotalPages > 1 ? (
            <div className="flex items-center justify-end gap-2 border-t border-border/60 pt-3">
              <Button
                type="button"
                size="sm"
                variant="secondary"
                disabled={!historyHasPreviousPage || queueLoading}
                onClick={() => {
                  void goToPreviousHistoryPage();
                }}
              >
                <ChevronLeft className="h-4 w-4" />
                {t("wanted.prev")}
              </Button>
              <span className="min-w-16 text-center text-xs text-muted-foreground">
                {historyPage} / {historyTotalPages}
              </span>
              <Button
                type="button"
                size="sm"
                variant="secondary"
                disabled={!historyHasNextPage || queueLoading}
                onClick={() => {
                  void goToNextHistoryPage();
                }}
              >
                {t("wanted.next")}
                <ChevronRight className="h-4 w-4" />
              </Button>
            </div>
          ) : null}
          </CardContent>
        </Card>
        </div>
      </div>
    </>
  );
}
