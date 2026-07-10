import { ChevronDown, ChevronUp } from "lucide-react";

import { queueStateClasses, type TranslateFn } from "@/lib/utils/activity-utils";

export function ActivityQueueStatusBadge({
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
  const className = `inline-flex items-center gap-1.5 rounded border px-2 py-1 text-xs font-medium ${queueStateClasses[stateKey.toLowerCase()] ?? "border-border bg-muted text-card-foreground"}`;

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

export function ActivityQueueTitleContent({
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

export function ActivityQueueDetailsPanel({
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
      className="rounded-lg border border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)] p-3"
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
              <p className="mt-1 break-words text-sm font-[var(--font-code)] text-foreground">{errorCode}</p>
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
            {failureReason || "—"}
          </p>
        </div>
      </div>
    </div>
  );
}
