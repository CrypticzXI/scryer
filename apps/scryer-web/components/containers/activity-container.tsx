
import { memo, useCallback, useEffect, useMemo, useState } from "react";
import { useClient, useMutation } from "urql";

import { AssignTrackedDownloadTitleDialog } from "@/components/dialogs/assign-tracked-download-title-dialog";
import { ManualImportDialog } from "@/components/dialogs/manual-import-dialog";
import { ActivityView } from "@/components/views/activity-view";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import {
  assignTrackedDownloadTitleMutation,
  ignoreTrackedDownloadMutation,
  queueManualImportMutation,
  pauseDownloadMutation,
  resumeDownloadMutation,
  deleteDownloadMutation,
} from "@/lib/graphql/mutations";
import { downloadClientsQuery } from "@/lib/graphql/queries";
import { useDownloadHistory } from "@/lib/hooks/use-download-history";
import { useDownloadImport } from "@/lib/hooks/use-download-import";
import { useDownloadQueue } from "@/lib/hooks/use-download-queue";
import { useImportHistorySubscription } from "@/lib/hooks/use-import-history-subscription";
import type {
  DownloadClientRecord,
  DownloadActivityStatus,
  DownloadClientFilterOption,
  DownloadHistoryStatus,
  DownloadImportStatus,
  DownloadQueueItem,
  SortConfig,
} from "@/lib/types";
import {
  collectDownloadClientFilterOptions,
  downloadQueueClientFilterKey,
  downloadQueueItemIdentityKey,
  matchesActivityStatuses,
  matchesImportStatuses,
} from "@/lib/utils/download-queue";

const HISTORY_STATES = new Set(["completed", "failed", "import_pending", "importpending"]);
type ActivityTab = "import" | "activity" | "history";
type SortConfigByTab = Record<ActivityTab, SortConfig>;

const IMPORT_STATUS_OPTIONS: DownloadImportStatus[] = [
  "importing",
  "pending",
  "blocked",
  "failed",
];
const ACTIVITY_STATUS_OPTIONS: DownloadActivityStatus[] = [
  "downloading",
  "queued",
  "paused",
  "post_processing",
];
const HISTORY_STATUS_OPTIONS: DownloadHistoryStatus[] = ["success", "failed"];
const DEFAULT_SORT_CONFIG_BY_TAB: SortConfigByTab = {
  import: { key: "status", direction: "asc" },
  activity: { key: "status", direction: "asc" },
  history: { key: "status", direction: "asc" },
};

function arraysEqual<T>(left: T[], right: T[]): boolean {
  return left.length === right.length && left.every((value, index) => value === right[index]);
}

function toggleSelectedValue<T extends string>(current: T[], nextValue: T): T[] {
  return current.includes(nextValue)
    ? current.filter((value) => value !== nextValue)
    : [...current, nextValue];
}

function mergeDownloadClientFilterOptions(
  configuredOptions: DownloadClientFilterOption[],
  visibleOptions: DownloadClientFilterOption[],
): DownloadClientFilterOption[] {
  const merged = new Map<string, DownloadClientFilterOption>();

  for (const option of visibleOptions) {
    merged.set(option.clientId, option);
  }

  for (const option of configuredOptions) {
    if (!merged.has(option.clientId)) {
      merged.set(option.clientId, option);
    }
  }

  return Array.from(merged.values()).sort((left, right) =>
    (left.clientName || left.clientType).localeCompare(right.clientName || right.clientType, undefined, {
      sensitivity: "base",
    }),
  );
}

export const ActivityContainer = memo(function ActivityContainer() {
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const client = useClient();
  const [, executeQueueManualImport] = useMutation(queueManualImportMutation);
  const [, executeAssignTrackedDownloadTitle] = useMutation(assignTrackedDownloadTitleMutation);
  const [, executeIgnoreTrackedDownload] = useMutation(ignoreTrackedDownloadMutation);
  const [, executePauseDownload] = useMutation(pauseDownloadMutation);
  const [, executeResumeDownload] = useMutation(resumeDownloadMutation);
  const [, executeDeleteDownload] = useMutation(deleteDownloadMutation);

  const [activeTab, setActiveTab] = useState<ActivityTab>("import");
  const [selectedImportStatuses, setSelectedImportStatuses] = useState<DownloadImportStatus[]>([
    ...IMPORT_STATUS_OPTIONS,
  ]);
  const [selectedActivityStatuses, setSelectedActivityStatuses] = useState<
    DownloadActivityStatus[]
  >([...ACTIVITY_STATUS_OPTIONS]);
  const [selectedHistoryStatuses, setSelectedHistoryStatuses] = useState<DownloadHistoryStatus[]>(
    [...HISTORY_STATUS_OPTIONS],
  );
  const [activityScryerSubmittedOnly, setActivityScryerSubmittedOnly] = useState(true);
  const [historyScryerSubmittedOnly, setHistoryScryerSubmittedOnly] = useState(true);
  const [selectedActivityClientIds, setSelectedActivityClientIds] = useState<string[] | null>(
    null,
  );
  const [selectedHistoryClientIds, setSelectedHistoryClientIds] = useState<string[] | null>(
    null,
  );
  const [sortConfigByTab, setSortConfigByTab] =
    useState<SortConfigByTab>(DEFAULT_SORT_CONFIG_BY_TAB);
  const [configuredClientOptions, setConfiguredClientOptions] = useState<
    DownloadClientFilterOption[]
  >([]);
  const [historyPage, setHistoryPage] = useState(1);
  const [manualImportItem, setManualImportItem] = useState<DownloadQueueItem | null>(null);
  const [assignTitleItem, setAssignTitleItem] = useState<DownloadQueueItem | null>(null);
  const [optimisticallyRemovedKeys, setOptimisticallyRemovedKeys] = useState<
    Record<string, true>
  >({});

  const {
    queueItems: activityQueueItems,
    queueLoading,
    queueError,
    refreshQueue,
  } = useDownloadQueue({
    enabled: true,
    includeAllActivity: !activityScryerSubmittedOnly,
    includeHistoryOnly: false,
    activityFilter: "all",
  });
  const {
    importItems,
    importLoading,
    importLoadingMore,
    importError,
    importHasMore,
    importTotalCount,
    refreshImport,
    loadMoreImport,
  } = useDownloadImport({
    enabled: true,
    filter: "all",
  });
  const {
    historyItems,
    historyLoading,
    historyError,
    historyTotalPages,
    historyAvailableClients,
    refreshHistory,
  } = useDownloadHistory({
    enabled: true,
    filters: selectedHistoryStatuses,
    clientIds: selectedHistoryClientIds,
    scryerSubmittedOnly: historyScryerSubmittedOnly,
    page: historyPage,
    sort: sortConfigByTab.history,
  });

  const filteredImportItems = useMemo(() => {
    return importItems.filter((item) => matchesImportStatuses(item, selectedImportStatuses));
  }, [importItems, selectedImportStatuses]);
  const hiddenImportItemCount = useMemo(() => {
    return importItems.filter((item) => optimisticallyRemovedKeys[downloadQueueItemIdentityKey(item)])
      .length;
  }, [importItems, optimisticallyRemovedKeys]);
  const importNotificationCount = Math.max(0, importTotalCount - hiddenImportItemCount);
  const statusFilteredActivityItems = useMemo(() => {
    return activityQueueItems.filter((item) => matchesActivityStatuses(item, selectedActivityStatuses));
  }, [activityQueueItems, selectedActivityStatuses]);
  const activityAvailableClients = useMemo<DownloadClientFilterOption[]>(() => {
    return mergeDownloadClientFilterOptions(
      configuredClientOptions,
      collectDownloadClientFilterOptions(statusFilteredActivityItems),
    );
  }, [configuredClientOptions, statusFilteredActivityItems]);
  const mergedHistoryAvailableClients = useMemo<DownloadClientFilterOption[]>(() => {
    return mergeDownloadClientFilterOptions(configuredClientOptions, historyAvailableClients);
  }, [configuredClientOptions, historyAvailableClients]);
  const filteredActivityItems = useMemo(() => {
    if (selectedActivityClientIds === null) {
      return statusFilteredActivityItems;
    }
    if (selectedActivityClientIds.length === 0) {
      return [];
    }
    const selectedClientIds = new Set(selectedActivityClientIds);
    return statusFilteredActivityItems.filter((item) =>
      selectedClientIds.has(downloadQueueClientFilterKey(item)),
    );
  }, [selectedActivityClientIds, statusFilteredActivityItems]);
  const visibleItems = useMemo(() => {
    const sourceItems =
      activeTab === "import"
        ? filteredImportItems
        : activeTab === "history"
          ? historyItems
          : filteredActivityItems;
    return sourceItems.filter(
      (item) => !optimisticallyRemovedKeys[downloadQueueItemIdentityKey(item)],
    );
  }, [
    activeTab,
    filteredActivityItems,
    filteredImportItems,
    historyItems,
    optimisticallyRemovedKeys,
  ]);
  const visibleLoading =
    activeTab === "import"
      ? importLoading
      : activeTab === "history"
        ? historyLoading
        : queueLoading;
  const visibleLoadingMore = activeTab === "import" ? importLoadingMore : false;
  const visibleHasMore = activeTab === "import" ? importHasMore : false;
  const visibleError =
    activeTab === "import"
      ? importError
      : activeTab === "history"
      ? historyError
      : queueError;
  const historyHasPreviousPage = historyPage > 1;
  const historyHasNextPage = historyPage < historyTotalPages;

  const refreshConfiguredClients = useCallback(async () => {
    try {
      const { data, error } = await client.query(downloadClientsQuery, {}).toPromise();
      if (error) {
        throw error;
      }
      const configuredClients: DownloadClientRecord[] = data?.downloadClientConfigs || [];
      setConfiguredClientOptions(
        configuredClients
          .filter((downloadClient) => downloadClient.isEnabled)
          .map((downloadClient) => ({
            clientId: downloadClient.id,
            clientName: downloadClient.name,
            clientType: downloadClient.clientType,
          })),
      );
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToLoad"));
    }
  }, [client, setGlobalStatus, t]);

  useEffect(() => {
    void refreshConfiguredClients();
  }, [refreshConfiguredClients]);

  useEffect(() => {
    const availableClientIds = activityAvailableClients.map((client) => client.clientId);
    setSelectedActivityClientIds((current) => {
      if (availableClientIds.length === 0) {
        if (current === null || current.length === 0) {
          return current;
        }
        return [];
      }
      if (current === null) {
        return availableClientIds;
      }
      const next = current.filter((clientId) => availableClientIds.includes(clientId));
      return arraysEqual(current, next) ? current : next;
    });
  }, [activityAvailableClients]);

  useEffect(() => {
    const availableClientIds = mergedHistoryAvailableClients.map((client) => client.clientId);
    setSelectedHistoryClientIds((current) => {
      if (availableClientIds.length === 0) {
        if (current === null || current.length === 0) {
          return current;
        }
        return [];
      }
      if (current === null) {
        return availableClientIds;
      }
      const next = current.filter((clientId) => availableClientIds.includes(clientId));
      return arraysEqual(current, next) ? current : next;
    });
  }, [mergedHistoryAvailableClients]);

  useEffect(() => {
    setHistoryPage(1);
  }, [
    selectedHistoryStatuses,
    selectedHistoryClientIds,
    historyScryerSubmittedOnly,
    sortConfigByTab.history.direction,
    sortConfigByTab.history.key,
  ]);

  useEffect(() => {
    if (historyTotalPages > 0 && historyPage > historyTotalPages) {
      setHistoryPage(historyTotalPages);
    }
  }, [historyPage, historyTotalPages]);

  const refreshActivityViews = useCallback(async () => {
    await Promise.all([refreshQueue(), refreshImport(), refreshHistory(), refreshConfiguredClients()]);
    window.dispatchEvent(new CustomEvent("scryer:activityQueueRefresh"));
  }, [refreshConfiguredClients, refreshHistory, refreshImport, refreshQueue]);

  useImportHistorySubscription(() => {
    void refreshActivityViews();
  });

  useEffect(() => {
    if (Object.keys(optimisticallyRemovedKeys).length === 0) {
      return;
    }

    const authoritativeItems = [...activityQueueItems, ...importItems, ...historyItems];
    const authoritativeByKey = new Map(
      authoritativeItems.map((item) => [downloadQueueItemIdentityKey(item), item]),
    );

    setOptimisticallyRemovedKeys((current) => {
      const next = Object.fromEntries(
        Object.entries(current).filter(([key]) => {
          const item = authoritativeByKey.get(key);
          if (!item) {
            return false;
          }

          return item.deleteStatus !== "failed";
        }),
      );

      return Object.keys(next).length === Object.keys(current).length ? current : next;
    });
  }, [activityQueueItems, historyItems, importItems, optimisticallyRemovedKeys]);

  const decrementImportBadges = useCallback(() => {
    window.dispatchEvent(
      new CustomEvent("scryer:pendingImportsRefresh", {
        detail: { delta: -1 },
      }),
    );
  }, []);

  const requestManualImport = useCallback(
    async (item: DownloadQueueItem) => {
      if (!item.titleId) {
        setGlobalStatus(t("queue.assignTitleBeforeImport"));
        return;
      }

      if (item.facet === "series" || item.facet === "anime") {
        setManualImportItem(item);
        return;
      }

      const result = await executeQueueManualImport({
        input: {
          downloadClientItemId: item.downloadClientItemId,
          titleId: item.titleId,
          clientType: item.clientType,
        },
      });
      if (result.error) {
        const message = result.error.message ?? t("queue.manualImportFailed");
        setGlobalStatus(message);
        throw result.error;
      }
      setGlobalStatus(t("queue.manualImportQueued"));
      await refreshActivityViews();
    },
    [executeQueueManualImport, refreshActivityViews, setGlobalStatus, t],
  );

  const requestAssignTitle = useCallback(
    async (item: DownloadQueueItem, titleId: string) => {
      const result = await executeAssignTrackedDownloadTitle({
        input: {
          clientType: item.clientType,
          downloadClientItemId: item.downloadClientItemId,
          titleId,
          scope: { title: true },
        },
      });
      if (result.error) {
        const message = result.error.message ?? t("queue.assignTitleFailed");
        setGlobalStatus(message);
        throw result.error;
      }
      setGlobalStatus(t("queue.assignTitleQueued"));
      await refreshActivityViews();
    },
    [executeAssignTrackedDownloadTitle, refreshActivityViews, setGlobalStatus, t],
  );

  const requestIgnore = useCallback(
    async (item: DownloadQueueItem) => {
      const result = await executeIgnoreTrackedDownload({
        input: {
          clientType: item.clientType,
          downloadClientItemId: item.downloadClientItemId,
        },
      });
      if (result.error) {
        const message = result.error.message ?? t("queue.ignoreFailed");
        setGlobalStatus(message);
        throw result.error;
      }
      setGlobalStatus(t("queue.ignoreSuccess"));
      await refreshActivityViews();
    },
    [executeIgnoreTrackedDownload, refreshActivityViews, setGlobalStatus, t],
  );

  const requestPause = useCallback(
    async (item: DownloadQueueItem) => {
      const result = await executePauseDownload({
        input: { downloadClientItemId: item.downloadClientItemId },
      });
      if (result.error) {
        const message = result.error.message ?? t("queue.pauseFailed");
        setGlobalStatus(message);
        throw result.error;
      }
      setGlobalStatus(t("queue.pauseSuccess"));
      await refreshActivityViews();
    },
    [refreshActivityViews, executePauseDownload, setGlobalStatus, t],
  );

  const requestResume = useCallback(
    async (item: DownloadQueueItem) => {
      const result = await executeResumeDownload({
        input: { downloadClientItemId: item.downloadClientItemId },
      });
      if (result.error) {
        const message = result.error.message ?? t("queue.resumeFailed");
        setGlobalStatus(message);
        throw result.error;
      }
      setGlobalStatus(t("queue.resumeSuccess"));
      await refreshActivityViews();
    },
    [refreshActivityViews, executeResumeDownload, setGlobalStatus, t],
  );

  const requestDelete = useCallback(
    async (item: DownloadQueueItem) => {
      const stateNormalized = item.state.trim().toLowerCase();
      const isHistory = HISTORY_STATES.has(stateNormalized);
      const result = await executeDeleteDownload({
        input: {
          clientType: item.clientType,
          downloadClientItemId: item.downloadClientItemId,
          isHistory,
        },
      });
      if (result.error) {
        const message = result.error.message ?? t("queue.deleteFailed");
        setGlobalStatus(message);
        throw result.error;
      }
      setOptimisticallyRemovedKeys((current) => ({
        ...current,
        [downloadQueueItemIdentityKey(item)]: true,
      }));
      if (matchesImportStatuses(item, IMPORT_STATUS_OPTIONS)) {
        decrementImportBadges();
      }
      setGlobalStatus(t("queue.deleteQueued"));
      void refreshQueue();
      void refreshImport();
      void refreshHistory();
    },
    [
      decrementImportBadges,
      executeDeleteDownload,
      refreshHistory,
      refreshImport,
      refreshQueue,
      setGlobalStatus,
      t,
    ],
  );

  return (
    <>
      <ActivityView
        state={{
          queueItems: visibleItems,
          queueLoading: visibleLoading,
          queueLoadingMore: visibleLoadingMore,
          queueError: visibleError,
          requestManualImport,
          requestAssignTitle: async (item) => {
            setAssignTitleItem(item);
          },
          requestIgnore,
          requestPause,
          requestResume,
          requestDelete,
          activeTab,
          setActiveTab,
          importNotificationCount,
          sortConfigByTab,
          toggleSort: (tab, nextKey) => {
            setSortConfigByTab((current) => {
              const currentConfig = current[tab];
              return {
                ...current,
                [tab]:
                  currentConfig.key === nextKey
                    ? {
                        key: nextKey,
                        direction: currentConfig.direction === "asc" ? "desc" : "asc",
                      }
                    : DEFAULT_SORT_CONFIG_BY_TAB[tab].key === nextKey
                      ? DEFAULT_SORT_CONFIG_BY_TAB[tab]
                      : { key: nextKey, direction: "asc" },
              };
            });
          },
          activityScryerSubmittedOnly,
          toggleActivityScryerSubmittedOnly: () => {
            setActivityScryerSubmittedOnly((current) => !current);
          },
          historyScryerSubmittedOnly,
          toggleHistoryScryerSubmittedOnly: () => {
            setHistoryScryerSubmittedOnly((current) => !current);
          },
          selectedImportStatuses,
          toggleImportStatus: (status) => {
            setSelectedImportStatuses((current) => toggleSelectedValue(current, status));
          },
          selectedActivityStatuses,
          toggleActivityStatus: (status) => {
            setSelectedActivityStatuses((current) => toggleSelectedValue(current, status));
          },
          selectedHistoryStatuses,
          toggleHistoryStatus: (status) => {
            setSelectedHistoryStatuses((current) => toggleSelectedValue(current, status));
          },
          activityAvailableClients,
          selectedActivityClientIds:
            selectedActivityClientIds ?? activityAvailableClients.map((client) => client.clientId),
          toggleActivityClientId: (clientId) => {
            setSelectedActivityClientIds((current) =>
              toggleSelectedValue(current ?? activityAvailableClients.map((client) => client.clientId), clientId),
            );
          },
          historyAvailableClients: mergedHistoryAvailableClients,
          selectedHistoryClientIds:
            selectedHistoryClientIds ??
            mergedHistoryAvailableClients.map((client) => client.clientId),
          toggleHistoryClientId: (clientId) => {
            setSelectedHistoryClientIds((current) =>
              toggleSelectedValue(
                current ?? mergedHistoryAvailableClients.map((client) => client.clientId),
                clientId,
              ),
            );
          },
          historyPage,
          historyTotalPages,
          goToPreviousHistoryPage: async () => {
            setHistoryPage((current) => Math.max(1, current - 1));
          },
          goToNextHistoryPage: async () => {
            setHistoryPage((current) => Math.min(historyTotalPages, current + 1));
          },
          historyHasPreviousPage,
          historyHasNextPage,
          visibleHasMore,
          requestMoreItems:
            activeTab === "import" ? loadMoreImport : async () => {},
        }}
      />
      {manualImportItem?.titleId ? (
        <ManualImportDialog
          open={manualImportItem !== null}
          onOpenChange={(open) => {
            if (!open) {
              setManualImportItem(null);
            }
          }}
          titleId={manualImportItem.titleId}
          titleName={manualImportItem.titleName}
          clientType={manualImportItem.clientType}
          downloadClientItemId={manualImportItem.downloadClientItemId}
          onImportComplete={() => {
            setOptimisticallyRemovedKeys((current) => ({
              ...current,
              [downloadQueueItemIdentityKey(manualImportItem)]: true,
            }));
            decrementImportBadges();
            void refreshActivityViews();
          }}
        />
      ) : null}
      <AssignTrackedDownloadTitleDialog
        open={assignTitleItem !== null}
        onOpenChange={(open) => {
          if (!open) {
            setAssignTitleItem(null);
          }
        }}
        queueItem={assignTitleItem}
        onAssign={requestAssignTitle}
      />
    </>
  );
});
