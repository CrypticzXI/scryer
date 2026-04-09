import { CheckCircle2, CircleAlert, Loader2 } from "lucide-react";

import { ActivityProgressBar } from "@/components/views/activity-progress-bar";
import type { Translate } from "@/components/root/types";
import type { LibraryScanPhaseProgress, LibraryScanProgress } from "@/lib/types";

function facetLabel(facet: LibraryScanProgress["facet"], t: Translate): string {
  switch (facet) {
    case "movie":
      return t("nav.movies");
    case "tv":
      return t("nav.series");
    case "anime":
      return t("nav.anime");
    default:
      return t("settings.libraryScanTitle");
  }
}

function isTerminal(status: LibraryScanProgress["status"]): boolean {
  return status === "completed" || status === "warning" || status === "failed";
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
  if (status === "completed" || status === "warning") {
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
  const terminal = isTerminal(session.status);
  const titleMatchPercent = percentForPhase(
    session.titleMatchProgress,
    session.titleMatchTotalKnown,
    terminal,
  );
  const hydrationPercent = percentForPhase(
    session.hydrationProgress,
    session.hydrationTotalKnown,
    terminal,
  );
  const mediaAnalysisPercent = percentForPhase(
    session.mediaAnalysisProgress,
    session.mediaAnalysisTotalKnown,
    terminal,
  );
  const hydrationDone =
    session.hydrationProgress.completed + session.hydrationProgress.failed;
  const hydrationActive =
    session.hydrationProgress.total > 0 &&
    hydrationDone < session.hydrationProgress.total;
  const titleMatchIndeterminate = !terminal && !session.titleMatchTotalKnown;
  const hydrationIndeterminate =
    !terminal &&
    (!session.hydrationTotalKnown || (hydrationActive && hydrationDone === 0));
  const mediaAnalysisIndeterminate =
    !terminal && !session.mediaAnalysisTotalKnown;

  return (
    <div className="w-[min(26rem,calc(100vw-3rem))] p-4">
      <div className="min-w-0 space-y-3">
        <div className="space-y-1">
          <div className="flex items-center gap-2">
            <p className="text-sm font-semibold text-foreground">
              {titleOverride ?? t("settings.libraryScanToastTitle", {
                facet: facetLabel(session.facet, t),
              })}
            </p>
            {statusIcon(session.status)}
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
              {t("settings.libraryScanFetchingMetadata")}
            </p>
            <ActivityProgressBar
              percent={hydrationPercent}
              indeterminate={hydrationIndeterminate}
              remainingLabel={phaseLabel(
                session.status,
                session.hydrationProgress,
                session.hydrationTotalKnown,
                t("settings.libraryScanNoMetadataNeeded"),
                t,
              )}
              colorClass="bg-emerald-500"
            />
          </div>

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
