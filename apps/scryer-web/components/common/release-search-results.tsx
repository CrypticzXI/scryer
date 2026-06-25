import * as React from "react";
import {
  Ban,
  ChevronDown,
  ChevronUp,
  ArrowDown,
  ArrowUp,
  Check,
  Database,
  Download,
  FilePlus2,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { useTranslate } from "@/lib/context/translate-context";
import { cn } from "@/lib/utils";
import {
  releaseSearchResultQueueAdditionalId,
  releaseSearchResultQueueId,
  releaseSearchResultRowId,
} from "@/lib/utils/dom-ids";
import type { Release } from "@/lib/types";

export type ReleaseSearchSortKey = "score" | "size";
export type ReleaseSearchSortDirection = "asc" | "desc";
type SortKey = ReleaseSearchSortKey;
type SortDirection = ReleaseSearchSortDirection;
type SearchResultPresentation = "default" | "selected-title";

const selectedTitleTagClassNames = [
  "bg-[rgba(56,189,248,0.13)] text-[#5cc8f5]",
  "bg-[rgba(var(--scry-accent-rgb),0.15)] text-[var(--scry-accent-text)]",
  "bg-[rgba(168,85,247,0.15)] text-[#c79bf5]",
  "bg-[var(--scry-chip)] text-[var(--scry-muted2)]",
];

function selectedTitleTagClassName(index: number) {
  return cn(
    "inline-flex items-center rounded-[6px] px-[9px] py-[3px] text-[10.5px] font-semibold",
    selectedTitleTagClassNames[index % selectedTitleTagClassNames.length],
  );
}

function getScoreText(score: number | undefined) {
  if (score == null) {
    return "—";
  }
  return score > 0 ? `+${score}` : `${score}`;
}

function bytesToWholeReadable(raw: number | null | undefined) {
  if (!raw || raw <= 0) {
    return "—";
  }
  const gb = 1024 * 1024 * 1024;
  const mb = 1024 * 1024;
  const kb = 1024;

  if (raw > gb) {
    return `${Math.floor(raw / gb)} GB`;
  }
  if (raw > mb) {
    return `${Math.floor(raw / mb)} MB`;
  }
  if (raw > kb) {
    return `${Math.floor(raw / kb)} KB`;
  }
  return `${Math.floor(raw)} B`;
}

function getSortableValue(a: Release, key: SortKey): number {
  if (key === "score") {
    return a.qualityProfileDecision?.releaseScore ?? Number.NEGATIVE_INFINITY;
  }
  return a.sizeBytes ?? 0;
}

function sortBy(
  releaseList: Release[],
  sortKey: SortKey,
  sortDirection: SortDirection,
): Release[] {
  const factor = sortDirection === "asc" ? 1 : -1;

  return [...releaseList].sort((left, right) => {
    const leftValue = getSortableValue(left, sortKey);
    const rightValue = getSortableValue(right, sortKey);
    const delta = (leftValue - rightValue) * factor;
    if (delta !== 0) {
      return delta;
    }
    return left.title.localeCompare(right.title);
  });
}

function SearchResultRow({
  result,
  onQueue,
  onQueueAdditional,
  blocked,
  disabled = false,
  requireCandidateToken = false,
  mobile = false,
  presentation = "default",
}: {
  result: Release;
  onQueue: (r: Release) => Promise<void> | void;
  onQueueAdditional?: (r: Release) => Promise<void> | void;
  blocked: boolean;
  disabled?: boolean;
  requireCandidateToken?: boolean;
  mobile?: boolean;
  presentation?: SearchResultPresentation;
}) {
  const t = useTranslate();
  const [expanded, setExpanded] = React.useState(false);
  const [queueRequested, setQueueRequested] = React.useState(false);
  const [additionalQueueRequested, setAdditionalQueueRequested] =
    React.useState(false);
  const decision = result.qualityProfileDecision;
  const hasLog = decision && decision.scoringLog.length > 0;
  const blockReason =
    blocked && decision && decision.blockCodes.length > 0
      ? decision.blockCodes.join(" · ")
      : null;
  const rejectionBadge = blockReason ? (
    <span className="inline-flex max-w-full items-center gap-1 rounded-[6px] bg-red-500/10 px-2 py-0.5 text-[10px] font-bold text-red-400">
      <Ban className="h-2.5 w-2.5 shrink-0" />
      <span className="min-w-0 break-words">{blockReason}</span>
    </span>
  ) : null;
  const approvedBadge =
    !blocked && decision?.allowed ? (
      <span className="inline-flex items-center gap-1 rounded-[6px] bg-emerald-500/15 px-[7px] py-px text-[10px] font-bold text-[#4ade80]">
        <Check className="h-2.5 w-2.5" />
        Approved
      </span>
    ) : null;
  const parsedBits = [
    result.parsedRelease?.quality,
    result.parsedRelease?.videoCodec,
    result.parsedRelease?.videoEncoding,
    result.parsedRelease?.audio,
  ]
    .filter((value) => value)
    .filter((value) => typeof value === "string" && value.trim().length > 0);
  const parsedMetadata = result.parsedRelease
    ? [
        result.parsedRelease.detectedHdr
          ? { label: "HDR", className: "bg-cyan-500/20 text-cyan-300" }
          : null,
        result.parsedRelease.isDolbyVision
          ? {
              label: "Dolby Vision",
              className: "bg-indigo-500/20 text-indigo-300",
            }
          : null,
        result.parsedRelease.isProperUpload
          ? { label: "Proper", className: "bg-amber-500/20 text-amber-300" }
          : null,
        result.parsedRelease.isRemux
          ? { label: "Remux", className: "bg-violet-500/20 text-violet-300" }
          : null,
        result.parsedRelease.isBdDisk
          ? { label: "BD", className: "bg-rose-500/20 text-rose-300" }
          : null,
        result.parsedRelease.isAiEnhanced
          ? { label: "AI Enhanced", className: "bg-red-500/20 text-red-300" }
          : null,
        result.parsedRelease.isAtmos
          ? { label: "Atmos", className: "bg-purple-500/20 text-purple-300" }
          : null,
      ]
        .filter(Boolean)
        .filter(
          (value) =>
            value !== null && value !== undefined && typeof value === "object",
        )
        .map((entry) => entry as { label: string; className: string })
    : [];
  const queueUnavailableReason =
    requireCandidateToken && !blocked && !result.candidateToken
      ? t("queue.manualUnavailableForResult")
      : null;
  const queueDisabled =
    disabled || blocked || queueRequested || queueUnavailableReason !== null;
  const queueButtonMuted = blocked || queueUnavailableReason !== null;
  const additionalQueueDisabled =
    disabled ||
    blocked ||
    additionalQueueRequested ||
    queueUnavailableReason !== null ||
    !onQueueAdditional;
  const idVariant = mobile ? "mobile" : undefined;
  const rowId = releaseSearchResultRowId(result, idVariant);
  const queueButtonId = releaseSearchResultQueueId(result, idVariant);
  const queueAdditionalButtonId = releaseSearchResultQueueAdditionalId(
    result,
    idVariant,
  );

  const handleQueueClick = React.useCallback(() => {
    if (queueDisabled) {
      return;
    }

    setQueueRequested(true);

    try {
      const maybePromise = onQueue(result);
      if (
        maybePromise &&
        typeof (maybePromise as Promise<void>).then === "function"
      ) {
        void (maybePromise as Promise<void>).catch(() => {
          setQueueRequested(false);
        });
      }
    } catch {
      setQueueRequested(false);
    }
  }, [onQueue, queueDisabled, result]);

  const handleQueueAdditionalClick = React.useCallback(() => {
    if (additionalQueueDisabled || !onQueueAdditional) {
      return;
    }

    setAdditionalQueueRequested(true);

    try {
      const maybePromise = onQueueAdditional(result);
      if (
        maybePromise &&
        typeof (maybePromise as Promise<void>).then === "function"
      ) {
        void (maybePromise as Promise<void>).catch(() => {
          setAdditionalQueueRequested(false);
        });
      }
    } catch {
      setAdditionalQueueRequested(false);
    }
  }, [additionalQueueDisabled, onQueueAdditional, result]);

  if (presentation === "selected-title") {
    const scoreToneClassName = decision
      ? decision.releaseScore < 0
        ? "text-[#ef6a7a]"
        : "text-[#4ade80]"
      : "text-[var(--scry-faint)]";
    const selectedTitleTags = [
      ...parsedBits.map((metadataBit, index) => ({
        className: selectedTitleTagClassName(index),
        label: metadataBit,
      })),
      ...parsedMetadata.map((metadataBit) => ({
        className: cn(
          "inline-flex items-center rounded-[6px] px-[9px] py-[3px] text-[10.5px] font-semibold",
          metadataBit.className,
        ),
        label: metadataBit.label,
      })),
    ];

    return (
      <div
        id={rowId}
        data-ui="release-search-result-row"
        data-release-source={result.source ?? ""}
        data-release-title={result.title}
        data-release-link={result.link ?? ""}
        data-release-download-url={result.downloadUrl ?? ""}
        data-release-candidate-token={result.candidateToken ?? ""}
        className="flex items-center gap-4 border-b border-[var(--scry-line2)] px-4 py-3.5 transition-colors last:border-b-0 hover:bg-[var(--scry-hover)] max-md:flex-col max-md:items-stretch"
      >
        <div className="min-w-0 flex-1">
          <div className="mb-1.5 break-words text-[13px] font-semibold leading-[1.35] text-[var(--scry-ink2)]">
            {result.title}
          </div>
          <div className="mb-2 flex flex-wrap items-center gap-x-[7px] gap-y-1 text-[11px] text-[var(--scry-faint)]">
            <Database className="h-3 w-3 shrink-0" />
            <span>{result.source ?? t("label.unknown")}</span>
            <span
              aria-hidden="true"
              className="h-[3px] w-[3px] rounded-full bg-[var(--scry-faint4)]"
            />
            <span>{result.publishedAt ?? t("label.unknown")}</span>
            {approvedBadge}
            {rejectionBadge}
          </div>
          {selectedTitleTags.length > 0 ? (
            <div className="flex flex-wrap gap-1.5">
              {selectedTitleTags.map((metadataBit, index) => (
                <span
                  key={`${metadataBit.label}-${index}`}
                  className={metadataBit.className}
                >
                  {metadataBit.label}
                </span>
              ))}
            </div>
          ) : null}
          {queueUnavailableReason ? (
            <p className="mt-2 text-[11px] text-[var(--scry-faint)]">
              {queueUnavailableReason}
            </p>
          ) : null}
        </div>
        <div className="w-[74px] shrink-0 text-center max-md:w-auto max-md:text-left">
          <div
            className={cn(
              "text-[14px] font-bold tabular-nums",
              scoreToneClassName,
            )}
          >
            {getScoreText(decision?.releaseScore)}
          </div>
          <div className="mt-0.5 text-[9.5px] font-bold uppercase tracking-[0.06em] text-[var(--scry-faint3)]">
            Score
          </div>
        </div>
        <div className="w-16 shrink-0 text-right max-md:w-auto max-md:text-left">
          <div
            className="text-[15px] font-bold text-[var(--scry-ink2)]"
            style={{
              fontFamily:
                "'Space Grotesk Variable', 'Inter Variable', ui-sans-serif, system-ui, sans-serif",
            }}
          >
            {bytesToWholeReadable(result.sizeBytes)}
          </div>
        </div>
        <div className="flex w-[166px] shrink-0 flex-col gap-[7px] max-md:w-full">
          <Button
            id={queueButtonId}
            data-ui="release-search-result-queue"
            size="sm"
            onClick={handleQueueClick}
            disabled={queueDisabled}
            className={cn(
              "h-[38px] justify-center gap-[7px] rounded-[10px] border-0 text-[12.5px] font-bold text-white shadow-[0_6px_16px_rgba(34,197,94,0.28)]",
              queueButtonMuted
                ? "border border-[var(--scry-border2)] bg-[var(--scry-soft)] text-[var(--scry-muted3)] shadow-none hover:bg-[var(--scry-soft)] hover:text-[var(--scry-muted3)]"
                : "bg-[linear-gradient(135deg,#1f9d57,#22c55e)] hover:bg-[linear-gradient(135deg,#22b863,#2ad06a)]",
            )}
          >
            {queueRequested ? (
              <>
                <Check className="h-[15px] w-[15px]" />
                {t("queue.state.queued")}
              </>
            ) : (
              <>
                <Download className="h-[15px] w-[15px]" />
                {t("nzb.queue")}
              </>
            )}
          </Button>
          {onQueueAdditional ? (
            <Button
              id={queueAdditionalButtonId}
              data-ui="release-search-result-queue-additional"
              type="button"
              size="sm"
              variant="outline"
              onClick={handleQueueAdditionalClick}
              disabled={additionalQueueDisabled}
              className={cn(
                "h-[34px] justify-center gap-[7px] rounded-[10px] border-[var(--scry-baccent)] bg-[rgba(var(--scry-accent-rgb),0.16)] text-[11.5px] font-semibold text-[var(--scry-accent-text)] shadow-none hover:border-[var(--scry-accent)] hover:bg-[rgba(var(--scry-accent-rgb),0.26)] hover:text-[var(--scry-accent-text)]",
                additionalQueueRequested &&
                  "border-emerald-500/30 bg-emerald-500/10 text-emerald-700 hover:bg-emerald-500/15 hover:text-emerald-800 dark:text-emerald-200 dark:hover:text-emerald-100",
              )}
            >
              {additionalQueueRequested ? (
                <>
                  <Check className="h-3.5 w-3.5" />
                  {t("queue.state.queued")}
                </>
              ) : (
                <>
                  <FilePlus2 className="h-3.5 w-3.5" />
                  {t("nzb.queueAdditionalFile")}
                </>
              )}
            </Button>
          ) : null}
        </div>
      </div>
    );
  }

  if (mobile) {
    return (
      <div
        id={rowId}
        data-ui="release-search-result-row"
        data-release-source={result.source ?? ""}
        data-release-title={result.title}
        data-release-link={result.link ?? ""}
        data-release-download-url={result.downloadUrl ?? ""}
        data-release-candidate-token={result.candidateToken ?? ""}
        className="rounded-lg border border-border bg-background/40 p-3"
      >
        <div className="space-y-2">
          <p className="min-w-0 whitespace-normal break-words text-sm font-semibold leading-snug text-foreground">
            {result.title}
          </p>
          <div className="flex flex-wrap items-center gap-x-2 gap-y-1 text-xs text-muted-foreground">
            <span>{result.source ?? t("label.unknown")}</span>
            <span aria-hidden="true">•</span>
            <span>{result.publishedAt ?? t("label.unknown")}</span>
            <span aria-hidden="true">•</span>
            <span className="font-medium text-foreground/80">
              {bytesToWholeReadable(result.sizeBytes)}
            </span>
            {rejectionBadge}
          </div>
          {parsedBits.length > 0 ? (
            <div className="flex flex-wrap gap-1.5">
              {parsedBits.map((metadataBit) => (
                <span
                  key={metadataBit}
                  className="inline-flex items-center rounded-full border border-border bg-muted px-2 py-0.5 text-[11px] font-medium text-muted-foreground"
                >
                  {metadataBit}
                </span>
              ))}
            </div>
          ) : null}
          {parsedMetadata.length > 0 ? (
            <div className="flex flex-wrap gap-1.5">
              {parsedMetadata.map((metadataBit) => (
                <span
                  key={metadataBit.label}
                  className={`inline-flex items-center rounded-full border border-transparent px-2 py-0.5 text-[11px] font-medium ${metadataBit.className}`}
                >
                  {metadataBit.label}
                </span>
              ))}
            </div>
          ) : null}
          {queueUnavailableReason ? (
            <p className="text-xs text-muted-foreground">
              {queueUnavailableReason}
            </p>
          ) : null}
          <div className="mt-1 grid gap-2">
            <Button
              id={queueButtonId}
              data-ui="release-search-result-queue"
              size="sm"
              onClick={handleQueueClick}
              disabled={queueDisabled}
              className={
                queueButtonMuted
                  ? "w-full"
                  : queueRequested
                    ? "w-full border border-emerald-500/50 dark:border-emerald-300/70 bg-emerald-200 dark:bg-emerald-900/80 text-emerald-800 dark:text-emerald-100"
                    : "w-full bg-emerald-600 text-foreground hover:bg-emerald-500 focus-visible:ring-emerald-300/70 border border-emerald-500/60 dark:border-emerald-400/50"
              }
              variant={queueButtonMuted ? "ghost" : "default"}
            >
              {queueRequested ? (
                <span className="inline-flex items-center gap-1.5">
                  <Check className="h-3.5 w-3.5" />
                  {t("queue.state.queued")}
                </span>
              ) : (
                t("nzb.queue")
              )}
            </Button>
            {onQueueAdditional ? (
              <Button
                id={queueAdditionalButtonId}
                data-ui="release-search-result-queue-additional"
                type="button"
                size="sm"
                variant="outline"
                onClick={handleQueueAdditionalClick}
                disabled={additionalQueueDisabled}
                className={cn(
                  "w-full border-[rgba(var(--scry-accent-rgb),0.30)] bg-[rgba(var(--scry-accent-rgb),0.08)] text-[var(--scry-accent-text)] shadow-none hover:bg-[rgba(var(--scry-accent-rgb),0.13)] hover:text-[var(--scry-accent-text)] focus-visible:ring-[rgba(var(--scry-accent-rgb),0.25)]",
                  additionalQueueRequested &&
                    "border-emerald-500/30 bg-emerald-500/10 text-emerald-700 hover:bg-emerald-500/15 hover:text-emerald-800 dark:text-emerald-200 dark:hover:text-emerald-100",
                )}
              >
                {additionalQueueRequested ? (
                  <span className="inline-flex items-center gap-1.5">
                    <Check className="h-3.5 w-3.5" />
                    {t("queue.state.queued")}
                  </span>
                ) : (
                  <span className="inline-flex items-center gap-1.5">
                    <FilePlus2 className="h-3.5 w-3.5" />
                    {t("nzb.queueAdditionalFile")}
                  </span>
                )}
              </Button>
            ) : null}
          </div>
        </div>
      </div>
    );
  }

  return (
    <>
      <tr
        id={rowId}
        data-ui="release-search-result-row"
        data-release-source={result.source ?? ""}
        data-release-title={result.title}
        data-release-link={result.link ?? ""}
        data-release-download-url={result.downloadUrl ?? ""}
        data-release-candidate-token={result.candidateToken ?? ""}
      >
        <td className="rounded-l-lg border border-border border-r-0 px-4 py-2 align-middle">
          <div className="space-y-1">
            <p className="min-w-0 whitespace-normal break-words text-base font-semibold text-foreground">
              {result.title}
            </p>
            <div className="flex flex-wrap items-center gap-x-2 gap-y-1 text-xs text-muted-foreground">
              <span>{result.source ?? t("label.unknown")}</span>
              <span aria-hidden="true">•</span>
              <span>{result.publishedAt ?? t("label.unknown")}</span>
              {rejectionBadge}
            </div>
            {parsedBits.length > 0 ? (
              <div className="mt-1 flex flex-wrap gap-1.5">
                {parsedBits.map((metadataBit) => (
                  <span
                    key={metadataBit}
                    className="inline-flex items-center rounded-full border border-border bg-muted px-2 py-0.5 text-[11px] font-medium text-muted-foreground"
                  >
                    {metadataBit}
                  </span>
                ))}
              </div>
            ) : null}
            {parsedMetadata.length > 0 ? (
              <div className="mt-1 flex flex-wrap gap-1.5">
                {parsedMetadata.map((metadataBit) => (
                  <span
                    key={metadataBit.label}
                    className={`inline-flex items-center rounded-full border border-transparent px-2 py-0.5 text-[11px] font-medium ${metadataBit.className}`}
                  >
                    {metadataBit.label}
                  </span>
                ))}
              </div>
            ) : null}
            {queueUnavailableReason ? (
              <p className="mt-1 text-xs text-muted-foreground">
                {queueUnavailableReason}
              </p>
            ) : null}
          </div>
        </td>
        <td className="border border-border border-x-0 px-4 py-2 text-center align-middle">
          {decision ? (
            hasLog ? (
              <button
                type="button"
                className={`text-sm font-mono underline-offset-2 hover:underline ${decision.releaseScore < 0 ? "text-red-400" : "text-emerald-700 dark:text-emerald-300"}`}
                onClick={() => setExpanded((prev) => !prev)}
                aria-label={
                  expanded ? t("nzb.hideScoringLog") : t("nzb.showScoringLog")
                }
              >
                {getScoreText(decision.releaseScore)}
              </button>
            ) : (
              <span
                className={`text-sm font-mono ${decision.releaseScore < 0 ? "text-red-400" : "text-emerald-700 dark:text-emerald-300"}`}
              >
                {getScoreText(decision.releaseScore)}
              </span>
            )
          ) : (
            <span className="text-sm font-mono text-muted-foreground">
              {getScoreText(undefined)}
            </span>
          )}
        </td>
        <td className="border border-border border-x-0 px-4 py-2 text-center text-xl font-semibold text-foreground align-middle">
          {bytesToWholeReadable(result.sizeBytes)}
        </td>
        <td className="rounded-r-lg border border-border border-l-0 px-4 py-2 text-center align-middle">
          <div className="flex flex-col items-stretch gap-2">
            <Button
              id={queueButtonId}
              data-ui="release-search-result-queue"
              size="default"
              onClick={handleQueueClick}
              disabled={queueDisabled}
              className={
                queueButtonMuted
                  ? "h-10"
                  : queueRequested
                    ? "h-10 border border-emerald-500/50 dark:border-emerald-300/70 bg-emerald-200 dark:bg-emerald-900/80 text-emerald-800 dark:text-emerald-100"
                    : "h-10 bg-emerald-600 text-foreground hover:bg-emerald-500 focus-visible:ring-emerald-300/70 border border-emerald-500/60 dark:border-emerald-400/50"
              }
              variant={queueButtonMuted ? "ghost" : "default"}
            >
              {queueRequested ? (
                <span className="inline-flex items-center gap-1.5">
                  <Check className="h-3.5 w-3.5" />
                  {t("queue.state.queued")}
                </span>
              ) : (
                t("nzb.queue")
              )}
            </Button>
            {onQueueAdditional ? (
              <Button
                id={queueAdditionalButtonId}
                data-ui="release-search-result-queue-additional"
                type="button"
                size="sm"
                variant="outline"
                onClick={handleQueueAdditionalClick}
                disabled={additionalQueueDisabled}
                className={cn(
                  "h-9 border-[rgba(var(--scry-accent-rgb),0.30)] bg-[rgba(var(--scry-accent-rgb),0.08)] text-[var(--scry-accent-text)] shadow-none hover:bg-[rgba(var(--scry-accent-rgb),0.13)] hover:text-[var(--scry-accent-text)] focus-visible:ring-[rgba(var(--scry-accent-rgb),0.25)]",
                  additionalQueueRequested &&
                    "border-emerald-500/30 bg-emerald-500/10 text-emerald-700 hover:bg-emerald-500/15 hover:text-emerald-800 dark:text-emerald-200 dark:hover:text-emerald-100",
                )}
              >
                {additionalQueueRequested ? (
                  <span className="inline-flex items-center gap-1.5">
                    <Check className="h-3.5 w-3.5" />
                    {t("queue.state.queued")}
                  </span>
                ) : (
                  <span className="inline-flex items-center gap-1.5">
                    <FilePlus2 className="h-3.5 w-3.5" />
                    {t("nzb.queueAdditionalFile")}
                  </span>
                )}
              </Button>
            ) : null}
          </div>
        </td>
      </tr>
      {expanded && hasLog ? (
        <tr>
          <td
            colSpan={4}
            className="border border-x border-t-0 border-border p-0"
          >
            <div className="bg-background/80 px-3 py-2">
              <p className="mb-1 text-xs font-semibold text-muted-foreground">
                {t("nzb.scoringLog")}
              </p>
              <div className="space-y-0.5">
                {decision.scoringLog.map((entry) => {
                  const badge = entry.source?.startsWith("user:")
                    ? "Custom Rule"
                    : entry.source?.startsWith("system:")
                      ? "System Rule"
                      : null;
                  return (
                    <div
                      key={entry.code}
                      className="flex justify-between gap-4 font-mono text-xs"
                    >
                      <span className="flex items-center gap-1.5 text-muted-foreground">
                        {entry.code}
                        {badge && (
                          <>
                            <span className="rounded bg-muted px-1 py-0.5 font-sans text-[10px] leading-none text-muted-foreground">
                              {badge}
                            </span>
                            {entry.ruleSetName && (
                              <span className="font-sans text-[10px] text-muted-foreground/60">
                                {entry.ruleSetName}
                              </span>
                            )}
                          </>
                        )}
                      </span>
                      <span
                        className={
                          entry.delta < 0
                            ? "text-red-400"
                            : "text-emerald-600 dark:text-emerald-400"
                        }
                      >
                        {entry.delta > 0 ? "+" : ""}
                        {entry.delta}
                      </span>
                    </div>
                  );
                })}
              </div>
              <div className="mt-1.5 flex justify-between border-t border-border pt-1.5 font-mono text-xs font-semibold">
                <span className="text-muted-foreground">{t("nzb.total")}</span>
                <span
                  className={
                    decision.releaseScore < 0
                      ? "text-red-400"
                      : "text-emerald-600 dark:text-emerald-400"
                  }
                >
                  {getScoreText(decision.releaseScore)}
                </span>
              </div>
            </div>
          </td>
        </tr>
      ) : null}
    </>
  );
}

export function SearchResultBuckets({
  results,
  onQueue,
  onQueueAdditional,
  canQueueAdditional,
  disabled = false,
  requireCandidateToken = false,
  compact = false,
  sortKey: controlledSortKey,
  sortDirection: controlledSortDirection,
  onSortChange,
  hideInlineSortControls = false,
  showBlockedInline = false,
  presentation = "default",
}: {
  results: Release[];
  onQueue: (r: Release) => Promise<void> | void;
  onQueueAdditional?: (r: Release) => Promise<void> | void;
  canQueueAdditional?: (r: Release) => boolean;
  disabled?: boolean;
  requireCandidateToken?: boolean;
  compact?: boolean;
  sortKey?: SortKey;
  sortDirection?: SortDirection;
  onSortChange?: (key: SortKey, direction: SortDirection) => void;
  hideInlineSortControls?: boolean;
  showBlockedInline?: boolean;
  presentation?: SearchResultPresentation;
}) {
  const t = useTranslate();
  const isBlockedEntry = React.useCallback(
    (entry: Release) =>
      Boolean(
        entry.qualityProfileDecision && !entry.qualityProfileDecision.allowed,
      ),
    [],
  );
  const considered = React.useMemo(
    () => results.filter((entry) => !isBlockedEntry(entry)),
    [isBlockedEntry, results],
  );
  const blocked = React.useMemo(
    () => results.filter((entry) => isBlockedEntry(entry)),
    [isBlockedEntry, results],
  );
  const [showBlocked, setShowBlocked] = React.useState(false);
  const [localSortKey, setLocalSortKey] = React.useState<SortKey>("score");
  const [localSortDirection, setLocalSortDirection] =
    React.useState<SortDirection>("desc");
  const sortKey = controlledSortKey ?? localSortKey;
  const sortDirection = controlledSortDirection ?? localSortDirection;

  const sortedConsidered = React.useMemo(
    () => sortBy(considered, sortKey, sortDirection),
    [considered, sortDirection, sortKey],
  );
  const sortedBlocked = React.useMemo(
    () => sortBy(blocked, sortKey, sortDirection),
    [blocked, sortDirection, sortKey],
  );
  const sortedInlineResults = React.useMemo(
    () => sortBy(results, sortKey, sortDirection),
    [results, sortDirection, sortKey],
  );

  const handleSort = React.useCallback(
    (next: SortKey) => {
      const nextDirection: SortDirection =
        sortKey === next
          ? sortDirection === "asc"
            ? "desc"
            : "asc"
          : "desc";

      if (controlledSortKey === undefined) {
        setLocalSortKey(next);
      }
      if (controlledSortDirection === undefined) {
        setLocalSortDirection(nextDirection);
      }
      onSortChange?.(next, nextDirection);
    },
    [controlledSortDirection, controlledSortKey, onSortChange, sortDirection, sortKey],
  );

  const renderSortIcon = React.useCallback(
    (key: SortKey) => {
      if (sortKey !== key) {
        return <ChevronDown className="h-3 w-3 opacity-30" />;
      }
      return sortDirection === "desc" ? (
        <ArrowDown className="h-3 w-3" />
      ) : (
        <ArrowUp className="h-3 w-3" />
      );
    },
    [sortDirection, sortKey],
  );

  const renderTable = React.useCallback(
    (
      entries: Release[],
      isBlocked: boolean | ((entry: Release) => boolean),
    ) => {
      const resolveBlocked =
        typeof isBlocked === "function" ? isBlocked : () => isBlocked;

      return (
        <div className="space-y-3">
          {hideInlineSortControls ? null : (
            <div
              className={cn(
                "flex flex-wrap items-center gap-2",
                !compact && "md:hidden",
              )}
            >
              <Button
                type="button"
                size="xs"
                variant={sortKey === "score" ? "secondary" : "outline"}
                onClick={() => handleSort("score")}
              >
                Score {renderSortIcon("score")}
              </Button>
              <Button
                type="button"
                size="xs"
                variant={sortKey === "size" ? "secondary" : "outline"}
                onClick={() => handleSort("size")}
              >
                Size {renderSortIcon("size")}
              </Button>
            </div>
          )}

          <div className={cn("space-y-2", !compact && "md:hidden")}>
            {entries.map((result) => (
              <SearchResultRow
                key={`${result.source}-${result.title}-${result.link}`}
                result={result}
                onQueue={onQueue}
                onQueueAdditional={
                  onQueueAdditional &&
                  (!canQueueAdditional || canQueueAdditional(result))
                    ? onQueueAdditional
                    : undefined
                }
                blocked={resolveBlocked(result)}
                disabled={disabled}
                requireCandidateToken={requireCandidateToken}
                mobile
              />
            ))}
          </div>

          <div
            className={cn(
              "hidden overflow-x-auto rounded-md border border-border bg-background/30",
              !compact && "md:block",
            )}
          >
            <table className="w-full table-fixed text-left">
              <thead className="bg-card/80">
                <tr>
                  <th className="w-[62%] px-4 py-3 text-base font-bold text-foreground">
                    Release
                  </th>
                  <th className="w-[8%] px-4 py-3 text-center text-base font-bold text-foreground">
                    <button
                      type="button"
                      className="inline-flex w-full items-center justify-center gap-1"
                      onClick={() => handleSort("score")}
                    >
                      Score {renderSortIcon("score")}
                    </button>
                  </th>
                  <th className="w-[10%] px-4 py-3 text-center text-base font-bold text-foreground">
                    <button
                      type="button"
                      className="inline-flex w-full items-center justify-center gap-1"
                      onClick={() => handleSort("size")}
                    >
                      Size {renderSortIcon("size")}
                    </button>
                  </th>
                  <th className="w-[20%] px-4 py-3 text-center text-base font-bold text-foreground">
                    Actions
                  </th>
                </tr>
              </thead>
              <tbody>
                {entries.map((result) => (
                  <SearchResultRow
                    key={`${result.source}-${result.title}-${result.link}`}
                    result={result}
                    onQueue={onQueue}
                    onQueueAdditional={
                      onQueueAdditional &&
                      (!canQueueAdditional || canQueueAdditional(result))
                        ? onQueueAdditional
                        : undefined
                    }
                    blocked={resolveBlocked(result)}
                    disabled={disabled}
                    requireCandidateToken={requireCandidateToken}
                  />
                ))}
              </tbody>
            </table>
          </div>
        </div>
      );
    },
    [
      canQueueAdditional,
      compact,
      disabled,
      handleSort,
      hideInlineSortControls,
      onQueue,
      onQueueAdditional,
      renderSortIcon,
      requireCandidateToken,
      sortKey,
    ],
  );

  const renderSelectedTitleList = React.useCallback(
    (
      entries: Release[],
      isBlocked: boolean | ((entry: Release) => boolean),
    ) => {
      const resolveBlocked =
        typeof isBlocked === "function" ? isBlocked : () => isBlocked;

      return (
        <div>
          {entries.map((result) => (
            <SearchResultRow
              key={`${result.source}-${result.title}-${result.link}`}
              result={result}
              onQueue={onQueue}
              onQueueAdditional={
                onQueueAdditional &&
                (!canQueueAdditional || canQueueAdditional(result))
                  ? onQueueAdditional
                  : undefined
              }
              blocked={resolveBlocked(result)}
              disabled={disabled}
              requireCandidateToken={requireCandidateToken}
              presentation="selected-title"
            />
          ))}
        </div>
      );
    },
    [
      canQueueAdditional,
      disabled,
      onQueue,
      onQueueAdditional,
      requireCandidateToken,
    ],
  );

  if (presentation === "selected-title") {
    return (
      <div>
        {results.length === 0 ? (
          <p className="p-4 text-sm text-muted-foreground">
            {t("nzb.noConsideredResults")}
          </p>
        ) : (
          renderSelectedTitleList(sortedInlineResults, isBlockedEntry)
        )}
      </div>
    );
  }

  if (showBlockedInline) {
    return (
      <div className="space-y-3">
        {results.length === 0 ? (
          <p className="text-sm text-muted-foreground">
            {t("nzb.noConsideredResults")}
          </p>
        ) : (
          renderTable(sortedInlineResults, isBlockedEntry)
        )}
      </div>
    );
  }

  return (
    <div className="space-y-3">
      {considered.length === 0 ? (
        <p className="text-sm text-muted-foreground">
          {t("nzb.noConsideredResults")}
        </p>
      ) : (
        renderTable(sortedConsidered, false)
      )}
      {blocked.length > 0 ? (
        <div>
          <button
            type="button"
            className="flex items-center gap-1 text-xs text-muted-foreground hover:text-card-foreground"
            onClick={() => setShowBlocked((v) => !v)}
          >
            {showBlocked ? (
              <ChevronUp className="h-3 w-3" />
            ) : (
              <ChevronDown className="h-3 w-3" />
            )}
            {t("nzb.blockedResults", { count: blocked.length })}
          </button>
          {showBlocked ? (
            <div className="mt-2 space-y-2">
              {renderTable(sortedBlocked, true)}
            </div>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}
