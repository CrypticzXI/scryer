import * as React from "react";
import { CheckCircle2, CircleAlert, Loader2 } from "lucide-react";
import { useClient } from "urql";

import { ActivityProgressBar } from "@/components/views/activity-progress-bar";
import { Button } from "@/components/ui/button";
import { toast } from "@/components/ui/sonner";
import type { Translate } from "@/components/root/types";
import { cancelLibraryScanMutation } from "@/lib/graphql/mutations";
import type { LibraryScanPhaseProgress, LibraryScanProgress } from "@/lib/types";

function facetLabel(facet: LibraryScanProgress["facet"], t: Translate): string {
  switch (facet) {
    case "movie":
      return t("nav.movies");
    case "series":
      return t("nav.series");
    case "anime":
      return t("nav.anime");
    default:
      return t("settings.libraryScanTitle");
  }
}

function isTerminal(status: LibraryScanProgress["status"]): boolean {
  return (
    status === "completed" ||
    status === "canceled" ||
    status === "warning" ||
    status === "failed"
  );
}

function percentForPhase(
  phase: LibraryScanPhaseProgress,
  totalKnown: boolean,
  terminal: boolean,
): number {
  if (phase.total <= 0) {
    return terminal || totalKnown ? 100 : 0;
  }
  const done = phase.completed + phase.failed;
  return Math.max(0, Math.min(100, Math.round((done / phase.total) * 100)));
}

function formatEtaCountdown(totalSeconds: number): string {
  const seconds = Math.max(1, Math.round(totalSeconds));
  const minutes = Math.floor(seconds / 60);
  const secs = seconds % 60;
  return `${String(minutes).padStart(2, "0")}:${String(secs).padStart(2, "0")}`;
}

function phaseLabel(
  status: LibraryScanProgress["status"],
  phase: LibraryScanPhaseProgress,
  totalKnown: boolean,
  emptyLabel: string,
  t: Translate,
): string {
  if (!totalKnown && !isTerminal(status)) {
    return t("settings.libraryScanProgressCalculatingTotal");
  }
  if (phase.total <= 0) {
    return isTerminal(status) || totalKnown
      ? emptyLabel
      : t("settings.libraryScanProgressPending");
  }
  const done = Math.min(phase.total, phase.completed + phase.failed);
  return t("settings.libraryScanProgressCount", {
    current: done,
    total: phase.total,
  });
}

function statusIcon(status: LibraryScanProgress["status"]) {
  if (status === "failed") {
    return <CircleAlert className="h-4 w-4 text-red-400" />;
  }
  if (status === "warning") {
    return <CircleAlert className="h-4 w-4 text-amber-400" />;
  }
  if (status === "canceled") {
    return <CircleAlert className="h-4 w-4 text-amber-400" />;
  }
  if (status === "completed") {
    return <CheckCircle2 className="h-4 w-4 text-emerald-400" />;
  }
  return <Loader2 className="h-4 w-4 animate-spin text-sky-400" />;
}

function scanSummaryText(
  summary: LibraryScanProgress["summary"],
  t: Translate,
  canceled = false,
): string | null {
  if (!summary) {
    return null;
  }

  return t(
    canceled
      ? "settings.libraryScanCanceledSummary"
      : "settings.libraryScanSummary",
    {
      imported: summary.imported,
      skipped: summary.skipped,
      unmatched: summary.unmatched,
    },
  );
}

export function LibraryScanToast({
  session,
  t,
  titleOverride,
}: {
  session: LibraryScanProgress;
  t: Translate;
  titleOverride?: string;
}) {
  const client = useClient();
  const [cancelPending, setCancelPending] = React.useState(false);
  const [nowMs, setNowMs] = React.useState(() => Date.now());
  const mediaAnalysisStartedAtRef = React.useRef<number | null>(null);
  const terminal = isTerminal(session.status);
  const titleMatchPercent = percentForPhase(
    session.titleMatchProgress,
    session.titleMatchTotalKnown,
    terminal,
  );
  const mediaAnalysisPercent = percentForPhase(
    session.mediaAnalysisProgress,
    session.mediaAnalysisTotalKnown,
    terminal,
  );
  const titleMatchIndeterminate = !terminal && !session.titleMatchTotalKnown;
  const mediaAnalysisIndeterminate =
    !terminal && !session.mediaAnalysisTotalKnown;
  const showCancel = session.mode === "full" && !terminal;
  const mediaAnalysisDone =
    session.mediaAnalysisProgress.completed + session.mediaAnalysisProgress.failed;
  const mediaAnalysisTotal = session.mediaAnalysisProgress.total;
  const mediaAnalysisTotalKnown = session.mediaAnalysisTotalKnown;
  const mediaAnalysisRemaining = Math.max(0, mediaAnalysisTotal - mediaAnalysisDone);
  const mediaAnalysisActive =
    !terminal &&
    mediaAnalysisTotal > 0 &&
    mediaAnalysisRemaining > 0;

  React.useEffect(() => {
    if (!mediaAnalysisActive) {
      mediaAnalysisStartedAtRef.current = null;
      return;
    }
    if (mediaAnalysisStartedAtRef.current == null) {
      mediaAnalysisStartedAtRef.current = Date.parse(session.updatedAt) || Date.now();
    }
  }, [mediaAnalysisActive, session.updatedAt]);

  React.useEffect(() => {
    if (!mediaAnalysisActive) {
      return;
    }
    const timer = window.setInterval(() => {
      setNowMs(Date.now());
    }, 1000);
    return () => {
      window.clearInterval(timer);
    };
  }, [mediaAnalysisActive]);

  const mediaAnalysisElapsedMs = mediaAnalysisStartedAtRef.current == null
    ? 0
    : Math.max(0, nowMs - mediaAnalysisStartedAtRef.current);
  const shouldShowEta =
    showCancel &&
    mediaAnalysisActive &&
    session.mediaAnalysisTotalKnown &&
    mediaAnalysisDone > 0 &&
    mediaAnalysisElapsedMs >= 30_000;
  const mediaAnalysisEtaSeconds = shouldShowEta
    ? (mediaAnalysisElapsedMs / 1000 / mediaAnalysisDone) * mediaAnalysisRemaining
    : null;
  const etaCountdown = mediaAnalysisEtaSeconds != null && Number.isFinite(mediaAnalysisEtaSeconds)
    ? formatEtaCountdown(mediaAnalysisEtaSeconds)
    : null;

  const handleCancel = React.useCallback(async () => {
    if (cancelPending) {
      return;
    }

    setCancelPending(true);
    try {
      const result = await client
        .mutation(cancelLibraryScanMutation, {
          input: {
            sessionId: session.sessionId,
          },
        })
        .toPromise();
      if (result.error) {
        throw result.error;
      }
    } catch (error) {
      setCancelPending(false);
      toast.error(
        error instanceof Error
          ? error.message
          : t("settings.libraryScanCancelFailed"),
      );
    }
  }, [cancelPending, client, session.sessionId, t]);

  return (
    <div className="w-[min(26rem,calc(100vw-3rem))] p-4">
      <div className="min-w-0 space-y-3">
        <div className="space-y-1">
          <div className="flex items-start justify-between gap-3">
            <div className="flex min-w-0 items-center gap-2">
              <p className="text-sm font-semibold text-foreground">
                {titleOverride ?? t("settings.libraryScanToastTitle", {
                  facet: facetLabel(session.facet, t),
                })}
              </p>
              {statusIcon(session.status)}
            </div>
            {showCancel ? (
              <div className="flex shrink-0 items-center gap-2">
                {etaCountdown ? (
                  <span className="text-[11px] font-medium tabular-nums text-muted-foreground">
                    {etaCountdown}
                  </span>
                ) : null}
                <Button
                  type="button"
                  variant="destructive"
                  size="sm"
                  className="h-7 shrink-0 px-2 text-xs"
                  onClick={handleCancel}
                  disabled={cancelPending}
                >
                  {t("settings.libraryScanCancel")}
                </Button>
              </div>
            ) : null}
          </div>
          <p className="text-xs text-muted-foreground">
            {session.foundTitles > 0 || terminal
              ? t("settings.libraryScanFoundTitles", {
                  count: session.foundTitles,
                })
              : t("settings.libraryScanDiscovering")}
          </p>
        </div>

        <div className="space-y-3">
          <div className="space-y-1">
            <p className="text-xs font-medium text-foreground">
              {t("settings.libraryScanTitleMatch")}
            </p>
            <ActivityProgressBar
              percent={titleMatchPercent}
              indeterminate={titleMatchIndeterminate}
              remainingLabel={phaseLabel(
                session.status,
                session.titleMatchProgress,
                session.titleMatchTotalKnown,
                t("settings.libraryScanNoTitleMatchNeeded"),
                t,
              )}
              colorClass="bg-sky-500"
            />
          </div>

          <div className="space-y-1">
            <p className="text-xs font-medium text-foreground">
              {t("settings.libraryScanFilesScanned")}
            </p>
            <ActivityProgressBar
              percent={mediaAnalysisPercent}
              indeterminate={mediaAnalysisIndeterminate}
              remainingLabel={phaseLabel(
                session.status,
                session.mediaAnalysisProgress,
                session.mediaAnalysisTotalKnown,
                t("settings.libraryScanNoFilesToScan"),
                t,
              )}
              colorClass="bg-purple-500"
            />
          </div>
        </div>

        {terminal ? (
          <p className="text-xs text-muted-foreground">
            {session.status === "failed"
              ? t("settings.libraryScanFailed")
              : session.status === "warning"
                ? session.warningMessage || t("settings.libraryScanCompletedWithWarnings")
              : session.status === "canceled"
                ? scanSummaryText(session.summary, t, true) ??
                  t("settings.libraryScanCanceled")
              : scanSummaryText(session.summary, t) ??
                t("settings.libraryScanCompleted")}
          </p>
        ) : null}
      </div>
    </div>
  );
}
