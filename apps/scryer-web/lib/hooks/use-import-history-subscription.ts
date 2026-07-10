import { useEffect, useId, useRef } from "react";

import { useReactiveRefresh } from "@/lib/context/reactive-refresh-context";
import { forEventTypes } from "@/lib/reactive/domain-event-feed";

// Import + media-file lifecycle events that change the import history table.
const IMPORT_HISTORY_EVENT_TYPES = [
  "import_completed",
  "import_rejected",
  "import_requested",
  "import_recovery_completed",
  "media_file_imported",
  "media_file_analyzed",
  "media_file_renamed",
  "media_file_deleted",
  "media_file_upgraded",
  "post_processing_completed",
] as const;

/**
 * Calls `onChanged` whenever an import/media-file lifecycle event occurs so the
 * consumer can refetch the import history table.
 *
 * ReactiveRefresh v2: invalidation is sourced from the unified `domainEventFeed`
 * (via the reactive-refresh registry) instead of the legacy `importHistoryChanged`
 * poke subscription. The public signature is unchanged.
 */
export function useImportHistorySubscription(
  onChanged: () => void,
  options?: { pause?: boolean },
) {
  const { registerReactiveRefresh } = useReactiveRefresh();
  const onChangedRef = useRef(onChanged);
  useEffect(() => {
    onChangedRef.current = onChanged;
  });

  const pause = options?.pause ?? false;
  const aliasId = useId();

  useEffect(() => {
    if (pause) {
      return;
    }
    return registerReactiveRefresh({
      aliasKey: `import-history:${aliasId}`,
      predicate: forEventTypes(...IMPORT_HISTORY_EVENT_TYPES),
      run: () => onChangedRef.current(),
    });
  }, [pause, aliasId, registerReactiveRefresh]);
}
