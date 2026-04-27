import { useEffect, useMemo, useRef } from "react";

import { useReactiveRefresh } from "@/lib/context/reactive-refresh-context";
import { useActivityEventStream } from "@/lib/hooks/use-activity-event-stream";
import { useImportHistorySubscription } from "@/lib/hooks/use-import-history-subscription";
import type {
  TitleOverviewDownloadFeedbackSnapshot,
  TitleOverviewNativeSnapshot,
} from "@/lib/title-overview-loader";

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

const HYDRATION_STARTED_KIND = "metadata_hydration_started";
const HYDRATION_COMPLETED_KIND = "metadata_hydration_completed";
const HYDRATION_FAILED_KIND = "metadata_hydration_failed";

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
  const applyNativeSnapshotRef = useRef(applyNativeSnapshot);
  const applyDownloadFeedbackSnapshotRef = useRef(applyDownloadFeedbackSnapshot);
  const onHydrationStartedRef = useRef(onHydrationStarted);
  const onHydrationCompletedRef = useRef(onHydrationCompleted);
  const onHydrationFailedRef = useRef(onHydrationFailed);

  useEffect(() => {
    applyNativeSnapshotRef.current = applyNativeSnapshot;
    applyDownloadFeedbackSnapshotRef.current = applyDownloadFeedbackSnapshot;
    onHydrationStartedRef.current = onHydrationStarted;
    onHydrationCompletedRef.current = onHydrationCompleted;
    onHydrationFailedRef.current = onHydrationFailed;
  });

  const queueNativeRefresh = () => {
    if (!titleId) {
      return;
    }

    queueTitleOverviewNativeRefresh({
      titleId,
      blocklistLimit,
      apply(snapshot) {
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
    if (!titleId || !downloadFeedbackEnabled) {
      return;
    }

    queueTitleOverviewDownloadFeedbackRefresh({
      titleId,
      apply(snapshot) {
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

  const activityKinds = useMemo(
    () =>
      new Set([
        ...importKinds,
        HYDRATION_STARTED_KIND,
        HYDRATION_COMPLETED_KIND,
        HYDRATION_FAILED_KIND,
      ]),
    [importKinds],
  );

  useActivityEventStream({
    kinds: activityKinds,
    titleId,
    pause,
    onEvent(activity) {
      switch (activity.kind) {
        case HYDRATION_STARTED_KIND:
          onHydrationStartedRef.current?.();
          return;
        case HYDRATION_COMPLETED_KIND:
          onHydrationCompletedRef.current?.();
          queueRefresh();
          return;
        case HYDRATION_FAILED_KIND:
          onHydrationFailedRef.current?.();
          return;
        default:
          queueRefresh();
      }
    },
  });

  useImportHistorySubscription(queueRefresh, { pause });
}
