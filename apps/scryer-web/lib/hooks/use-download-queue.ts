import { useCallback, useContext, useEffect, useRef, useState } from "react";
import { useClient } from "urql";

import {
  downloadQueueQuery,
  downloadQueueSubscription,
} from "@/lib/graphql/queries";
import type { DownloadQueueItem } from "@/lib/types";
import { GlobalStatusContext } from "@/lib/context/global-status-context";
import { useDeferredWsSubscription } from "@/lib/hooks/use-deferred-ws-subscription";
import {
  downloadQueueItemIdentityKey,
  isActiveQueueState,
  sortDownloadQueueItems,
} from "@/lib/utils/download-queue";
import type { DownloadActivityFilter } from "@/lib/types";

type UseDownloadQueueArgs = {
  enabled: boolean;
  includeAllActivity: boolean;
  includeHistoryOnly: boolean;
  activityFilter: DownloadActivityFilter;
  onErrorStatus?: (message: string) => void;
};

export type UseDownloadQueueResult = {
  queueItems: DownloadQueueItem[];
  queueLoading: boolean;
  queueError: string | null;
  lastRefreshedAt: Date | null;
  refreshQueue: () => Promise<void>;
};

export function useDownloadQueue({
  enabled,
  includeAllActivity,
  includeHistoryOnly,
  activityFilter,
  onErrorStatus,
}: UseDownloadQueueArgs): UseDownloadQueueResult {
  const contextGlobalStatus = useContext(GlobalStatusContext);
  const client = useClient();
  const [queueItems, setQueueItems] = useState<DownloadQueueItem[]>([]);
  const [queueLoading, setQueueLoading] = useState(false);
  const [queueError, setQueueError] = useState<string | null>(null);
  const [lastRefreshedAt, setLastRefreshedAt] = useState<Date | null>(null);
  const pollingRef = useRef<ReturnType<typeof setInterval> | null>(null);

  // Track whether the initial HTTP query has completed so the WS subscription
  // doesn't race with it and overwrite the authoritative query data.
  const [initialFetchDone, setInitialFetchDone] = useState(false);
  const initialFetchDoneRef = useRef(false);
  // Keep ref in sync for use in refreshQueue without adding it as a dep
  initialFetchDoneRef.current = initialFetchDone;

  // --- WS subscription via graphql-ws ---
  // Deferred-cleanup pattern to survive React StrictMode's fake unmount/remount.
  // On cleanup, we delay the actual unsubscribe. If the effect re-runs within
  // the grace period (StrictMode re-mount), we cancel the teardown and keep
  // the existing subscription alive.
  //
  // The subscription is gated on `initialFetchDone` so the first broadcast
  // (which may carry stale/un-enriched data) cannot overwrite the HTTP query
  // result that the user is already looking at.
  useDeferredWsSubscription<{ data?: { downloadQueue?: DownloadQueueItem[] } }>({
    enabled: enabled && !includeHistoryOnly && initialFetchDone,
    requestKey: `downloadQueue:${includeAllActivity ? 1 : 0}:${includeHistoryOnly ? 1 : 0}:${activityFilter}`,
    request: {
      query: downloadQueueSubscription,
      variables: { includeAllActivity, includeHistoryOnly, activityFilter },
    },
    onNext(result) {
      const items = result.data?.downloadQueue;
      if (!items) {
        return;
      }

      // Merge: the subscription only carries live jobs from the
      // download client. Preserve active items that are missing from the
      // latest broadcast until the next authoritative query refresh catches up.
      setQueueItems((prev) => {
        const liveIds = new Set(items.map((item) => downloadQueueItemIdentityKey(item)));
        const kept = prev.filter(
          (item) =>
            isActiveQueueState(item.state) &&
            !liveIds.has(downloadQueueItemIdentityKey(item)),
        );
        return sortDownloadQueueItems([...items, ...kept]);
      });
      setQueueError(null);
      setLastRefreshedAt(new Date());
      if (pollingRef.current) {
        clearInterval(pollingRef.current);
        pollingRef.current = null;
      }
    },
    onError(err) {
      console.error("[download-queue] subscription error:", err);
    },
  });

  // --- Query fetch (initial load + manual refresh) ---
  // The query is authoritative — it returns enriched data with import status,
  // submission linkage, and history items that the WS subscription doesn't carry.
  // To avoid wiping live socket data that may be fresher for active downloads,
  // we merge: query items win for terminal states (completed, failed,
  // import_pending) and the socket's version wins for active states if the
  // subscription is already running.
  const refreshQueue = useCallback(async () => {
    if (!enabled) {
      return;
    }
    setQueueLoading(true);
    try {
      const { data, error } = await client
        .query(downloadQueueQuery, {
          includeAllActivity,
          includeHistoryOnly,
          activityFilter,
        })
        .toPromise();
      if (error) throw error;
      const queryItems = data?.downloadQueue || [];
      // If the subscription isn't active yet (initial load), full replace.
      // Once the subscription is running, merge so we don't clobber live data.
      if (!initialFetchDoneRef.current) {
        setQueueItems(sortDownloadQueueItems(queryItems));
      } else {
        setQueueItems((prev) => {
          // Build a map of query items keyed by downloadClientItemId
          const queryMap = new Map(
            queryItems.map((i: DownloadQueueItem) => [
              downloadQueueItemIdentityKey(i),
              i,
            ]),
          );
          // Keep existing active items that the query didn't return
          // (subscription may have fresher live data)
          const merged = [...queryItems];
          for (const item of prev) {
            if (
              isActiveQueueState(item.state) &&
              !queryMap.has(downloadQueueItemIdentityKey(item))
            ) {
              merged.push(item);
            }
          }
          return sortDownloadQueueItems(merged);
        });
      }
      setQueueError(null);
      setLastRefreshedAt(new Date());
    } catch (error) {
      const message =
        error instanceof Error ? error.message : "Failed to load queue.";
      setQueueError(message);
      (onErrorStatus ?? contextGlobalStatus)?.(message);
    } finally {
      setQueueLoading(false);
    }
  }, [
    activityFilter,
    client,
    contextGlobalStatus,
    enabled,
    includeAllActivity,
    includeHistoryOnly,
    onErrorStatus,
  ]);

  // --- Initial fetch + polling for history-only mode ---
  useEffect(() => {
    if (!enabled) {
      if (pollingRef.current) {
        clearInterval(pollingRef.current);
        pollingRef.current = null;
      }
      return;
    }

    setInitialFetchDone(false);
    refreshQueue().finally(() => setInitialFetchDone(true));

    if (includeHistoryOnly) {
      pollingRef.current = setInterval(() => void refreshQueue(), 10_000);
      return () => {
        if (pollingRef.current) {
          clearInterval(pollingRef.current);
          pollingRef.current = null;
        }
      };
    }
  }, [activityFilter, enabled, includeAllActivity, includeHistoryOnly, refreshQueue]);

  return {
    queueItems,
    queueLoading,
    queueError,
    lastRefreshedAt,
    refreshQueue,
  };
}
