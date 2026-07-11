import { useEffect, useRef } from "react";

import { settingsChangedSubscription } from "@/lib/graphql/queries";
import { useDeferredWsSubscription } from "@/lib/hooks/use-deferred-ws-subscription";

/**
 * Notifies consumers whenever server-side configuration changes so they can
 * selectively refetch their affected data.
 *
 * The legacy `settingsChanged` poke is the primary signal here: the settings
 * runtime emits it alongside the unified `configuration_changed` domain event
 * on every save, and it carries the real changed-key list that consumers'
 * key-scoped guards depend on. Routing this hook through the coarse
 * `domainEventFeed` event instead (with an any-key sentinel) made every
 * settings consumer refetch on every configuration change, which raced live
 * editors and in-progress form state across unrelated settings surfaces.
 */
export function useSettingsSubscription(
  onChanged: (changedKeys: string[]) => void,
  { enabled = true }: { enabled?: boolean } = {},
) {
  const onChangedRef = useRef(onChanged);
  useEffect(() => {
    onChangedRef.current = onChanged;
  });

  useDeferredWsSubscription<{ data?: { settingsChanged?: string[] } }>({
    enabled,
    requestKey: "settingsChanged",
    request: { query: settingsChangedSubscription },
    onNext(result) {
      const keys = result.data?.settingsChanged;
      if (keys?.length) {
        onChangedRef.current(keys);
      }
    },
    onError(err) {
      console.error("[settings-changed] subscription error:", err);
    },
  });
}
