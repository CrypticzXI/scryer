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
                ? session.summary
                  ? t("settings.libraryScanCanceledSummary", {
                      imported: session.summary.imported,
                      skipped: session.summary.skipped,
                      unmatched: session.summary.unmatched,
                    })
                  : t("settings.libraryScanCanceled")
              : session.summary
                ? t("settings.libraryScanSummary", {
                    imported: session.summary.imported,
                    skipped: session.summary.skipped,
                    unmatched: session.summary.unmatched,
                  })
                : t("settings.libraryScanCompleted")}
          </p>
        ) : null}
      </div>
    </div>
  );
}
