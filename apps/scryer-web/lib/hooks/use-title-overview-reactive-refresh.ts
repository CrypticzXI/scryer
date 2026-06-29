import { useEffect, useLayoutEffect, useMemo, useRef } from "react";

import { useReactiveRefresh } from "@/lib/context/reactive-refresh-context";
import { useActivityEventStream } from "@/lib/hooks/use-activity-event-stream";
import type {
  TitleOverviewDownloadFeedbackSnapshot,
  TitleOverviewNativeSnapshot,
} from "@/lib/title-overview-loader";
import {
  shouldHandleTitleOverviewActivity,
  TITLE_OVERVIEW_BULK_REFRESH_DEBOUNCE_MS,
  TITLE_OVERVIEW_BULK_REFRESH_MAX_WAIT_MS,
  titleOverviewReactiveRefreshKinds,
  titleOverviewReactiveRefreshPlan,
} from "@/lib/utils/title-overview-refresh-policy";

type UseTitleOverviewReactiveRefreshOptions<
  TTitle = unknown,
  TDiagnostics = unknown,
  TEvent = unknown,
  TBlocklist = unknown,
  TSubtitle = unknown,
> = {
  titleId?: string | null;
  blocklistLimit: number;
  applyNativeSnapshot: (
    snapshot: TitleOverviewNativeSnapshot<
      TTitle,
      TDiagnostics,
      TEvent,
      TBlocklist,
      TSubtitle
    >,
  ) => void;
  applyDownloadFeedbackSnapshot: (
    snapshot: TitleOverviewDownloadFeedbackSnapshot,
  ) => void;
  importKinds: ReadonlySet<string>;
  pause?: boolean;
  downloadFeedbackEnabled?: boolean;
  onHydrationStarted?: () => void;
  onHydrationCompleted?: () => void;
  onHydrationFailed?: () => void;
};

export function useTitleOverviewReactiveRefresh<
  TTitle = unknown,
  TDiagnostics = unknown,
  TEvent = unknown,
  TBlocklist = unknown,
  TSubtitle = unknown,
>({
  titleId,
  blocklistLimit,
  applyNativeSnapshot,
  applyDownloadFeedbackSnapshot,
  importKinds,
  pause = false,
  downloadFeedbackEnabled = true,
  onHydrationStarted,
  onHydrationCompleted,
  onHydrationFailed,
}: UseTitleOverviewReactiveRefreshOptions<
  TTitle,
  TDiagnostics,
  TEvent,
  TBlocklist,
  TSubtitle
>) {
  const {
    queueTitleOverviewDownloadFeedbackRefresh,
    queueTitleOverviewNativeRefresh,
  } = useReactiveRefresh();
  const titleIdRef = useRef(titleId ?? null);
  const applyNativeSnapshotRef = useRef(applyNativeSnapshot);
  const applyDownloadFeedbackSnapshotRef = useRef(applyDownloadFeedbackSnapshot);
  const onHydrationStartedRef = useRef(onHydrationStarted);
  const onHydrationCompletedRef = useRef(onHydrationCompleted);
  const onHydrationFailedRef = useRef(onHydrationFailed);
  const bulkNativeRefreshTimerRef = useRef<ReturnType<typeof setTimeout> | null>(
    null,
  );
  const bulkNativeRefreshStartedAtRef = useRef<number | null>(null);

  useLayoutEffect(() => {
    titleIdRef.current = titleId ?? null;
  }, [titleId]);

  useEffect(() => {
    applyNativeSnapshotRef.current = applyNativeSnapshot;
    applyDownloadFeedbackSnapshotRef.current = applyDownloadFeedbackSnapshot;
    onHydrationStartedRef.current = onHydrationStarted;
    onHydrationCompletedRef.current = onHydrationCompleted;
    onHydrationFailedRef.current = onHydrationFailed;
  });

  const queueNativeRefresh = () => {
    const requestedTitleId = titleId;
    if (!requestedTitleId) {
      return;
    }

    queueTitleOverviewNativeRefresh({
      titleId: requestedTitleId,
      blocklistLimit,
      apply(snapshot) {
        if (titleIdRef.current !== requestedTitleId) {
          return;
        }
        applyNativeSnapshotRef.current(
          snapshot as TitleOverviewNativeSnapshot<
            TTitle,
            TDiagnostics,
            TEvent,
            TBlocklist,
            TSubtitle
          >,
        );
      },
      onError(error) {
        console.error("[title-overview-reactive-refresh] refresh failed:", error);
      },
    });
  };

  const queueDownloadFeedbackRefresh = () => {
    const requestedTitleId = titleId;
    if (!requestedTitleId || !downloadFeedbackEnabled) {
      return;
    }

    queueTitleOverviewDownloadFeedbackRefresh({
      titleId: requestedTitleId,
      apply(snapshot) {
        if (titleIdRef.current !== requestedTitleId) {
          return;
        }
        applyDownloadFeedbackSnapshotRef.current(snapshot);
      },
      onError(error) {
        console.error("[title-overview-reactive-refresh] feedback refresh failed:", error);
      },
    });
  };

  const queueRefresh = () => {
    queueNativeRefresh();
    queueDownloadFeedbackRefresh();
  };

  const clearBulkNativeRefresh = () => {
    if (bulkNativeRefreshTimerRef.current) {
      clearTimeout(bulkNativeRefreshTimerRef.current);
      bulkNativeRefreshTimerRef.current = null;
    }
    bulkNativeRefreshStartedAtRef.current = null;
  };

  const queueBulkNativeRefresh = () => {
    if (!titleId) {
      return;
    }

    const now = Date.now();
    const startedAt = bulkNativeRefreshStartedAtRef.current ?? now;
    bulkNativeRefreshStartedAtRef.current = startedAt;
    const elapsedMs = now - startedAt;
    const delayMs =
      elapsedMs >= TITLE_OVERVIEW_BULK_REFRESH_MAX_WAIT_MS
        ? 0
        : Math.min(
            TITLE_OVERVIEW_BULK_REFRESH_DEBOUNCE_MS,
            TITLE_OVERVIEW_BULK_REFRESH_MAX_WAIT_MS - elapsedMs,
          );

    if (bulkNativeRefreshTimerRef.current) {
      clearTimeout(bulkNativeRefreshTimerRef.current);
    }
    bulkNativeRefreshTimerRef.current = setTimeout(() => {
      bulkNativeRefreshTimerRef.current = null;
      bulkNativeRefreshStartedAtRef.current = null;
      queueNativeRefresh();
    }, delayMs);
  };

  useEffect(
    () => () => {
      if (bulkNativeRefreshTimerRef.current) {
        clearTimeout(bulkNativeRefreshTimerRef.current);
        bulkNativeRefreshTimerRef.current = null;
      }
      bulkNativeRefreshStartedAtRef.current = null;
    },
    [pause, titleId],
  );

  const activityKinds = useMemo(
    () => titleOverviewReactiveRefreshKinds(importKinds),
    [importKinds],
  );

  useActivityEventStream({
    kinds: activityKinds,
    titleId,
    pause,
    onEvent(activity) {
      if (!shouldHandleTitleOverviewActivity(titleId, activity.titleId)) {
        return;
      }

      const refreshPlan = titleOverviewReactiveRefreshPlan(
        activity.kind,
        importKinds,
      );
      switch (refreshPlan.type) {
        case "hydrationStarted":
          onHydrationStartedRef.current?.();
          return;
        case "hydrationCompleted":
          onHydrationCompletedRef.current?.();
          clearBulkNativeRefresh();
          queueNativeRefresh();
          return;
        case "hydrationFailed":
          onHydrationFailedRef.current?.();
          return;
        case "refresh":
          if (refreshPlan.mode === "bulk") {
            queueBulkNativeRefresh();
            return;
          }

          clearBulkNativeRefresh();
          if (refreshPlan.downloadFeedback) {
            queueRefresh();
            return;
          }
          queueNativeRefresh();
          return;
        case "none":
          return;
        default: {
          const exhaustiveCheck: never = refreshPlan;
          throw new Error(
            `unsupported title overview refresh plan: ${exhaustiveCheck}`,
          );
        }
      }
    },
  });
}
