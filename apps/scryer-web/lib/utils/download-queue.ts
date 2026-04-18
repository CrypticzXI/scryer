import type { DownloadQueueItem } from "@/lib/types";

export type DownloadQueueDisplayStateInput = Pick<
  DownloadQueueItem,
  | "state"
  | "attentionReason"
  | "importStatus"
  | "importErrorMessage"
  | "trackedState"
  | "trackedStatusMessages"
>;

export function downloadQueueItemIdentityKey(
  item: Pick<DownloadQueueItem, "id" | "clientType" | "downloadClientItemId">,
): string {
  if (!item.clientType.trim() && !item.downloadClientItemId.trim()) {
    return item.id;
  }

  return `${item.clientType}::${item.downloadClientItemId}`;
}

function parseQueueSortTimestamp(value: string | null | undefined): number {
  const parsed = Number.parseInt(value ?? "", 10);
  return Number.isFinite(parsed) ? parsed : 0;
}

function queueStateSortRank(state: string | null | undefined): number {
  switch (normalizeQueueState(state)) {
    case "downloading":
    case "verifying":
    case "repairing":
    case "extracting":
      return 0;
    case "queued":
      return 1;
    case "paused":
      return 2;
    case "import_pending":
    case "importpending":
    case "completed":
      return 3;
    case "failed":
      return 4;
    default:
      return 5;
  }
}

export function normalizeQueueState(state: string | null | undefined): string {
  return (state ?? "").trim().toLowerCase();
}

export function isActiveQueueState(state: string | null | undefined): boolean {
  const normalized = normalizeQueueState(state);
  return (
    normalized === "downloading" ||
    normalized === "queued" ||
    normalized === "paused" ||
    normalized === "verifying" ||
    normalized === "repairing" ||
    normalized === "extracting"
  );
}

export function isHistoryQueueState(state: string | null | undefined): boolean {
  const normalized = normalizeQueueState(state);
  return (
    normalized === "completed" ||
    normalized === "failed" ||
    normalized === "import_pending" ||
    normalized === "importpending"
  );
}

export function compareDownloadQueueItems(
  left: DownloadQueueItem,
  right: DownloadQueueItem,
): number {
  const leftRank = queueStateSortRank(left.state);
  const rightRank = queueStateSortRank(right.state);
  if (leftRank !== rightRank) {
    return leftRank - rightRank;
  }

  const leftState = normalizeQueueState(left.state);
  if (
    leftState === "downloading" ||
    leftState === "verifying" ||
    leftState === "repairing" ||
    leftState === "extracting"
  ) {
    return (
      right.progressPercent - left.progressPercent ||
      left.id.localeCompare(right.id)
    );
  }

  if (leftState === "queued" || leftState === "paused") {
    return (
      parseQueueSortTimestamp(left.queuedAt) - parseQueueSortTimestamp(right.queuedAt) ||
      left.id.localeCompare(right.id)
    );
  }

  return (
    parseQueueSortTimestamp(right.lastUpdatedAt) -
      parseQueueSortTimestamp(left.lastUpdatedAt) ||
    left.id.localeCompare(right.id)
  );
}

export function sortDownloadQueueItems(
  items: DownloadQueueItem[],
): DownloadQueueItem[] {
  return [...items].sort(compareDownloadQueueItems);
}

export function buildQueueStatusDetail(
  queueItem: DownloadQueueDisplayStateInput,
): string {
  const messages = [
    ...(queueItem.trackedStatusMessages ?? []),
    queueItem.attentionReason,
    queueItem.importErrorMessage,
  ]
    .map((value) => value?.trim())
    .filter((value): value is string => Boolean(value));

  return Array.from(new Set(messages)).join("\n");
}

export function isPostProcessingReason(reason: string | null | undefined): boolean {
  if (!reason) return false;
  const normalized = reason.toUpperCase();
  return (
    normalized.includes("PP_QUEUED") ||
    normalized.includes("POSTPROCESSING") ||
    normalized.includes("UNPACKING") ||
    normalized.includes("REPAIRING") ||
    normalized.includes("VERIFYING") ||
    normalized.includes("RENAMING") ||
    normalized.includes("MOVING") ||
    normalized.includes("EXECUTING_SCRIPT")
  );
}

export function deriveDownloadQueueDisplayState(
  queueItem: DownloadQueueDisplayStateInput,
): string {
  const stateKey = normalizeQueueState(queueItem.state);
  const trackedStateKey = normalizeQueueState(queueItem.trackedState);
  const failureReason = buildQueueStatusDetail(queueItem);
  const importStatusKey = normalizeQueueState(queueItem.importStatus);

  if (
    importStatusKey === "pending" ||
    importStatusKey === "running" ||
    importStatusKey === "processing"
  ) {
    return "importing";
  }

  if (
    (importStatusKey === "failed" || importStatusKey === "skipped") &&
    (trackedStateKey === "import_blocked" ||
      stateKey === "completed" ||
      stateKey === "import_pending" ||
      stateKey === "failed")
  ) {
    return "import_failed";
  }

  if (trackedStateKey === "import_blocked" || trackedStateKey === "import_pending") {
    return trackedStateKey;
  }

  const canDeriveBlockedState =
    trackedStateKey.length === 0 &&
    failureReason.length > 0 &&
    (stateKey === "completed" || stateKey === "import_pending" || stateKey === "failed") &&
    (importStatusKey === "skipped" || importStatusKey === "failed");
  if (canDeriveBlockedState) {
    return "import_blocked";
  }

  if (
    stateKey === "extracting" ||
    stateKey === "verifying" ||
    stateKey === "repairing"
  ) {
    return "post_processing";
  }

  if (
    stateKey === "downloading" &&
    isPostProcessingReason(queueItem.attentionReason)
  ) {
    return "post_processing";
  }

  return stateKey;
}

export function isManualImportRequiredQueueItem(
  queueItem: DownloadQueueDisplayStateInput,
): boolean {
  const state = deriveDownloadQueueDisplayState(queueItem);
  return state === "import_blocked" || state === "importing" || state === "import_failed";
}
