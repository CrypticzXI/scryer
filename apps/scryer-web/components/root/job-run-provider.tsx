import * as React from "react";
import { useClient } from "urql";

import { toast } from "@/components/ui/sonner";
import { useTranslate } from "@/lib/context/translate-context";
import {
  activeJobRunsQuery,
  jobRunEventsSubscription,
} from "@/lib/graphql/queries";
import { useDeferredWsSubscription } from "@/lib/hooks/use-deferred-ws-subscription";
import {
  isTerminalJobRunStatus,
  normalizeJobRun,
  preferJobRunSnapshot,
} from "@/lib/utils/job-runs";
import type { JobKey, JobRun } from "@/lib/types";

const TERMINAL_TOAST_DURATION_MS = 6_000;

type JobRunToastContextValue = {
  registerInteractiveJobRun: (run: JobRun) => void;
};

const JobRunToastContext = React.createContext<JobRunToastContextValue | null>(null);

function usesDedicatedLibraryScanToast(jobKey: JobKey): boolean {
  return (
    jobKey === "library_scan_movies" ||
    jobKey === "library_scan_series" ||
    jobKey === "library_scan_anime" ||
    jobKey === "background_library_refresh_movies" ||
    jobKey === "background_library_refresh_series" ||
    jobKey === "background_library_refresh_anime"
  );
}

export function JobRunProvider({ children }: { children: React.ReactNode }) {
  const client = useClient();
  const t = useTranslate();
  const [runsById, setRunsById] = React.useState<Record<string, JobRun>>({});
  const dismissTimersRef = React.useRef<Record<string, ReturnType<typeof setTimeout>>>({});
  const interactiveRunIdsRef = React.useRef(new Set<string>());

  const upsertRun = React.useCallback((run: JobRun) => {
    setRunsById((current) => ({
      ...current,
      [run.id]: preferJobRunSnapshot(current[run.id], run),
    }));
  }, []);

  const registerInteractiveJobRun = React.useCallback((run: JobRun) => {
    interactiveRunIdsRef.current.add(run.id);
    upsertRun(run);
  }, [upsertRun]);

  React.useEffect(() => {
    let cancelled = false;
    (async () => {
      const { data, error } = await client.query(activeJobRunsQuery, {}).toPromise();
      if (cancelled || error) {
        if (error) {
          console.error("[job-runs] failed to load active jobs:", error);
        }
        return;
      }

      const rawRuns: unknown[] = Array.isArray(data?.activeJobRuns) ? data.activeJobRuns : [];
      const normalizedRuns = rawRuns
        .map(normalizeJobRun)
        .filter((run): run is JobRun => run !== null);

      setRunsById((current) => {
        const next = { ...current };
        for (const run of normalizedRuns) {
          next[run.id] = preferJobRunSnapshot(next[run.id], run);
        }
        return next;
      });
    })();

    return () => {
      cancelled = true;
    };
  }, [client]);

  useDeferredWsSubscription<{ data?: { jobRunEvents?: unknown } }>({
    requestKey: "jobRunEvents",
    request: { query: jobRunEventsSubscription },
    onNext(result) {
      const normalized = normalizeJobRun(result.data?.jobRunEvents);
      if (normalized) {
        upsertRun(normalized);
      }
    },
    onError(error) {
      console.error("[job-runs] subscription error:", error);
    },
  });

  React.useEffect(() => {
    const idsToPrune: string[] = [];

    for (const run of Object.values(runsById)) {
      const isInteractiveRun = interactiveRunIdsRef.current.has(run.id);
      const shouldRender =
        isInteractiveRun && !usesDedicatedLibraryScanToast(run.jobKey);

      if (!shouldRender) {
        if (isTerminalJobRunStatus(run.status)) {
          idsToPrune.push(run.id);
        }
        continue;
      }

      if (isTerminalJobRunStatus(run.status)) {
        const existingTimer = dismissTimersRef.current[run.id];
        if (!existingTimer) {
          dismissTimersRef.current[run.id] = setTimeout(() => {
            setRunsById((current) => {
              const next = { ...current };
              delete next[run.id];
              return next;
            });
            interactiveRunIdsRef.current.delete(run.id);
            delete dismissTimersRef.current[run.id];
          }, TERMINAL_TOAST_DURATION_MS);
        }
      } else if (dismissTimersRef.current[run.id]) {
        clearTimeout(dismissTimersRef.current[run.id]);
        delete dismissTimersRef.current[run.id];
      }

      const description =
        run.errorText ??
        run.summaryText ??
        (isTerminalJobRunStatus(run.status)
          ? t("jobs.runSummaryCompleted")
          : t("jobs.runSummaryRunning"));

      if (run.status === "failed") {
        toast.error(run.displayName, {
          id: run.id,
          description,
          duration: TERMINAL_TOAST_DURATION_MS,
        });
        continue;
      }

      if (run.status === "warning") {
        toast.warning(run.displayName, {
          id: run.id,
          description,
          duration: TERMINAL_TOAST_DURATION_MS,
        });
        continue;
      }

      if (run.status === "completed") {
        toast.success(run.displayName, {
          id: run.id,
          description,
          duration: TERMINAL_TOAST_DURATION_MS,
        });
        continue;
      }

      toast.loading(run.displayName, {
        id: run.id,
        description,
        duration: Infinity,
      });
    }

    if (idsToPrune.length > 0) {
      setRunsById((current) => {
        const next = { ...current };
        for (const id of idsToPrune) {
          delete next[id];
          interactiveRunIdsRef.current.delete(id);
        }
        return next;
      });
    }
  }, [runsById, t]);

  React.useEffect(
    () => () => {
      for (const timer of Object.values(dismissTimersRef.current)) {
        clearTimeout(timer);
      }
    },
    [],
  );

  const contextValue = React.useMemo<JobRunToastContextValue>(() => ({
    registerInteractiveJobRun,
  }), [registerInteractiveJobRun]);

  return (
    <JobRunToastContext.Provider value={contextValue}>
      {children}
    </JobRunToastContext.Provider>
  );
}

export function useJobRunToasts(): JobRunToastContextValue {
  const context = React.useContext(JobRunToastContext);

  if (!context) {
    throw new Error("useJobRunToasts must be used within a JobRunProvider");
  }

  return context;
}
