import { useEffect, useLayoutEffect, useMemo, useRef } from "react";

import { useReactiveRefresh } from "@/lib/context/reactive-refresh-context";
import { useActivityEventStream } from "@/lib/hooks/use-activity-event-stream";
import type {
  TitleOverviewDownloadFeedbackSnapshot,
  TitleSidePanelOverviewSnapshot,
} from "@/lib/title-overview-loader";
import type { TitleSidePanelOverviewProjection } from "@/lib/graphql/queries";
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
  projection: TitleSidePanelOverviewProjection;
  applyOverviewSnapshot: (
    snapshot: TitleSidePanelOverviewSnapshot<
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
  projection,
  applyOverviewSnapshot,
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
    queueTitleSidePanelOverviewRefresh,
  } = useReactiveRefresh();
  const titleIdRef = useRef(titleId ?? null);
  const applyOverviewSnapshotRef = useRef(applyOverviewSnapshot);
  const applyDownloadFeedbackSnapshotRef = useRef(applyDownloadFeedbackSnapshot);
  const onHydrationStartedRef = useRef(onHydrationStarted);
  const onHydrationCompletedRef = useRef(onHydrationCompleted);
  const onHydrationFailedRef = useRef(onHydrationFailed);
  const bulkOverviewRefreshTimerRef = useRef<ReturnType<typeof setTimeout> | null>(
    null,
  );
  const bulkOverviewRefreshStartedAtRef = useRef<number | null>(null);

  useLayoutEffect(() => {
    titleIdRef.current = titleId ?? null;
  }, [titleId]);

  useEffect(() => {
    applyOverviewSnapshotRef.current = applyOverviewSnapshot;
    applyDownloadFeedbackSnapshotRef.current = applyDownloadFeedbackSnapshot;
    onHydrationStartedRef.current = onHydrationStarted;
    onHydrationCompletedRef.current = onHydrationCompleted;
    onHydrationFailedRef.current = onHydrationFailed;
  });

  const queueOverviewRefresh = () => {
    const requestedTitleId = titleId;
    if (!requestedTitleId) {
      return;
    }

    queueTitleSidePanelOverviewRefresh({
      titleId: requestedTitleId,
      blocklistLimit,
      projection,
      apply(snapshot) {
        if (titleIdRef.current !== requestedTitleId) {
          return;
        }
        applyOverviewSnapshotRef.current(
          snapshot as TitleSidePanelOverviewSnapshot<
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
    queueOverviewRefresh();
    queueDownloadFeedbackRefresh();
  };

  const clearBulkOverviewRefresh = () => {
    if (bulkOverviewRefreshTimerRef.current) {
      clearTimeout(bulkOverviewRefreshTimerRef.current);
      bulkOverviewRefreshTimerRef.current = null;
    }
    bulkOverviewRefreshStartedAtRef.current = null;
  };

  const queueBulkOverviewRefresh = () => {
    if (!titleId) {
      return;
    }

    const now = Date.now();
    const startedAt = bulkOverviewRefreshStartedAtRef.current ?? now;
    bulkOverviewRefreshStartedAtRef.current = startedAt;
    const elapsedMs = now - startedAt;
    const delayMs =
      elapsedMs >= TITLE_OVERVIEW_BULK_REFRESH_MAX_WAIT_MS
        ? 0
        : Math.min(
            TITLE_OVERVIEW_BULK_REFRESH_DEBOUNCE_MS,
            TITLE_OVERVIEW_BULK_REFRESH_MAX_WAIT_MS - elapsedMs,
          );

    if (bulkOverviewRefreshTimerRef.current) {
      clearTimeout(bulkOverviewRefreshTimerRef.current);
    }
    bulkOverviewRefreshTimerRef.current = setTimeout(() => {
      bulkOverviewRefreshTimerRef.current = null;
      bulkOverviewRefreshStartedAtRef.current = null;
      queueOverviewRefresh();
    }, delayMs);
  };

  useEffect(
    () => () => {
      if (bulkOverviewRefreshTimerRef.current) {
        clearTimeout(bulkOverviewRefreshTimerRef.current);
        bulkOverviewRefreshTimerRef.current = null;
      }
      bulkOverviewRefreshStartedAtRef.current = null;
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
          clearBulkOverviewRefresh();
          queueOverviewRefresh();
          return;
        case "hydrationFailed":
          onHydrationFailedRef.current?.();
          return;
        case "refresh":
          if (refreshPlan.mode === "bulk") {
            queueBulkOverviewRefresh();
            return;
          }

          clearBulkOverviewRefresh();
          if (refreshPlan.downloadFeedback) {
            queueRefresh();
            return;
          }
          queueOverviewRefresh();
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
