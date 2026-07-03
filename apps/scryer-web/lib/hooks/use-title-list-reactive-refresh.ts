import { useEffect, useMemo, useRef } from "react";

import { useReactiveRefresh } from "@/lib/context/reactive-refresh-context";
import { useActivityEventStream } from "@/lib/hooks/use-activity-event-stream";
import type { TitleRecord } from "@/lib/types";
import { TITLE_OVERVIEW_REFRESH_KINDS } from "@/lib/utils/title-overview-refresh-kinds";

type UseTitleListReactiveRefreshOptions = {
  facet?: string | null;
  pause?: boolean;
  onTitleRefreshed: (
    titleId: string,
    title: TitleRecord | null,
    requestEpoch: number,
  ) => void;
};

const TITLE_ACTIVITY_KINDS = [
  "title_added",
  "title_updated",
  "metadata_hydration_completed",
  "metadata_hydration_failed",
] as const;

// Canonical reactive bridge for catalog tables. Title-list consumers should
// react to title lifecycle events plus media-file changes that affect list rows.
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

  const kinds = useMemo(
    () =>
      new Set<string>([
        ...TITLE_ACTIVITY_KINDS,
        ...Array.from(TITLE_OVERVIEW_REFRESH_KINDS),
      ]),
    [],
  );

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
        apply(title, requestEpoch) {
          onTitleRefreshedRef.current(titleId, title, requestEpoch);
        },
        onError(error) {
          console.error("[title-list-reactive-refresh] refresh failed:", error);
        },
      });
    },
  });
}
