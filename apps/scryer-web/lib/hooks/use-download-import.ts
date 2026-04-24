import { useCallback, useEffect, useRef, useState } from "react";
import { useClient } from "urql";

import { useGlobalStatus } from "@/lib/context/global-status-context";
import { downloadImportQuery } from "@/lib/graphql/queries";
import type {
  DownloadImportFilter,
  DownloadImportPage,
  DownloadQueueItem,
} from "@/lib/types";
import { downloadQueueItemIdentityKey } from "@/lib/utils/download-queue";

const IMPORT_PAGE_SIZE = 50;

type UseDownloadImportArgs = {
  enabled: boolean;
  filter: DownloadImportFilter;
};

export type UseDownloadImportResult = {
  importItems: DownloadQueueItem[];
  importLoading: boolean;
  importLoadingMore: boolean;
  importError: string | null;
  importHasMore: boolean;
  importTotalCount: number;
  lastRefreshedAt: Date | null;
  refreshImport: () => Promise<void>;
  loadMoreImport: () => Promise<void>;
};

function mergeImportItems(
  previousItems: DownloadQueueItem[],
  nextItems: DownloadQueueItem[],
): DownloadQueueItem[] {
  const seen = new Set(
    previousItems.map(downloadQueueItemIdentityKey),
  );
  const merged = [...previousItems];
  for (const item of nextItems) {
    const key = downloadQueueItemIdentityKey(item);
    if (seen.has(key)) {
      continue;
    }
    seen.add(key);
    merged.push(item);
  }
  return merged;
}

export function useDownloadImport({
  enabled,
  filter,
}: UseDownloadImportArgs): UseDownloadImportResult {
  const client = useClient();
  const setGlobalStatus = useGlobalStatus();
  const [importItems, setImportItems] = useState<DownloadQueueItem[]>([]);
  const [importLoading, setImportLoading] = useState(false);
  const [importLoadingMore, setImportLoadingMore] = useState(false);
  const [importError, setImportError] = useState<string | null>(null);
  const [importHasMore, setImportHasMore] = useState(false);
  const [importTotalCount, setImportTotalCount] = useState(0);
  const [lastRefreshedAt, setLastRefreshedAt] = useState<Date | null>(null);
  const importItemCountRef = useRef(0);
  importItemCountRef.current = importItems.length;

  const fetchImportPage = useCallback(
    async (limit: number, offset: number): Promise<DownloadImportPage> => {
      const { data, error } = await client
        .query(downloadImportQuery, { limit, offset, filter })
        .toPromise();
      if (error) {
        throw error;
      }
      return (
        data?.downloadImport ?? {
          items: [],
          hasMore: false,
          totalCount: 0,
        }
      );
    },
    [client, filter],
  );

  const refreshImport = useCallback(async () => {
    if (!enabled) {
      return;
    }

    setImportLoading(true);
    try {
      const limit = Math.max(importItemCountRef.current, IMPORT_PAGE_SIZE);
      const page = await fetchImportPage(limit, 0);
      setImportItems(page.items);
      setImportHasMore(page.hasMore);
      setImportTotalCount(page.totalCount);
      setImportError(null);
      setLastRefreshedAt(new Date());
    } catch (error) {
      const message =
        error instanceof Error ? error.message : "Failed to load import activity.";
      setImportError(message);
      setGlobalStatus(message);
    } finally {
      setImportLoading(false);
    }
  }, [enabled, fetchImportPage, setGlobalStatus]);

  const loadMoreImport = useCallback(async () => {
    if (!enabled || importLoadingMore || !importHasMore) {
      return;
    }

    setImportLoadingMore(true);
    try {
      const page = await fetchImportPage(IMPORT_PAGE_SIZE, importItems.length);
      setImportItems((current) => mergeImportItems(current, page.items));
      setImportHasMore(page.hasMore);
      setImportTotalCount(page.totalCount);
      setImportError(null);
      setLastRefreshedAt(new Date());
    } catch (error) {
      const message =
        error instanceof Error ? error.message : "Failed to load more import activity.";
      setImportError(message);
      setGlobalStatus(message);
    } finally {
      setImportLoadingMore(false);
    }
  }, [
    enabled,
    fetchImportPage,
    importHasMore,
    importItems.length,
    importLoadingMore,
    setGlobalStatus,
  ]);

  useEffect(() => {
    if (!enabled) {
      return;
    }
    void refreshImport();
  }, [enabled, refreshImport]);

  useEffect(() => {
    if (!enabled) {
      return;
    }

    const intervalId = setInterval(() => {
      void refreshImport();
    }, 10_000);

    return () => clearInterval(intervalId);
  }, [enabled, refreshImport]);

  return {
    importItems,
    importLoading,
    importLoadingMore,
    importError,
    importHasMore,
    importTotalCount,
    lastRefreshedAt,
    refreshImport,
    loadMoreImport,
  };
}
