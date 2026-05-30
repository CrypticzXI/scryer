import { useEffect, useRef } from "react";

import { indexersChangedSubscription } from "@/lib/graphql/queries";
import { useDeferredWsSubscription } from "@/lib/hooks/use-deferred-ws-subscription";

export function useIndexersSubscription(
  onChanged: () => void,
  options?: { pause?: boolean },
) {
  const onChangedRef = useRef(onChanged);

  useEffect(() => {
    onChangedRef.current = onChanged;
  });

  useDeferredWsSubscription<{ data?: { indexersChanged?: boolean } }>({
    enabled: !(options?.pause ?? false),
    requestKey: "indexersChanged",
    request: { query: indexersChangedSubscription },
    onNext(result) {
      if (result.data?.indexersChanged) {
        onChangedRef.current();
      }
    },
    onError(err) {
      console.error("[indexers-changed] subscription error:", err);
    },
  });
}
