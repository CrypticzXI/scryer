import type { ActivitySection } from "@/components/root/types";
import type { DownloadQueueItem } from "@/lib/types";
import { useTranslate } from "@/lib/context/translate-context";
import {
  buildQueueStatusDetail,
  normalizeQueueState,
} from "@/lib/utils/download-queue";

export type TranslateFn = ReturnType<typeof useTranslate>;

export type ActivityTab = ActivitySection;

export const queueStateClasses: Record<string, string> = {
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

export const queueStateLabels: Record<string, string> = {
  queued: "queue.state.queued",
  downloading: "queue.state.downloading",
  post_processing: "queue.state.postProcessing",
  paused: "queue.state.paused",
  completed: "queue.state.completed",
  importing: "queue.state.importing",
  removing: "queue.deleting",
  import_pending: "queue.state.importPending",
  import_blocked: "queue.state.importBlocked",
  import_failed: "queue.manualImportFailed",
  remove_failed: "queue.removeFailed",
  failed: "queue.state.failed",
};

export const queueStateAttention: Record<string, boolean> = {
  failed: true,
  importing: true,
  removing: true,
  import_pending: true,
  import_blocked: true,
  import_failed: true,
  remove_failed: true,
};

export function compareStrings(left: string, right: string): number {
  return left.localeCompare(right, undefined, { sensitivity: "base" });
}

export function activityStatusRank(tab: ActivityTab, displayState: string): number {
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

export type QueueRowPresentation = {
  stateKey: string;
  trackedStateKey: string;
  trackedMatchTypeKey: string;
  displayStateKey: string;
  percent: number;
  remainingLabel: string | null;
  hasTransferProgress: boolean;
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
  canMarkFailed: boolean;
  canInteractiveManualImport: boolean;
  canDirectManualImport: boolean;
};

export function deriveQueueRowPresentation(
  queueItem: DownloadQueueItem,
  t: TranslateFn,
): QueueRowPresentation {
  const stateKey = normalizeQueueState(queueItem.state);
  const trackedStateKey = normalizeQueueState(queueItem.trackedState);
  const trackedMatchTypeKey = normalizeQueueState(queueItem.trackedMatchType);
  const failureReason = buildQueueStatusDetail(queueItem);
  const displayStateKey = queueItem.displayState;
  const transferBytes = parseByteCount(queueItem.importTransferBytes);
  const transferTotalBytes = parseByteCount(queueItem.importTransferTotalBytes);
  const hasTransferProgress =
    displayStateKey === "importing" &&
    queueItem.importTransferPhase !== null &&
    transferBytes !== null &&
    transferTotalBytes !== null &&
    transferTotalBytes > 0;
  const percent = hasTransferProgress
    ? formatProgress((transferBytes / transferTotalBytes) * 100)
    : formatProgress(queueItem.progressPercent);
  const remainingLabel = hasTransferProgress
    ? `${formatByteCount(transferBytes)} / ${formatByteCount(transferTotalBytes)}`
    : formatRemainingDuration(queueItem.remainingSeconds);
  const needsManualImport =
    queueItem.attentionRequired ||
    queueStateAttention[stateKey] ||
    queueStateAttention[displayStateKey];
  const postProcessingStatusKey =
    stateKey === "verifying"
      ? "queue.state.verifying"
      : stateKey === "repairing"
        ? "queue.state.repairing"
        : stateKey === "extracting"
          ? "queue.state.extracting"
          : "queue.state.postProcessing";
  const statusLabel =
    queueItem.importTransferPhase === "copying"
      ? t("queue.transfer.copying")
      : queueItem.importTransferPhase === "finalizing"
        ? t("queue.transfer.finalizing")
        : displayStateKey === "post_processing"
          ? t(postProcessingStatusKey)
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
  const canMarkFailed =
    (trackedStateKey === "import_blocked" ||
      trackedStateKey === "import_pending" ||
      trackedStateKey === "failed_pending") &&
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
    queueItem.titleName.trim() || queueItem.downloadClientItemId.trim() || "—";
  const displayTitle = releaseTitle;
  const hasExpandableDetails =
    (displayStateKey === "import_blocked" ||
      displayStateKey === "import_failed" ||
      displayStateKey === "remove_failed") &&
    (failureReason.length > 0 || releaseTitle !== "—");

  return {
    stateKey,
    trackedStateKey,
    trackedMatchTypeKey,
    displayStateKey,
    percent,
    remainingLabel,
    hasTransferProgress,
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
    canMarkFailed,
    canInteractiveManualImport,
    canDirectManualImport,
  };
}

export function downloadQueueItemRowSelectorKey(
  queueItem: DownloadQueueItem,
  fallbackKey: string,
): string {
  if (queueItem.downloadId?.trim()) {
    return queueItem.downloadId.trim();
  }

  const ownerKey = queueItem.clientId.trim() || queueItem.clientType.trim();
  const itemKey = queueItem.downloadClientItemId.trim() || queueItem.id.trim();
  const queuedAt = queueItem.queuedAt?.trim();
  const selectorParts = [ownerKey, itemKey, queuedAt].filter(Boolean);
  return selectorParts.length >= 2 ? selectorParts.join("::") : fallbackKey;
}

export function canIgnoreImportItem(queueItem: DownloadQueueItem): boolean {
  const trackedStateKey = normalizeQueueState(queueItem.trackedState);
  const displayStateKey = normalizeQueueState(queueItem.displayState);
  return (
    trackedStateKey === "import_blocked" &&
    displayStateKey !== "importing" &&
    displayStateKey !== "removing"
  );
}

export function canDeleteImportItem(queueItem: DownloadQueueItem): boolean {
  const displayStateKey = normalizeQueueState(queueItem.displayState);
  return displayStateKey !== "importing" && displayStateKey !== "removing";
}

export function parseByteCount(sizeBytes: number | string | null): number | null {
  if (sizeBytes === null || sizeBytes === "") {
    return null;
  }
  const bytes = typeof sizeBytes === "number" ? sizeBytes : Number.parseFloat(sizeBytes);
  if (!Number.isFinite(bytes) || bytes < 0) {
    return null;
  }
  return bytes;
}

export function formatByteCount(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) {
    return "—";
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

export function formatBytes(sizeBytes: number | string | null): string {
  const bytes = parseByteCount(sizeBytes);
  return bytes === null ? "—" : formatByteCount(bytes);
}

export function formatProgress(progressPercent: number): number {
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

export function effectiveQueueItemProgress(queueItem: DownloadQueueItem): number {
  const transferBytes = parseByteCount(queueItem.importTransferBytes);
  const transferTotalBytes = parseByteCount(queueItem.importTransferTotalBytes);
  if (
    queueItem.displayState === "importing" &&
    queueItem.importTransferPhase !== null &&
    transferBytes !== null &&
    transferTotalBytes !== null &&
    transferTotalBytes > 0
  ) {
    return formatProgress((transferBytes / transferTotalBytes) * 100);
  }
  return formatProgress(queueItem.progressPercent);
}

export function formatRemainingDuration(remainingSeconds: number | null): string | null {
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

export function getProgressBarColor(stateKey: string): string {
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
