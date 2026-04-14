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

export function normalizeQueueState(state: string | null | undefined): string {
  return (state ?? "").trim().toLowerCase();
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

  if (trackedStateKey === "import_blocked" || trackedStateKey === "import_pending") {
    return trackedStateKey;
  }

  const importStatusKey = normalizeQueueState(queueItem.importStatus);
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
  return deriveDownloadQueueDisplayState(queueItem) === "import_blocked";
}
