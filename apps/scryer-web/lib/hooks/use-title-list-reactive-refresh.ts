import { useEffect, useMemo, useRef } from "react";

import { useReactiveRefresh } from "@/lib/context/reactive-refresh-context";
import { useActivityEventStream } from "@/lib/hooks/use-activity-event-stream";
import type { TitleRecord } from "@/lib/types";

type UseTitleListReactiveRefreshOptions = {
  facet?: string | null;
  pause?: boolean;
  onTitleRefreshed: (
    titleId: string,
    title: TitleRecord | null,
  ) => void;
};

const TITLE_ACTIVITY_KINDS = [
  "title_added",
  "title_updated",
  "metadata_hydration_completed",
  "metadata_hydration_failed",
] as const;

// Canonical reactive bridge for catalog tables. Title-list consumers should
// react to semantic title lifecycle events instead of workflow-specific signals.
export function useTitleListReactiveRefresh({
  facet,
  pause = false,
  onTitleRefreshed,
}: UseTitleListReactiveRefreshOptions) {
  const { queueCatalogTitleRefresh } = useReactiveRefresh();
  const onTitleRefreshedRef = useRef(onTitleRefreshed);

  useEffect(() => {
    onTitleRefreshedRef.current = onTitleRefreshed;
  });

  const kinds = useMemo(() => new Set<string>(TITLE_ACTIVITY_KINDS), []);

  useActivityEventStream({
    kinds,
    facet,
    pause,
    onEvent(activity) {
      const titleId = activity.titleId;
      if (!titleId) {
        return;
      }

      queueCatalogTitleRefresh({
        titleId,
        apply(title) {
          onTitleRefreshedRef.current(titleId, title);
        },
        onError(error) {
          console.error("[title-list-reactive-refresh] refresh failed:", error);
        },
      });
    },
  });
}
