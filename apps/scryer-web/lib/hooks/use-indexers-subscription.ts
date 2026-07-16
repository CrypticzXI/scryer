import { useEffect, useId, useRef } from "react";

import { useReactiveRefresh } from "@/lib/context/reactive-refresh-context";
import { forEventTypes } from "@/lib/reactive/domain-event-feed";

/**
 * Calls `onChanged` whenever indexer configuration changes so the consumer can
 * refetch indexer state.
 *
 * ReactiveRefresh v2: invalidation is sourced from the unified `domainEventFeed`
 * (`configuration_changed`) via the reactive-refresh registry instead of the
 * legacy `indexersChanged` poke subscription. The public signature is unchanged.
 */
export function useIndexersSubscription(
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
      aliasKey: `indexers:${aliasId}`,
      predicate: forEventTypes("CONFIGURATION_CHANGED"),
      run: () => onChangedRef.current(),
    });
  }, [pause, aliasId, registerReactiveRefresh]);
}
