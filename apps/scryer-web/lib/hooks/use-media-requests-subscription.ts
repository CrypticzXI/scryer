import { useEffect, useRef } from "react";

import { mediaRequestsChangedSubscription } from "@/lib/graphql/queries";

import { useDeferredWsSubscription } from "@/lib/hooks/use-deferred-ws-subscription";

export function useMediaRequestsSubscription(
  onChanged: () => void,
  options?: { pause?: boolean },
) {
  const onChangedRef = useRef(onChanged);
  useEffect(() => {
    onChangedRef.current = onChanged;
  });

  useDeferredWsSubscription({
    enabled: !(options?.pause ?? false),
    requestKey: "mediaRequestsChanged",
    request: { query: mediaRequestsChangedSubscription },
    onNext() {
      onChangedRef.current();
    },
    onError(err) {
      console.error("[media-requests] subscription error:", err);
    },
  });
}
