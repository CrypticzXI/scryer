import { useEffect, useId, useRef } from "react";

import { useReactiveRefreshOptional } from "@/lib/context/reactive-refresh-context";
import { settingsChangedSubscription } from "@/lib/graphql/queries";
import { useDeferredWsSubscription } from "@/lib/hooks/use-deferred-ws-subscription";
import { forEventTypes } from "@/lib/reactive/domain-event-feed";

// The unified `domainEventFeed` emits a single, coarse `configuration_changed`
// event (`{ resource_type, resource_id, action }`) rather than the granular
// setting-key list the legacy `settingsChanged` poke carried. To preserve every
// consumer's refresh without regressing to "never refetch", we report a match
// for ANY key: settings consumers refetch on any configuration change. This is
// broader than the old key-scoped refetch but bounded — configuration changes
// are infrequent and admin-driven, and the reactive-refresh registry coalesces
// bursts through its shared debounce.
const ANY_SETTINGS_KEY_CHANGED: string[] = Object.assign([] as string[], {
  includes: () => true,
  some: () => true,
});

/**
 * Notifies consumers whenever server-side configuration changes so they can
 * selectively refetch their affected data.
 *
 * ReactiveRefresh v2: inside the authenticated shell (where the reactive
 * refresh provider is mounted) invalidation comes from the unified
 * `domainEventFeed` (`configuration_changed`) via the registry; because that
 * event is coarse, `onChanged` receives a sentinel key set that satisfies every
 * key-scoped guard. Consumers mounted ABOVE the provider (e.g. shell-level SMG
 * notices) transparently fall back to the legacy `settingsChanged` poke
 * subscription, which the server still emits.
 */
export function useSettingsSubscription(
  onChanged: (changedKeys: string[]) => void,
) {
  const reactiveRefresh = useReactiveRefreshOptional();
  const onChangedRef = useRef(onChanged);
  useEffect(() => {
    onChangedRef.current = onChanged;
  });

  const aliasId = useId();

  useEffect(() => {
    if (!reactiveRefresh) {
      return;
    }
    return reactiveRefresh.registerReactiveRefresh({
      aliasKey: `settings:${aliasId}`,
      predicate: forEventTypes("configuration_changed"),
      run: () => onChangedRef.current(ANY_SETTINGS_KEY_CHANGED),
    });
  }, [aliasId, reactiveRefresh]);

  useDeferredWsSubscription<{ data?: { settingsChanged?: string[] } }>({
    enabled: reactiveRefresh === null,
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
