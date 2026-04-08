import { useEffect, useMemo, useRef } from "react";

import { useReactiveRefresh } from "@/lib/context/reactive-refresh-context";
import { useActivityEventStream } from "@/lib/hooks/use-activity-event-stream";
import { useImportHistorySubscription } from "@/lib/hooks/use-import-history-subscription";
import type { TitleOverviewSnapshot } from "@/lib/title-overview-loader";

type UseTitleOverviewReactiveRefreshOptions<
  TTitle = unknown,
  TEvent = unknown,
  TBlocklist = unknown,
  TSubtitle = unknown,
> = {
  titleId?: string | null;
  blocklistLimit: number;
  applySnapshot: (
    snapshot: TitleOverviewSnapshot<TTitle, TEvent, TBlocklist, TSubtitle>,
  ) => void;
  importKinds: ReadonlySet<string>;
  pause?: boolean;
  onHydrationStarted?: () => void;
  onHydrationCompleted?: () => void;
  onHydrationFailed?: () => void;
};

const HYDRATION_STARTED_KIND = "metadata_hydration_started";
const HYDRATION_COMPLETED_KIND = "metadata_hydration_completed";
const HYDRATION_FAILED_KIND = "metadata_hydration_failed";

export function useTitleOverviewReactiveRefresh<
  TTitle = unknown,
  TEvent = unknown,
  TBlocklist = unknown,
  TSubtitle = unknown,
>({
  titleId,
  blocklistLimit,
  applySnapshot,
  importKinds,
  pause = false,
  onHydrationStarted,
  onHydrationCompleted,
  onHydrationFailed,
}: UseTitleOverviewReactiveRefreshOptions<
  TTitle,
  TEvent,
  TBlocklist,
  TSubtitle
>) {
  const { queueTitleOverviewRefresh } = useReactiveRefresh();
  const applySnapshotRef = useRef(applySnapshot);
  const onHydrationStartedRef = useRef(onHydrationStarted);
  const onHydrationCompletedRef = useRef(onHydrationCompleted);
  const onHydrationFailedRef = useRef(onHydrationFailed);

  useEffect(() => {
    applySnapshotRef.current = applySnapshot;
    onHydrationStartedRef.current = onHydrationStarted;
    onHydrationCompletedRef.current = onHydrationCompleted;
    onHydrationFailedRef.current = onHydrationFailed;
  });

  const queueRefresh = () => {
    if (!titleId) {
      return;
    }

    queueTitleOverviewRefresh({
      titleId,
      blocklistLimit,
      apply(snapshot) {
        applySnapshotRef.current(
          snapshot as TitleOverviewSnapshot<
            TTitle,
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
