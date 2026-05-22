import { useCallback, useEffect, useRef, useState } from "react";
import { useClient } from "urql";

import { useGlobalStatus } from "@/lib/context/global-status-context";
import { downloadHistoryQuery } from "@/lib/graphql/queries";
import type {
  DownloadClientFilterOption,
  DownloadHistoryPage,
  DownloadHistoryStatus,
  DownloadQueueItem,
  SortConfig,
} from "@/lib/types";

const HISTORY_PAGE_SIZE = 50;

type UseDownloadHistoryArgs = {
  enabled: boolean;
  filters: DownloadHistoryStatus[];
  clientIds: string[] | null;
  scryerSubmittedOnly: boolean;
  page: number;
  sort: SortConfig;
};

export type UseDownloadHistoryResult = {
  historyItems: DownloadQueueItem[];
  historyLoading: boolean;
  historyError: string | null;
  historyTotalCount: number;
  historyTotalPages: number;
  historyAvailableClients: DownloadClientFilterOption[];
  lastRefreshedAt: Date | null;
  refreshHistory: () => Promise<void>;
};

export function useDownloadHistory({
  enabled,
  filters,
  clientIds,
  scryerSubmittedOnly,
  page,
  sort,
}: UseDownloadHistoryArgs): UseDownloadHistoryResult {
  const client = useClient();
  const setGlobalStatus = useGlobalStatus();
  const [historyItems, setHistoryItems] = useState<DownloadQueueItem[]>([]);
  const [historyLoading, setHistoryLoading] = useState(false);
  const [historyError, setHistoryError] = useState<string | null>(null);
  const [historyTotalCount, setHistoryTotalCount] = useState(0);
  const [historyTotalPages, setHistoryTotalPages] = useState(0);
  const [historyAvailableClients, setHistoryAvailableClients] = useState<DownloadClientFilterOption[]>(
    [],
  );
  const [lastRefreshedAt, setLastRefreshedAt] = useState<Date | null>(null);
  const lastReportedErrorRef = useRef<string | null>(null);
  const filtersKey = filters.join("|");
  const clientIdsKey = clientIds?.join("|") ?? "";
  const sortKey = `${sort.key}:${sort.direction}`;
  const sourceKey = scryerSubmittedOnly ? "scryer" : "all";
  const pageRef = useRef(page);
  const filtersRef = useRef(filters);
  const clientIdsRef = useRef(clientIds);
  const sortRef = useRef(sort);

  useEffect(() => {
    pageRef.current = page;
  }, [page]);

  useEffect(() => {
    filtersRef.current = filters;
  }, [filters]);

  useEffect(() => {
    clientIdsRef.current = clientIds;
  }, [clientIds]);

  useEffect(() => {
    sortRef.current = sort;
  }, [sort]);

  const fetchHistoryPage = useCallback(
    async (pageNumber: number): Promise<DownloadHistoryPage> => {
      const offset = Math.max(pageNumber - 1, 0) * HISTORY_PAGE_SIZE;
      const { data, error } = await client
        .query(downloadHistoryQuery, {
          limit: HISTORY_PAGE_SIZE,
          offset,
          filters: filtersRef.current,
          clientIds: clientIdsRef.current,
          scryerSubmittedOnly,
          sortKey: sortRef.current.key,
          sortDirection: sortRef.current.direction,
        })
        .toPromise();
      if (error) {
        throw error;
      }
      return (
        data?.downloadHistory ?? {
          items: [],
          hasMore: false,
          totalCount: 0,
          availableClients: [],
        }
      );
    },
    [client, scryerSubmittedOnly],
  );

  const refreshHistory = useCallback(async () => {
    if (!enabled) {
      return;
    }

    setHistoryLoading(true);
    try {
      const historyPage = await fetchHistoryPage(pageRef.current);
      setHistoryItems(historyPage.items);
      setHistoryTotalCount(historyPage.totalCount);
      setHistoryTotalPages(Math.max(1, Math.ceil(historyPage.totalCount / HISTORY_PAGE_SIZE)));
      setHistoryAvailableClients(historyPage.availableClients);
      setHistoryError(null);
      lastReportedErrorRef.current = null;
      setLastRefreshedAt(new Date());
    } catch (error) {
      const message =
        error instanceof Error ? error.message : "Failed to load activity history.";
      setHistoryError(message);
      if (lastReportedErrorRef.current !== message) {
        lastReportedErrorRef.current = message;
        setGlobalStatus(message);
      }
    } finally {
      setHistoryLoading(false);
    }
  }, [enabled, fetchHistoryPage, setGlobalStatus]);

  useEffect(() => {
    lastReportedErrorRef.current = null;
  }, [clientIdsKey, enabled, filtersKey, page, sortKey, sourceKey]);

  useEffect(() => {
    if (!enabled) {
      return;
    }
    void refreshHistory();
  }, [clientIdsKey, enabled, filtersKey, page, refreshHistory, sortKey, sourceKey]);

  useEffect(() => {
    if (!enabled) {
      return;
    }

    const intervalId = setInterval(() => {
      void refreshHistory();
    }, 10_000);

    return () => clearInterval(intervalId);
  }, [clientIdsKey, enabled, filtersKey, page, refreshHistory, sortKey, sourceKey]);

  return {
    historyItems,
    historyLoading,
    historyError,
    historyTotalCount,
    historyTotalPages,
    historyAvailableClients,
    lastRefreshedAt,
    refreshHistory,
  };
}
