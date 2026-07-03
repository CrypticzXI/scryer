import { AlertCircle } from "lucide-react";

import { DownloadClientTypeLogo } from "@/components/common/download-client-type-logo";
import { ActivityProgressBar } from "@/components/views/activity-progress-bar";
import { useTranslate } from "@/lib/context/translate-context";
import type { DownloadQueueItem } from "@/lib/types/download-queue";
import { cn } from "@/lib/utils";
import {
  buildQueueStatusDetail,
  downloadQueueItemIdentityKey,
} from "@/lib/utils/download-queue";

const queueStateClasses: Record<string, string> = {
  queued: "border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)] text-[var(--scry-warning-text)]",
  downloading: "border-[var(--scry-info-border)] bg-[var(--scry-info-bg)] text-[var(--scry-info-text)]",
  post_processing: "border-[var(--scry-info-border)] bg-[var(--scry-info-bg)] text-[var(--scry-info-text)]",
  paused: "border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)] text-[var(--scry-warning-text)]",
  completed: "border-[var(--scry-success-border)] bg-[var(--scry-success-bg)] text-[var(--scry-success-text)]",
  importing: "border-[var(--scry-info-border)] bg-[var(--scry-info-bg)] text-[var(--scry-info-text)]",
  removing: "border-[var(--scry-info-border)] bg-[var(--scry-info-bg)] text-[var(--scry-info-text)]",
  import_pending: "border-[rgba(var(--scry-accent-rgb),0.4)] bg-[rgba(var(--scry-accent-rgb),0.1)] text-[var(--scry-accent-text)]",
  import_blocked: "border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)] text-[var(--scry-warning-text)]",
  import_failed: "border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] text-[var(--scry-danger-text)]",
  remove_failed: "border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] text-[var(--scry-danger-text)]",
  failed: "border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] text-[var(--scry-danger-text)]",
};

const queueStateLabels: Record<string, string> = {
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

function parseByteCount(sizeBytes: number | string | null): number | null {
  if (sizeBytes === null || sizeBytes === "") {
    return null;
  }
  const bytes = typeof sizeBytes === "number" ? sizeBytes : Number.parseFloat(sizeBytes);
  if (!Number.isFinite(bytes) || bytes < 0) {
    return null;
  }
  return bytes;
}

function formatByteCount(bytes: number): string {
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

function queueItemProgress(item: DownloadQueueItem): {
  percent: number;
  remainingLabel: string | null;
} {
  const transferBytes = parseByteCount(item.importTransferBytes);
  const transferTotalBytes = parseByteCount(item.importTransferTotalBytes);
  if (
    item.displayState === "importing" &&
    item.importTransferPhase !== null &&
    transferBytes !== null &&
    transferTotalBytes !== null &&
    transferTotalBytes > 0
  ) {
    return {
      percent: formatProgress((transferBytes / transferTotalBytes) * 100),
      remainingLabel: `${formatByteCount(transferBytes)} / ${formatByteCount(transferTotalBytes)}`,
    };
  }
  return {
    percent: formatProgress(item.progressPercent),
    remainingLabel: formatRemainingDuration(item.remainingSeconds),
  };
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
      return "bg-[var(--scry-success-solid)]";
    case "failed":
    case "remove_failed":
      return "bg-[var(--scry-danger-solid)]";
    case "paused":
      return "bg-[var(--scry-warning-solid)]";
    case "import_pending":
      return "bg-[rgb(var(--scry-accent-rgb))]";
    case "downloading":
    case "removing":
    case "importing":
      return "bg-[var(--scry-info-solid)]";
    case "post_processing":
      return "bg-[var(--scry-info-solid)]";
    case "queued":
      return "bg-gray-400";
    default:
      return "bg-muted-foreground";
  }
}

function queueStatusLabel(
  queueItem: DownloadQueueItem,
  t: ReturnType<typeof useTranslate>,
): string {
  const stateKey = queueItem.displayState;
  const stageLabel =
    queueItem.attentionReason?.trim() ??
    queueItem.trackedStatusMessages[0]?.trim() ??
    "";
  if (stateKey === "post_processing" && stageLabel.length > 0) {
    return stageLabel;
  }
  if (queueItem.importTransferPhase === "copying") {
    return t("queue.transfer.copying");
  }
  if (queueItem.importTransferPhase === "finalizing") {
    return t("queue.transfer.finalizing");
  }
  return t(queueStateLabels[stateKey] ?? "queue.state.unknown");
}

export function MovieOverviewDownloadList({
  items,
  className,
}: {
  items: DownloadQueueItem[];
  className?: string;
}) {
  const t = useTranslate();

  return (
    <div className={cn("space-y-3", className)}>
      {items.map((item) => {
        const stateKey = item.displayState;
        const { percent: progress, remainingLabel } = queueItemProgress(item);
        const detail = buildQueueStatusDetail(item);
        const rowId = downloadQueueItemIdentityKey(item);

        return (
          <div
            key={rowId}
            className="rounded-xl border border-border/70 bg-card/70 px-4 py-3"
          >
            <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
              <div className="min-w-0 flex-1 space-y-2">
                <div className="flex items-start gap-3">
                  <div className="mt-0.5 rounded-md border border-border/70 bg-background/80 p-1.5">
                    <DownloadClientTypeLogo
                      typeValue={item.clientType}
                      className="h-4 w-4"
                    />
                  </div>
                  <div className="min-w-0 flex-1 space-y-1">
                    <div className="flex flex-wrap items-center gap-2">
                      <span
                        className={cn(
                          "inline-flex items-center rounded-full border px-2.5 py-0.5 text-[11px] font-medium",
                          queueStateClasses[stateKey] ??
                            "border-border/70 bg-background/80 text-foreground",
                        )}
                      >
                        {queueStatusLabel(item, t)}
                      </span>
                      <span className="text-xs text-muted-foreground">
                        {item.clientName}
                      </span>
                    </div>
                    <p className="truncate text-sm font-medium text-card-foreground">
                      {item.titleName}
                    </p>
                    {detail ? (
                      <p className="flex items-start gap-1 text-xs text-muted-foreground">
                        <AlertCircle className="mt-0.5 h-3 w-3 shrink-0" />
                        <span className="line-clamp-2 whitespace-pre-wrap">{detail}</span>
                      </p>
                    ) : null}
                  </div>
                </div>
                <ActivityProgressBar
                  percent={progress}
                  remainingLabel={remainingLabel}
                  colorClass={getProgressBarColor(stateKey)}
                  indeterminate={stateKey === "queued"}
                />
              </div>
            </div>
          </div>
        );
      })}
    </div>
  );
}

export function EpisodeQueueIndicator({
  item,
  className,
}: {
  item: DownloadQueueItem;
  className?: string;
}) {
  const stateKey = item.displayState;

  return (
    <div className={cn("w-24", className)}>
      <ActivityProgressBar
        percent={queueItemProgress(item).percent}
        remainingLabel={queueItemProgress(item).remainingLabel}
        colorClass={getProgressBarColor(stateKey)}
        compact
        hideLabel
        indeterminate={stateKey === "queued"}
      />
    </div>
  );
}
