
import {
  ArrowDown,
  ArrowDownToLine,
  ArrowUp,
  CheckCircle2,
  ChevronLeft,
  ChevronRight,
  ChevronDown,
  ChevronUp,
  CircleOff,
  CircleAlert,
  Clock3,
  Filter,
  HardDrive,
  Link2,
  Loader2,
  Pause,
  Play,
  Trash2,
  XCircle,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";
import {
  Fragment,
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
import { ActivityProgressBar } from "@/components/views/activity-progress-bar";
import {
  Table,
  TableBody,
  TableCell,
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
import type { ActivitySection } from "@/components/root/types";
import { useTranslate } from "@/lib/context/translate-context";
import { useIsMobile } from "@/lib/hooks/use-mobile";
import { cn } from "@/lib/utils";
import {
  buildQueueStatusDetail,
  downloadQueueItemIdentityKey,
  normalizeQueueState,
} from "@/lib/utils/download-queue";

type TranslateFn = ReturnType<typeof useTranslate>;

type ActivityTab = ActivitySection;

type ActivityViewState = {
  queueItems: DownloadQueueItem[];
  queueLoading: boolean;
  queueLoadingMore: boolean;
  queueError: string | null;
  requestManualImport: (item: DownloadQueueItem) => Promise<void>;
  requestAssignTitle: (item: DownloadQueueItem) => Promise<void>;
  requestIgnore: (item: DownloadQueueItem) => Promise<void>;
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

const queueStateClasses: Record<string, string> = {
  queued: "border-amber-500/40 bg-amber-500/10 text-amber-200",
  downloading: "border-sky-500/40 bg-sky-500/10 text-sky-200",
  post_processing: "border-cyan-500/40 bg-cyan-500/10 text-cyan-200",
  paused: "border-purple-500/40 bg-purple-500/10 text-purple-200",
  completed: "border-emerald-500/40 bg-emerald-500/15 dark:bg-emerald-500/10 text-emerald-700 dark:text-emerald-200",
  importing: "border-sky-500/40 bg-sky-500/10 text-sky-200",
  removing: "border-sky-500/40 bg-sky-500/10 text-sky-200",
  import_pending: "border-indigo-500/40 bg-indigo-500/10 text-indigo-200",
  import_blocked: "border-amber-500/40 bg-amber-500/10 text-amber-200",
  import_failed: "border-rose-500/40 bg-rose-500/10 text-rose-200",
  remove_failed: "border-rose-500/40 bg-rose-500/10 text-rose-200",
  failed: "border-rose-500/40 bg-rose-500/10 text-rose-200",
};

const queueStateLabels: Record<string, string> = {
  queued: "queue.state.queued",
  downloading: "queue.state.downloading",
  post_processing: "queue.state.postProcessing",
  paused: "queue.state.paused",
  completed: "queue.state.completed",
  importing: "queue.manualImporting",
  removing: "queue.deleting",
  import_pending: "queue.state.importPending",
  import_blocked: "queue.state.importBlocked",
  import_failed: "queue.manualImportFailed",
  remove_failed: "queue.removeFailed",
  failed: "queue.state.failed",
};

const queueStateAttention: Record<string, boolean> = {
  failed: true,
  importing: true,
  removing: true,
  import_pending: true,
  import_blocked: true,
  import_failed: true,
  remove_failed: true,
};

type ActivityFilterChipOption<T extends string> = {
  value: T;
  labelKey: string;
  icon: LucideIcon;
  iconClassName?: string;
};

const importFilterOptions: ActivityFilterChipOption<DownloadImportStatus>[] = [
  {
    value: "importing",
    labelKey: "activity.importFilter.importing",
    icon: HardDrive,
    iconClassName: "text-sky-400",
  },
  {
    value: "pending",
    labelKey: "activity.importFilter.pending",
    icon: Clock3,
    iconClassName: "text-indigo-400",
  },
  {
    value: "blocked",
    labelKey: "activity.importFilter.blocked",
    icon: CircleAlert,
    iconClassName: "text-amber-400",
  },
  {
    value: "failed",
    labelKey: "activity.importFilter.failed",
    icon: XCircle,
    iconClassName: "text-rose-400",
  },
];

const activityFilterOptions: ActivityFilterChipOption<DownloadActivityStatus>[] = [
  {
    value: "downloading",
    labelKey: "activity.activityFilter.downloading",
    icon: ArrowDownToLine,
    iconClassName: "text-sky-400",
  },
  {
    value: "queued",
    labelKey: "activity.activityFilter.queued",
    icon: Clock3,
    iconClassName: "text-amber-400",
  },
  {
    value: "paused",
    labelKey: "activity.activityFilter.paused",
    icon: Pause,
    iconClassName: "text-purple-400",
  },
  {
    value: "post_processing",
    labelKey: "activity.activityFilter.postProcessing",
    icon: HardDrive,
    iconClassName: "text-cyan-400",
  },
];

const historyFilterOptions: ActivityFilterChipOption<DownloadHistoryStatus>[] = [
  {
    value: "success",
    labelKey: "activity.historyFilter.success",
    icon: CheckCircle2,
    iconClassName: "text-emerald-400",
  },
  {
    value: "failed",
    labelKey: "activity.historyFilter.failed",
    icon: XCircle,
    iconClassName: "text-rose-400",
  },
];

function compareStrings(left: string, right: string): number {
  return left.localeCompare(right, undefined, { sensitivity: "base" });
}

function activityStatusRank(tab: ActivityTab, displayState: string): number {
  switch (tab) {
    case "import":
      switch (displayState) {
        case "importing":
          return 0;
        case "import_pending":
          return 1;
        case "import_blocked":
          return 2;
        case "import_failed":
          return 3;
        default:
          return 99;
      }
    case "history":
      switch (displayState) {
        case "completed":
          return 0;
        case "failed":
        case "remove_failed":
          return 1;
        default:
          return 99;
      }
    case "activity":
    default:
      switch (displayState) {
        case "downloading":
          return 0;
        case "queued":
          return 1;
        case "paused":
          return 2;
        case "post_processing":
          return 3;
        default:
          return 99;
      }
  }
}

type QueueRowPresentation = {
  stateKey: string;
  trackedStateKey: string;
  trackedMatchTypeKey: string;
  displayStateKey: string;
  percent: number;
  remainingLabel: string | null;
  needsManualImport: boolean;
  statusLabel: string;
  failureReason: string;
  hasStatusDetails: boolean;
  hasExpandableDetails: boolean;
  displayTitle: string;
  releaseTitle: string;
  canPause: boolean;
  canResume: boolean;
  canAssignTitle: boolean;
  canIgnore: boolean;
  canInteractiveManualImport: boolean;
  canDirectManualImport: boolean;
};

function deriveQueueRowPresentation(
  queueItem: DownloadQueueItem,
  t: TranslateFn,
): QueueRowPresentation {
  const stateKey = normalizeQueueState(queueItem.state);
  const trackedStateKey = normalizeQueueState(queueItem.trackedState);
  const trackedMatchTypeKey = normalizeQueueState(queueItem.trackedMatchType);
  const failureReason = buildQueueStatusDetail(queueItem);
  const displayStateKey = queueItem.displayState;
  const percent = formatProgress(queueItem.progressPercent);
  const remainingLabel = formatRemainingDuration(queueItem.remainingSeconds);
  const needsManualImport =
    queueItem.attentionRequired ||
    queueStateAttention[stateKey] ||
    queueStateAttention[displayStateKey];
  const stageLabel =
    queueItem.attentionReason?.trim() ??
    queueItem.trackedStatusMessages[0]?.trim() ??
    "";
  const statusLabel =
    displayStateKey === "post_processing" && stageLabel.length > 0
      ? stageLabel
      : t(queueStateLabels[displayStateKey] ?? "queue.state.unknown");
  const hasStatusDetails =
    (stateKey === "failed" ||
      displayStateKey === "remove_failed" ||
      displayStateKey === "import_blocked" ||
      displayStateKey === "import_failed") &&
    failureReason.length > 0;
  const isCompleted = stateKey === "completed" || stateKey === "import_pending";
  const canRetryManualImport =
    displayStateKey === "import_blocked" || displayStateKey === "import_failed";
  const canAssignTitle =
    trackedStateKey === "import_blocked" &&
    displayStateKey !== "importing" &&
    displayStateKey !== "removing";
  const canIgnore =
    trackedStateKey === "import_blocked" &&
    displayStateKey !== "importing" &&
    displayStateKey !== "removing";
  const canInteractiveManualImport =
    Boolean(queueItem.titleId) &&
    (queueItem.facet === "series" || queueItem.facet === "anime") &&
    canRetryManualImport;
  const canDirectManualImport =
    Boolean(queueItem.titleId) &&
    displayStateKey !== "importing" &&
    displayStateKey !== "removing" &&
    ((isCompleted && needsManualImport) ||
      (canRetryManualImport && queueItem.facet === "movie"));
  const releaseTitle =
    queueItem.titleName.trim() || queueItem.downloadClientItemId.trim() || "\u2014";
  const displayTitle = releaseTitle;
  const hasExpandableDetails =
    (displayStateKey === "import_blocked" ||
      displayStateKey === "import_failed" ||
      displayStateKey === "remove_failed") &&
    (failureReason.length > 0 || releaseTitle !== "\u2014");

  return {
    stateKey,
    trackedStateKey,
    trackedMatchTypeKey,
    displayStateKey,
    percent,
    remainingLabel,
    needsManualImport,
    statusLabel,
    failureReason,
    hasStatusDetails,
    hasExpandableDetails,
    displayTitle,
    releaseTitle,
    canPause: stateKey === "downloading" || stateKey === "queued",
    canResume: stateKey === "paused",
    canAssignTitle,
    canIgnore,
    canInteractiveManualImport,
    canDirectManualImport,
  };
}

function canIgnoreImportItem(queueItem: DownloadQueueItem): boolean {
  const trackedStateKey = normalizeQueueState(queueItem.trackedState);
  const displayStateKey = normalizeQueueState(queueItem.displayState);
  return (
    trackedStateKey === "import_blocked" &&
    displayStateKey !== "importing" &&
    displayStateKey !== "removing"
  );
}

function canDeleteImportItem(queueItem: DownloadQueueItem): boolean {
  const displayStateKey = normalizeQueueState(queueItem.displayState);
  return displayStateKey !== "importing" && displayStateKey !== "removing";
}

function ActivityQueueStatusBadge({
  stateKey,
  statusLabel,
  isExpandable,
  isExpanded,
  detailId,
  expandLabel,
  onToggle,
}: {
  stateKey: string;
  statusLabel: string;
  isExpandable: boolean;
  isExpanded: boolean;
  detailId: string;
  expandLabel: string;
  onToggle: () => void;
}) {
  const className = `inline-flex items-center gap-1.5 rounded border px-2 py-1 text-xs font-medium ${queueStateClasses[stateKey] ?? "border-border bg-muted text-card-foreground"}`;

  if (!isExpandable) {
    return <span className={className}>{statusLabel}</span>;
  }

  return (
    <button
      type="button"
      className={className}
      aria-expanded={isExpanded}
      aria-controls={detailId}
      aria-label={`${statusLabel}. ${expandLabel}`}
      onClick={onToggle}
    >
      <span>{statusLabel}</span>
      {isExpanded ? (
        <ChevronUp className="h-3.5 w-3.5 opacity-80" aria-hidden="true" />
      ) : (
        <ChevronDown className="h-3.5 w-3.5 opacity-80" aria-hidden="true" />
      )}
    </button>
  );
}

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
              <Checkbox checked={isSelected} onCheckedChange={() => onToggle(option.value)} />
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
              <Checkbox checked={isSelected} onCheckedChange={() => onToggle(option.clientId)} />
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
        <Checkbox checked={checked} onCheckedChange={onToggle} />
        <span>{label}</span>
      </label>
    </div>
  );
}

function ActivityQueueTitleContent({
  displayTitle,
  releaseTitle,
}: {
  displayTitle: string;
  releaseTitle: string;
}) {
  return (
    <div className="space-y-1">
      <p className="break-words whitespace-normal text-sm text-foreground">{displayTitle}</p>
      {releaseTitle !== displayTitle ? (
        <p
          className="break-words whitespace-normal text-xs text-muted-foreground"
          title={releaseTitle}
        >
          {releaseTitle}
        </p>
      ) : null}
    </div>
  );
}

function ActivityQueueDetailsPanel({
  detailId,
  releaseTitle,
  errorCode,
  failureReason,
  t,
}: {
  detailId: string;
  releaseTitle: string;
  errorCode?: string | null;
  failureReason: string;
  t: TranslateFn;
}) {
  return (
    <div
      id={detailId}
      className="rounded-lg border border-amber-500/25 bg-amber-500/5 p-3"
    >
      <div className="grid gap-4 md:grid-cols-2">
        <div>
          <p className="text-[11px] font-semibold uppercase tracking-[0.16em] text-muted-foreground">
            {t("queue.releaseTitle")}
          </p>
          <p className="mt-1 break-words text-sm text-foreground">{releaseTitle}</p>
        </div>
        <div>
          {errorCode ? (
            <>
              <p className="text-[11px] font-semibold uppercase tracking-[0.16em] text-muted-foreground">
                {t("queue.errorCode")}
              </p>
              <p className="mt-1 break-words text-sm font-mono text-foreground">{errorCode}</p>
            </>
          ) : null}
        </div>
      </div>
      <div className="mt-4">
        <div>
          <p className="text-[11px] font-semibold uppercase tracking-[0.16em] text-muted-foreground">
            {t("queue.blockReason")}
          </p>
          <p className="mt-1 whitespace-pre-wrap break-words text-sm text-foreground">
            {failureReason || "\u2014"}
          </p>
        </div>
      </div>
    </div>
  );
}

function formatBytes(sizeBytes: string | null): string {
  if (!sizeBytes) {
    return "\u2014";
  }
  const bytes = Number.parseFloat(sizeBytes);
  if (!Number.isFinite(bytes) || bytes < 0) {
    return "\u2014";
  }
  if (bytes === 0) {
    return "0 B";
  }
  const units = ["B", "KB", "MB", "GB", "TB", "PB"];
  let value = bytes;
  let index = 0;
  while (value >= 1024 && index < units.length - 1) {
    value /= 1024;
    index += 1;
  }
  return `${value.toFixed(value >= 10 || index === 0 ? 0 : 1)} ${units[index]}`;
}

function formatProgress(progressPercent: number): number {
  if (!Number.isFinite(progressPercent)) {
    return 0;
  }
  if (progressPercent < 0) {
    return 0;
  }
  if (progressPercent > 100) {
    return 100;
  }
  return Math.round(progressPercent);
}

function formatRemainingDuration(remainingSeconds: number | null): string | null {
  if (remainingSeconds === null || !Number.isFinite(remainingSeconds)) {
    return null;
  }
  const totalSeconds = Math.max(0, Math.floor(remainingSeconds));
  const hours = Math.floor(totalSeconds / 3600);
  const minutes = Math.floor((totalSeconds % 3600) / 60);
  const seconds = totalSeconds % 60;
  if (hours > 0) {
    return `${hours}:${minutes.toString().padStart(2, "0")}:${seconds
      .toString()
      .padStart(2, "0")}`;
  }
  return `${minutes}:${seconds.toString().padStart(2, "0")}`;
}

function getProgressBarColor(stateKey: string): string {
  switch (stateKey) {
    case "completed":
      return "bg-emerald-500";
    case "failed":
    case "remove_failed":
      return "bg-rose-500";
    case "paused":
      return "bg-amber-500";
    case "import_pending":
      return "bg-indigo-500";
    case "downloading":
    case "removing":
      return "bg-sky-500";
    case "post_processing":
      return "bg-cyan-500";
    case "queued":
      return "bg-gray-400";
    default:
      return "bg-muted-foreground";
  }
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

      return activeSortConfig.direction === "asc" ? (
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
            ? activeSortConfig.direction === "asc"
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

    const directionMultiplier = activeSortConfig.direction === "asc" ? 1 : -1;
    const items = [...queueItems];

    items.sort((leftItem, rightItem) => {
      let comparison = 0;

      switch (activeSortConfig.key) {
        case "title": {
          const leftTitle = leftItem.titleName.trim() || leftItem.downloadClientItemId.trim();
          const rightTitle = rightItem.titleName.trim() || rightItem.downloadClientItemId.trim();
          comparison = compareStrings(leftTitle, rightTitle);
          break;
        }
        case "client": {
          const leftClient = leftItem.clientName.trim() || leftItem.clientType.trim();
          const rightClient = rightItem.clientName.trim() || rightItem.clientType.trim();
          comparison = compareStrings(leftClient, rightClient);
          if (comparison === 0) {
            comparison = compareStrings(leftItem.clientType, rightItem.clientType);
          }
          break;
        }
        case "status": {
          comparison =
            activityStatusRank(activeTab, leftItem.displayState) -
            activityStatusRank(activeTab, rightItem.displayState);
          if (comparison === 0) {
            const leftStatus = t(queueStateLabels[leftItem.displayState] ?? "queue.state.unknown");
            const rightStatus = t(
              queueStateLabels[rightItem.displayState] ?? "queue.state.unknown",
            );
            comparison = compareStrings(leftStatus, rightStatus);
          }
          break;
        }
        case "progress": {
          comparison = formatProgress(leftItem.progressPercent) - formatProgress(rightItem.progressPercent);
          break;
        }
        case "size": {
          const leftSize = Number.parseFloat(leftItem.sizeBytes ?? "0");
          const rightSize = Number.parseFloat(rightItem.sizeBytes ?? "0");
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
          !rowActionBusyRef.current[rowId] &&
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
          !rowActionBusyRef.current[rowId] &&
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

  const renderMobileQueueCards = (
    items: DownloadQueueItem[],
    showHistorySpinner = false,
  ) => (
    <div className="space-y-3">
      {items.map((queueItem) => {
        const rowId = downloadQueueItemIdentityKey(queueItem);
        const row = deriveQueueRowPresentation(queueItem, t);
        const isActionLoading = actionLoadingId === rowId;
        const isRowBusy =
          rowActionBusy[rowId] ?? rowActionBusyRef.current[rowId] ?? false;
        const isManualImportPending = row.displayStateKey === "importing";
        const isDeletePending = row.displayStateKey === "removing";
        const isRowBlocked =
          isRowBusy || isManualImportPending || isDeletePending || isActionLoading;
        const isDeleteConfirming =
          deleteConfirmItem !== null && downloadQueueItemIdentityKey(deleteConfirmItem) === rowId;
        const isRowFullyBusy = isRowBlocked || isDeleteConfirming;
        const isExpanded = Boolean(expandedItemIds[rowId]);
        const detailId = `activity-queue-details-${rowId}`;
        const rowActionVisualClass = isRowFullyBusy
          ? "pointer-events-none opacity-45 grayscale"
          : "";
        const isImportSelected = Boolean(selectedImportItemKeys[rowId]);

        return (
          <div key={rowId} className="rounded-xl border border-border bg-card/40 p-3">
            <div className="flex items-start justify-between gap-3">
              <div className="flex min-w-0 flex-1 items-start gap-3">
                {activeTab === "import" ? (
                  <Checkbox
                    checked={isImportSelected}
                    aria-label={t("activity.selectImportItem")}
                    className="mt-0.5"
                    onCheckedChange={() => toggleImportItemSelected(queueItem)}
                  />
                ) : null}
                <div className="min-w-0 flex-1">
                  <ActivityQueueTitleContent
                    displayTitle={row.displayTitle}
                    releaseTitle={row.releaseTitle}
                  />
                  <p className="mt-1 text-xs text-muted-foreground">
                    {queueItem.clientName || queueItem.clientType} • {queueItem.clientType}
                  </p>
                </div>
              </div>
              <div className="shrink-0">
                <ActivityQueueStatusBadge
                  stateKey={row.displayStateKey}
                  statusLabel={row.statusLabel}
                  isExpandable={row.hasExpandableDetails}
                  isExpanded={isExpanded}
                  detailId={detailId}
                  expandLabel={t(
                    isExpanded ? "queue.hideDetails" : "queue.showDetails",
                  )}
                  onToggle={() => toggleExpandedDetails(rowId)}
                />
              </div>
            </div>
            {(queueItem.deleteErrorMessage || queueItem.importErrorMessage) &&
            !row.hasStatusDetails ? (
              <p className="mt-2 break-words text-xs text-rose-400">
                {queueItem.deleteErrorMessage ?? queueItem.importErrorMessage}
              </p>
            ) : null}
            {row.hasExpandableDetails && isExpanded ? (
              <div className="mt-3">
                <ActivityQueueDetailsPanel
                  detailId={detailId}
                  releaseTitle={row.releaseTitle}
                  errorCode={queueItem.importErrorCode}
                  failureReason={row.failureReason}
                  t={t}
                />
              </div>
            ) : null}
            <div className="mt-3">
              <ActivityProgressBar
                percent={row.percent}
                remainingLabel={row.remainingLabel}
                colorClass={getProgressBarColor(row.displayStateKey)}
              />
            </div>
            <div className="mt-3 flex items-center justify-between text-xs text-muted-foreground">
              <span>{formatBytes(queueItem.sizeBytes)}</span>
            </div>
            <div className="mt-3 flex flex-wrap gap-2">
              {row.canPause && (
                <Button
                  type="button"
                  size="sm"
                  variant="secondary"
                  className={`flex-1 ${rowActionVisualClass}`}
                  disabled={isRowFullyBusy}
                  onClick={() => {
                    if (
                      rowActionBusyRef.current[rowId] ||
                      isActionLoading ||
                      isRowBlocked
                    ) {
                      return;
                    }
                    setActionLoadingId(rowId);
                    setRowBusy(rowId, true);
                    void requestPause(queueItem).finally(() => {
                      setRowBusy(rowId, false);
                      setActionLoadingId((c) => (c === rowId ? null : c));
                    });
                  }}
                >
                  <Pause className="h-4 w-4" />
                  <span>{t("queue.pause")}</span>
                </Button>
              )}
              {row.canResume && (
                <Button
                  type="button"
                  size="sm"
                  variant="secondary"
                  className={`flex-1 ${rowActionVisualClass}`}
                  disabled={isRowFullyBusy}
                  onClick={() => {
                    if (
                      rowActionBusyRef.current[rowId] ||
                      isActionLoading ||
                      isRowBlocked
                    ) {
                      return;
                    }
                    setActionLoadingId(rowId);
                    setRowBusy(rowId, true);
                    void requestResume(queueItem).finally(() => {
                      setRowBusy(rowId, false);
                      setActionLoadingId((c) => (c === rowId ? null : c));
                    });
                  }}
                >
                  <Play className="h-4 w-4" />
                  <span>{t("queue.resume")}</span>
                </Button>
              )}
              {(row.canInteractiveManualImport || row.canDirectManualImport) && (
                <Button
                  type="button"
                  size="sm"
                  variant="secondary"
                  className={`flex-1 ${rowActionVisualClass}`}
                  disabled={isRowFullyBusy}
                  onClick={() => {
                    if (
                      rowActionBusyRef.current[rowId] ||
                      isActionLoading ||
                      isRowBlocked
                    ) {
                      return;
                    }
                    setRowBusy(rowId, true);
                    void requestManualImport(queueItem).finally(() => {
                      setRowBusy(rowId, false);
                    });
                  }}
                >
                  {isManualImportPending ? (
                    <Loader2 className="h-4 w-4 animate-spin" />
                  ) : (
                    <ArrowDownToLine className="h-4 w-4" />
                  )}
                  <span>
                    {isManualImportPending
                      ? t("queue.manualImporting")
                      : t("queue.manualImportTooltip")}
                  </span>
                </Button>
              )}
              {row.canAssignTitle && (
                <Button
                  type="button"
                  size="sm"
                  variant="secondary"
                  className={`flex-1 ${rowActionVisualClass}`}
                  disabled={isRowFullyBusy}
                  onClick={() => {
                    if (
                      rowActionBusyRef.current[rowId] ||
                      isActionLoading ||
                      isRowBlocked
                    ) {
                      return;
                    }
                    setActionLoadingId(rowId);
                    setRowBusy(rowId, true);
                    void requestAssignTitle(queueItem).finally(() => {
                      setRowBusy(rowId, false);
                      setActionLoadingId((current) =>
                        current === rowId ? null : current,
                      );
                    });
                  }}
                >
                  <Link2 className="h-4 w-4" />
                  <span>
                    {row.trackedMatchTypeKey === "unmatched" || !queueItem.titleId
                      ? t("queue.assignTitle")
                      : t("queue.reassignTitle")}
                  </span>
                </Button>
              )}
              {row.canIgnore && (
                <Button
                  type="button"
                  size="sm"
                  variant="secondary"
                  className={`flex-1 ${rowActionVisualClass}`}
                  disabled={isRowFullyBusy}
                  onClick={() => {
                    if (
                      rowActionBusyRef.current[rowId] ||
                      isActionLoading ||
                      isRowBlocked
                    ) {
                      return;
                    }
                    setActionLoadingId(rowId);
                    setRowBusy(rowId, true);
                    void requestIgnore(queueItem).finally(() => {
                      setRowBusy(rowId, false);
                      setActionLoadingId((current) =>
                        current === rowId ? null : current,
                      );
                    });
                  }}
                >
                  <CircleOff className="h-4 w-4" />
                  <span>{t("queue.ignore")}</span>
                </Button>
              )}
              <Button
                type="button"
                size="sm"
                variant="destructive"
                className={`flex-1 ${rowActionVisualClass}`}
                disabled={isRowFullyBusy}
                onClick={() => {
                  if (
                    rowActionBusyRef.current[rowId] ||
                    isActionLoading ||
                    isRowBlocked
                  ) {
                    return;
                  }
                  setRowBusy(rowId, true);
                  setDeleteConfirmItem(queueItem);
                }}
              >
                <Trash2 className="h-4 w-4" />
                <span>{t("label.delete")}</span>
              </Button>
            </div>
          </div>
        );
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
        const rowId = downloadQueueItemIdentityKey(queueItem);
        const row = deriveQueueRowPresentation(queueItem, t);
        const isActionLoading = actionLoadingId === rowId;
        const isRowBusy =
          rowActionBusy[rowId] ??
          rowActionBusyRef.current[rowId] ??
          false;
        const isManualImportPending = row.displayStateKey === "importing";
        const isDeletePending = row.displayStateKey === "removing";
        const isRowBlocked =
          isRowBusy || isManualImportPending || isDeletePending || isActionLoading;
        const isDeleteConfirming =
          deleteConfirmItem !== null && downloadQueueItemIdentityKey(deleteConfirmItem) === rowId;
        const isRowFullyBusy = isRowBlocked || isDeleteConfirming;
        const rowActionVisualClass = isRowFullyBusy
          ? "pointer-events-none opacity-45 grayscale"
          : "";
        const isExpanded = Boolean(expandedItemIds[rowId]);
        const detailId = `activity-queue-details-${rowId}`;

        return (
          <Fragment key={rowId}>
            <TableRow>
              {activeTab === "import" ? (
                <TableCell className="w-12 min-w-12 align-middle">
                  <Checkbox
                    checked={Boolean(selectedImportItemKeys[rowId])}
                    aria-label={t("activity.selectImportItem")}
                    onCheckedChange={() => toggleImportItemSelected(queueItem)}
                  />
                </TableCell>
              ) : null}
              <TableCell className="min-w-0">
                <ActivityQueueTitleContent
                  displayTitle={row.displayTitle}
                  releaseTitle={row.releaseTitle}
                />
              </TableCell>
              <TableCell className="min-w-0 align-middle">
                <p className="break-words whitespace-normal text-sm">
                  {queueItem.clientName || queueItem.clientType}
                </p>
                <p className="text-xs text-muted-foreground">{queueItem.clientType}</p>
              </TableCell>
              <TableCell className="min-w-0 align-middle">
                <ActivityQueueStatusBadge
                  stateKey={row.displayStateKey}
                  statusLabel={row.statusLabel}
                  isExpandable={row.hasExpandableDetails}
                  isExpanded={isExpanded}
                  detailId={detailId}
                  expandLabel={t(
                    isExpanded ? "queue.hideDetails" : "queue.showDetails",
                  )}
                  onToggle={() => toggleExpandedDetails(rowId)}
                />
                {(queueItem.deleteErrorMessage || queueItem.importErrorMessage) &&
                  !row.hasStatusDetails && (
                  <p
                    className="mt-1 max-w-full break-words whitespace-normal text-xs text-rose-400"
                    title={queueItem.deleteErrorMessage ?? queueItem.importErrorMessage ?? ""}
                  >
                    {queueItem.deleteErrorMessage ?? queueItem.importErrorMessage}
                  </p>
                )}
              </TableCell>
              {activeTab === "activity" ? (
                <TableCell className="w-52 min-w-52 align-middle">
                  <ActivityProgressBar
                    percent={row.percent}
                    remainingLabel={row.remainingLabel}
                    colorClass={getProgressBarColor(row.displayStateKey)}
                  />
                </TableCell>
              ) : null}
              <TableCell className="w-24 min-w-24 align-middle">
                {formatBytes(queueItem.sizeBytes)}
              </TableCell>
              <TableCell className="w-44 min-w-44 align-middle text-right">
                <div className="flex items-center justify-end gap-2">
                  {row.canPause && (
                    <Button
                      type="button"
                      size="sm"
                      variant="secondary"
                      className={`h-10 w-10 border border-border/50 bg-muted/70 text-foreground hover:bg-accent/90 ${rowActionVisualClass}`}
                      disabled={isRowFullyBusy}
                      title={t("queue.pause")}
                      aria-label={t("queue.pause")}
                      onClick={() => {
                        if (
                          rowActionBusyRef.current[rowId] ||
                          isActionLoading ||
                          isRowBlocked
                        ) {
                          return;
                        }
                        setActionLoadingId(rowId);
                        setRowBusy(rowId, true);
                        void requestPause(queueItem).finally(() => {
                          setRowBusy(rowId, false);
                          setActionLoadingId((c) => (c === rowId ? null : c));
                        });
                      }}
                    >
                      <Pause className="h-6 w-6" />
                    </Button>
                  )}
                  {row.canResume && (
                    <Button
                      type="button"
                      size="sm"
                      variant="secondary"
                      className={`h-10 w-10 border border-border/50 bg-muted/70 text-foreground hover:bg-accent/90 ${rowActionVisualClass}`}
                      disabled={isRowFullyBusy}
                      title={t("queue.resume")}
                      aria-label={t("queue.resume")}
                      onClick={() => {
                        if (
                          rowActionBusyRef.current[rowId] ||
                          isActionLoading ||
                          isRowBlocked
                        ) {
                          return;
                        }
                        setActionLoadingId(rowId);
                        setRowBusy(rowId, true);
                        void requestResume(queueItem).finally(() => {
                          setRowBusy(rowId, false);
                          setActionLoadingId((c) => (c === rowId ? null : c));
                        });
                      }}
                    >
                      <Play className="h-6 w-6" />
                    </Button>
                  )}
                  {(row.canInteractiveManualImport || row.canDirectManualImport) && (
                    <Button
                      type="button"
                      size="sm"
                      variant="secondary"
                      className={`h-10 w-10 border border-emerald-500/60 dark:border-emerald-500/50 bg-emerald-600/20 dark:bg-emerald-600/15 text-emerald-700 dark:text-emerald-200 hover:bg-emerald-600/30 dark:hover:bg-emerald-600/25 ${rowActionVisualClass}`}
                      disabled={isRowFullyBusy}
                      title={
                        isManualImportPending
                          ? t("queue.manualImporting")
                          : t("queue.manualImportTooltip")
                      }
                      aria-label={
                        isManualImportPending
                          ? t("queue.manualImporting")
                          : t("queue.manualImportTooltip")
                      }
                      onClick={() => {
                        if (
                          rowActionBusyRef.current[rowId] ||
                          isActionLoading ||
                          isRowBlocked
                        ) {
                          return;
                        }
                        setRowBusy(rowId, true);
                        void requestManualImport(queueItem).finally(() => {
                          setRowBusy(rowId, false);
                        });
                      }}
                    >
                      {isManualImportPending ? (
                        <Loader2 className="h-5 w-5 animate-spin" />
                      ) : (
                        <ArrowDownToLine className="h-5 w-5" />
                      )}
                    </Button>
                  )}
                  {row.canAssignTitle && (
                    <Button
                      type="button"
                      size="sm"
                      variant="secondary"
                      className={`h-10 w-10 border border-amber-500/60 bg-amber-600/15 text-amber-200 hover:bg-amber-600/25 ${rowActionVisualClass}`}
                      disabled={isRowFullyBusy}
                      title={
                        row.trackedMatchTypeKey === "unmatched" || !queueItem.titleId
                          ? t("queue.assignTitle")
                          : t("queue.reassignTitle")
                      }
                      aria-label={
                        row.trackedMatchTypeKey === "unmatched" || !queueItem.titleId
                          ? t("queue.assignTitle")
                          : t("queue.reassignTitle")
                      }
                      onClick={() => {
                        if (
                          rowActionBusyRef.current[rowId] ||
                          isActionLoading ||
                          isRowBlocked
                        ) {
                          return;
                        }
                        setActionLoadingId(rowId);
                        setRowBusy(rowId, true);
                        void requestAssignTitle(queueItem).finally(() => {
                          setRowBusy(rowId, false);
                          setActionLoadingId((current) =>
                            current === rowId ? null : current,
                          );
                        });
                      }}
                    >
                      <Link2 className="h-5 w-5" />
                    </Button>
                  )}
                  {row.canIgnore && (
                    <Button
                      type="button"
                      size="sm"
                      variant="secondary"
                      className={`h-10 w-10 border border-border/50 bg-muted/70 text-foreground hover:bg-accent/90 ${rowActionVisualClass}`}
                      disabled={isRowFullyBusy}
                      title={t("queue.ignore")}
                      aria-label={t("queue.ignore")}
                      onClick={() => {
                        if (
                          rowActionBusyRef.current[rowId] ||
                          isActionLoading ||
                          isRowBlocked
                        ) {
                          return;
                        }
                        setActionLoadingId(rowId);
                        setRowBusy(rowId, true);
                        void requestIgnore(queueItem).finally(() => {
                          setRowBusy(rowId, false);
                          setActionLoadingId((current) =>
                            current === rowId ? null : current,
                          );
                        });
                      }}
                    >
                      <CircleOff className="h-5 w-5" />
                    </Button>
                  )}
                  <Button
                    type="button"
                    size="sm"
                    variant="secondary"
                    className={`h-10 w-10 border border-rose-500/50 bg-rose-600/15 text-rose-300 hover:bg-rose-600/25 ${rowActionVisualClass}`}
                    disabled={isRowFullyBusy}
                    title={t("label.delete")}
                    aria-label={t("label.delete")}
                    onClick={() => {
                      if (
                        rowActionBusyRef.current[rowId] ||
                        isActionLoading ||
                        isRowBlocked
                      ) {
                        return;
                      }
                      setRowBusy(rowId, true);
                      setDeleteConfirmItem(queueItem);
                    }}
                  >
                    <Trash2 className="h-6 w-6" />
                  </Button>
                </div>
              </TableCell>
            </TableRow>
            {row.hasExpandableDetails && isExpanded ? (
              <TableRow>
                <TableCell
                  colSpan={activeTab === "activity" ? 6 : activeTab === "import" ? 6 : 5}
                  className="bg-muted/10 p-3"
                >
                  <ActivityQueueDetailsPanel
                    detailId={detailId}
                    releaseTitle={row.releaseTitle}
                    errorCode={queueItem.importErrorCode}
                    failureReason={row.failureReason}
                    t={t}
                  />
                </TableCell>
              </TableRow>
            ) : null}
          </Fragment>
        );
      })}
      {showHistorySpinner ? (
        <TableRow>
          <TableCell
            colSpan={activeTab === "activity" ? 6 : activeTab === "import" ? 6 : 5}
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
      <Card>
        <CardContent className="space-y-4">
          {queueError ? (
            <p className="rounded border border-rose-500/40 bg-rose-950/40 p-2 text-sm text-rose-200">
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
              <div className="overflow-x-auto">
                <Table
                  className={cn(
                    "table-fixed",
                    activeTab === "activity" ? "min-w-[820px]" : "min-w-[700px]",
                  )}
                >
                  <TableHeader>
                    <TableRow>
                      {activeTab === "import" ? (
                        <TableHead className="w-12 min-w-12">
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
                          />
                        </TableHead>
                      ) : null}
                      {renderSortableHeader("title", t("queue.title"), "w-[34%] min-w-0")}
                      {renderSortableHeader("client", t("queue.client"), "w-32 min-w-0")}
                      {renderSortableHeader("status", t("queue.status"), "w-44 min-w-0")}
                      {activeTab === "activity"
                        ? renderSortableHeader("progress", t("queue.progress"), "w-52 min-w-52")
                        : null}
                      {renderSortableHeader("size", t("queue.size"), "w-24 min-w-24")}
                      <TableHead className="w-44 min-w-44 text-right">{t("label.actions")}</TableHead>
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {sortedQueueItems.length === 0 ? (
                      <TableRow>
                        <TableCell
                          colSpan={activeTab === "activity" ? 6 : activeTab === "import" ? 6 : 5}
                          className={queueLoading ? "p-0" : "text-sm text-muted-foreground"}
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
    </>
  );
}
