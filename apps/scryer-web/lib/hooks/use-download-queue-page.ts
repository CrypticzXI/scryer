import { useCallback, useContext, useEffect, useMemo, useRef, useState } from "react";
import { useClient } from "urql";

import { GlobalStatusContext } from "@/lib/context/global-status-context";
import {
  downloadQueuePageQuery,
  downloadQueueSyncSubscription,
} from "@/lib/graphql/queries";
import { useDeferredWsSubscription } from "@/lib/hooks/use-deferred-ws-subscription";
import type {
  DownloadActivityStatus,
  DownloadClientFilterOption,
  DownloadQueueItem,
  SortConfig,
} from "@/lib/types";
import {
  DOWNLOAD_QUEUE_PAGE_SIZE,
  type DownloadQueueRetainedPage,
  downloadQueueSyncRefreshRanges,
  flattenDownloadQueuePages,
  markDownloadQueuePagesStale,
  mergeDownloadQueuePageRange,
  nextContiguousDownloadQueueOffset,
  retainedDownloadQueuePageNeedsRefresh,
  shouldApplyDownloadQueuePageResponse,
  shouldRefreshDownloadQueueSync,
} from "@/lib/utils/download-queue-page";

const SYNC_DEBOUNCE_MS = 300;

type DownloadQueuePagePayload = {
  items: DownloadQueueItem[];
  hasMore: boolean;
  totalCount: number;
  availableClients: DownloadClientFilterOption[];
  revision: number;
  updatedAt: string | null;
  ready: boolean;
  stale: boolean;
};

type QueryPageOptions = {
  reset?: boolean;
  markRetainedStale?: boolean;
  minimumRevision?: number;
};

type UseDownloadQueuePageArgs = {
  enabled: boolean;
  filters: DownloadActivityStatus[];
  clientIds: string[] | null;
  scryerSubmittedOnly: boolean;
  sort: SortConfig;
  titleId?: string | null;
  onErrorStatus?: (message: string) => void;
};

export type UseDownloadQueuePageResult = {
  queueItems: DownloadQueueItem[];
  queueLoading: boolean;
  queueLoadingMore: boolean;
  queueError: string | null;
  queueHasMore: boolean;
  queueTotalCount: number;
  queueAvailableClients: DownloadClientFilterOption[];
  queueReady: boolean;
  queueStale: boolean;
  lastRefreshedAt: Date | null;
  refreshQueue: () => Promise<void>;
  loadMoreQueue: () => Promise<void>;
  setVisibleQueueOffset: (offset: number) => void;
};

export function useDownloadQueuePage({
  enabled,
  filters,
  clientIds,
  scryerSubmittedOnly,
  sort,
  titleId = null,
  onErrorStatus,
}: UseDownloadQueuePageArgs): UseDownloadQueuePageResult {
  const contextGlobalStatus = useContext(GlobalStatusContext);
  const client = useClient();
  const [pages, setPages] = useState<Map<number, DownloadQueueRetainedPage>>(new Map());
  const [queueLoading, setQueueLoading] = useState(false);
  const [queueLoadingMore, setQueueLoadingMore] = useState(false);
  const [queueError, setQueueError] = useState<string | null>(null);
  const [queueHasMore, setQueueHasMore] = useState(false);
  const [queueTotalCount, setQueueTotalCount] = useState(0);
  const [queueAvailableClients, setQueueAvailableClients] = useState<
    DownloadClientFilterOption[]
  >([]);
  const [queueReady, setQueueReady] = useState(false);
  const [queueStale, setQueueStale] = useState(false);
  const [lastRefreshedAt, setLastRefreshedAt] = useState<Date | null>(null);
  const nextOffsetRef = useRef(0);
  const visibleOffsetRef = useRef(0);
  const revisionRef = useRef(0);
  const targetRevisionRef = useRef(0);
  const scopeEpochRef = useRef(0);
  const requestSequenceRef = useRef(new Map<string, number>());
  const pagesRef = useRef<Map<number, DownloadQueueRetainedPage>>(new Map());
  const pendingSyncRef = useRef(false);
  const syncTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const lastReportedErrorRef = useRef<string | null>(null);
  const filtersKey = filters.join(",");
  const clientIdsKey = clientIds === null ? "all" : clientIds.join(",");
  const scopeKey = `${filtersKey}:${clientIdsKey}:${scryerSubmittedOnly ? 1 : 0}:${sort.key}:${sort.direction}:${titleId ?? "all"}`;
  const activeScopeKeyRef = useRef(scopeKey);

  const queueItems = useMemo(() => flattenDownloadQueuePages(pages), [pages]);

  useEffect(() => {
    activeScopeKeyRef.current = scopeKey;
  }, [scopeKey]);

  const applyPage = useCallback(
    (
      payload: DownloadQueuePagePayload,
      offset: number,
      limit: number,
      options: QueryPageOptions,
    ) => {
      const next = mergeDownloadQueuePageRange(
        pagesRef.current,
        payload.items,
        offset,
        limit,
        {
          reset: options.reset ?? false,
          revision: payload.revision,
          totalCount: payload.totalCount,
          markRetainedStale: options.markRetainedStale,
        },
      );
      pagesRef.current = next;
      setPages(next);
      nextOffsetRef.current = nextContiguousDownloadQueueOffset(next, payload.totalCount);
      setQueueHasMore(nextOffsetRef.current < payload.totalCount);
      setQueueTotalCount(payload.totalCount);
      setQueueAvailableClients(payload.availableClients);
      setQueueReady(payload.ready);
      setQueueStale(payload.stale || [...next.values()].some((page) => page.stale));
      revisionRef.current = Math.max(revisionRef.current, payload.revision);
      setLastRefreshedAt(payload.updatedAt ? new Date(payload.updatedAt) : null);
      setQueueError(null);
      lastReportedErrorRef.current = null;
    },
    [],
  );

  const queryPage = useCallback(
    async (offset: number, limit: number, options: QueryPageOptions = {}) => {
      const requestScopeKey = scopeKey;
      const requestEpoch = scopeEpochRef.current;
      const rangeKey = `${offset}:${limit}`;
      const requestSequence = (requestSequenceRef.current.get(rangeKey) ?? 0) + 1;
      requestSequenceRef.current.set(rangeKey, requestSequence);
      const minimumRevision = Math.max(
        options.minimumRevision ?? 0,
        targetRevisionRef.current,
      );

      for (let attempt = 0; attempt < 2; attempt += 1) {
        const { data, error } = await client
          .query(
            downloadQueuePageQuery,
            {
              limit,
              offset,
              filters,
              clientIds,
              scryerSubmittedOnly,
              titleId,
              sortKey: sort.key,
              sortDirection: sort.direction,
            },
            { requestPolicy: "network-only" },
          )
          .toPromise();
        if (
          activeScopeKeyRef.current !== requestScopeKey ||
          scopeEpochRef.current !== requestEpoch ||
          requestSequenceRef.current.get(rangeKey) !== requestSequence
        ) {
          return null;
        }
        if (error) {
          throw error;
        }
        const payload = data?.downloadQueuePage as DownloadQueuePagePayload | undefined;
        if (!payload) {
          throw new Error("Failed to load queue page.");
        }
        if (
          !shouldApplyDownloadQueuePageResponse(
            payload.revision,
            revisionRef.current,
            minimumRevision,
          )
        ) {
          if (payload.revision < minimumRevision && attempt === 0) {
            continue;
          }
          return null;
        }
        applyPage(payload, offset, limit, options);
        return payload;
      }
      return null;
    },
    [
      applyPage,
      client,
      clientIds,
      filters,
      scopeKey,
      scryerSubmittedOnly,
      sort.direction,
      sort.key,
      titleId,
    ],
  );

  const reportError = useCallback(
    (error: unknown) => {
      const message = error instanceof Error ? error.message : "Failed to load queue.";
      setQueueError(message);
      if (lastReportedErrorRef.current !== message) {
        lastReportedErrorRef.current = message;
        (onErrorStatus ?? contextGlobalStatus)?.(message);
      }
    },
    [contextGlobalStatus, onErrorStatus],
  );

  const refreshQueue = useCallback(async () => {
    if (!enabled) {
      pendingSyncRef.current = true;
      return;
    }
    setQueueLoading(true);
    try {
      const targetRevision = targetRevisionRef.current;
      const ranges = downloadQueueSyncRefreshRanges(
        nextOffsetRef.current,
        visibleOffsetRef.current,
      );
      let reconciled = true;
      for (const [index, range] of ranges.entries()) {
        const payload = await queryPage(range.offset, range.limit, {
          reset: index === 0 && nextOffsetRef.current === 0,
          markRetainedStale: index === 0,
          minimumRevision: targetRevision,
        });
        reconciled = reconciled && payload !== null;
      }
      if (reconciled && revisionRef.current >= targetRevision) {
        pendingSyncRef.current = false;
      }
    } catch (error) {
      reportError(error);
    } finally {
      setQueueLoading(false);
    }
  }, [enabled, queryPage, reportError]);

  const loadMoreQueue = useCallback(async () => {
    if (!enabled || queueLoadingMore || !queueHasMore) {
      return;
    }
    setQueueLoadingMore(true);
    try {
      const offset = nextOffsetRef.current;
      await queryPage(offset, DOWNLOAD_QUEUE_PAGE_SIZE, {
        minimumRevision: targetRevisionRef.current,
      });
    } catch (error) {
      reportError(error);
    } finally {
      setQueueLoadingMore(false);
    }
  }, [enabled, queryPage, queueHasMore, queueLoadingMore, reportError]);

  const scheduleSync = useCallback(() => {
    pendingSyncRef.current = true;
    if (
      !shouldRefreshDownloadQueueSync(enabled, document.visibilityState) ||
      syncTimerRef.current
    ) {
      return;
    }
    syncTimerRef.current = setTimeout(() => {
      syncTimerRef.current = null;
      void refreshQueue();
    }, SYNC_DEBOUNCE_MS);
  }, [enabled, refreshQueue]);

  useDeferredWsSubscription<{
    data?: { downloadQueueSync?: { revision: number; updatedAt: string | null } };
  }>({
    enabled,
    requestKey: `downloadQueueSync:${scopeKey}`,
    request: { query: downloadQueueSyncSubscription },
    onNext(result) {
      const sync = result.data?.downloadQueueSync;
      if (!sync || sync.revision <= targetRevisionRef.current) {
        return;
      }
      targetRevisionRef.current = sync.revision;
      const next = markDownloadQueuePagesStale(pagesRef.current, sync.revision);
      pagesRef.current = next;
      setPages(next);
      setQueueStale(true);
      scheduleSync();
    },
    onError(error) {
      console.error("[download-queue] sync subscription error:", error);
    },
  });

  useEffect(() => {
    if (!enabled) {
      return;
    }
    const requestScopeKey = scopeKey;
    scopeEpochRef.current += 1;
    requestSequenceRef.current.clear();
    nextOffsetRef.current = 0;
    visibleOffsetRef.current = 0;
    revisionRef.current = 0;
    targetRevisionRef.current = 0;
    pendingSyncRef.current = false;
    const emptyPages = new Map<number, DownloadQueueRetainedPage>();
    pagesRef.current = emptyPages;
    setPages(emptyPages);
    setQueueHasMore(false);
    setQueueTotalCount(0);
    setQueueStale(false);
    setQueueLoading(true);
    void queryPage(0, DOWNLOAD_QUEUE_PAGE_SIZE, { reset: true })
      .catch(reportError)
      .finally(() => {
        if (activeScopeKeyRef.current === requestScopeKey) {
          setQueueLoading(false);
        }
      });
  }, [enabled, queryPage, reportError, scopeKey]);

  useEffect(() => {
    if (!enabled) {
      return;
    }
    const reconcileOnVisibility = () => {
      if (document.visibilityState === "visible" && pendingSyncRef.current) {
        scheduleSync();
      }
    };
    document.addEventListener("visibilitychange", reconcileOnVisibility);
    window.addEventListener("focus", reconcileOnVisibility);
    return () => {
      document.removeEventListener("visibilitychange", reconcileOnVisibility);
      window.removeEventListener("focus", reconcileOnVisibility);
      if (syncTimerRef.current) {
        clearTimeout(syncTimerRef.current);
        syncTimerRef.current = null;
      }
    };
  }, [enabled, scheduleSync]);

  const setVisibleQueueOffset = useCallback(
    (offset: number) => {
      const normalizedOffset = Math.max(0, offset);
      visibleOffsetRef.current = normalizedOffset;
      if (!enabled || document.visibilityState !== "visible") {
        return;
      }
      if (pendingSyncRef.current) {
        scheduleSync();
        return;
      }
      if (
        retainedDownloadQueuePageNeedsRefresh(
          pagesRef.current,
          normalizedOffset,
          targetRevisionRef.current,
        )
      ) {
        const pageOffset =
          Math.floor(normalizedOffset / DOWNLOAD_QUEUE_PAGE_SIZE) * DOWNLOAD_QUEUE_PAGE_SIZE;
        void queryPage(pageOffset, DOWNLOAD_QUEUE_PAGE_SIZE, {
          minimumRevision: targetRevisionRef.current,
        }).catch(reportError);
      }
    },
    [enabled, queryPage, reportError, scheduleSync],
  );

  return {
    queueItems,
    queueLoading,
    queueLoadingMore,
    queueError,
    queueHasMore,
    queueTotalCount,
    queueAvailableClients,
    queueReady,
    queueStale,
    lastRefreshedAt,
    refreshQueue,
    loadMoreQueue,
    setVisibleQueueOffset,
  };
}
