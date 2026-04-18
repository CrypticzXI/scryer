
import { memo, useCallback, useMemo, useState } from "react";
import { useMutation } from "urql";

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
import { useDownloadHistory } from "@/lib/hooks/use-download-history";
import { useDownloadQueue } from "@/lib/hooks/use-download-queue";
import { useImportHistorySubscription } from "@/lib/hooks/use-import-history-subscription";
import type { DownloadQueueItem } from "@/lib/types";
import {
  isActiveQueueState,
  isHistoryQueueState,
  isManualImportRequiredQueueItem,
} from "@/lib/utils/download-queue";

const HISTORY_STATES = new Set(["completed", "failed", "import_pending", "importpending"]);
type QueueMode = "scryer" | "all" | "history";

export const ActivityContainer = memo(function ActivityContainer() {
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const [, executeQueueManualImport] = useMutation(queueManualImportMutation);
  const [, executeAssignTrackedDownloadTitle] = useMutation(assignTrackedDownloadTitleMutation);
  const [, executeIgnoreTrackedDownload] = useMutation(ignoreTrackedDownloadMutation);
  const [, executePauseDownload] = useMutation(pauseDownloadMutation);
  const [, executeResumeDownload] = useMutation(resumeDownloadMutation);
  const [, executeDeleteDownload] = useMutation(deleteDownloadMutation);

  const [queueMode, setQueueMode] = useState<QueueMode>("scryer");
  const [manualImportItem, setManualImportItem] = useState<DownloadQueueItem | null>(null);
  const [assignTitleItem, setAssignTitleItem] = useState<DownloadQueueItem | null>(null);

  const {
    queueItems: activityQueueItems,
    queueLoading,
    queueError,
    lastRefreshedAt,
    refreshQueue,
  } = useDownloadQueue({
    enabled: queueMode !== "history",
    includeAllActivity: true,
    includeHistoryOnly: false,
  });
  const {
    historyItems,
    historyLoading,
    historyLoadingMore,
    historyError,
    historyHasMore,
    lastRefreshedAt: historyLastRefreshedAt,
    refreshHistory,
    loadMoreHistory,
  } = useDownloadHistory({
    enabled: queueMode === "history",
  });

  const manualImportRequiredItems = useMemo(
    () =>
      queueMode === "history"
        ? []
        : activityQueueItems.filter((item) => isManualImportRequiredQueueItem(item)),
    [activityQueueItems, queueMode],
  );

  const visibleItems = useMemo(() => {
    if (queueMode === "history") {
      return historyItems;
    }

    const blockedIds = new Set(manualImportRequiredItems.map((item) => item.id));
    const filteredQueueItems = activityQueueItems.filter((item) => {
      if (blockedIds.has(item.id)) {
        return false;
      }

      if (queueMode === "scryer") {
        return item.isScryerOrigin && isActiveQueueState(item.state);
      }

      return !isHistoryQueueState(item.state);
    });

    return filteredQueueItems;
  }, [activityQueueItems, historyItems, manualImportRequiredItems, queueMode]);
  const visibleLoading = queueMode === "history" ? historyLoading : queueLoading;
  const visibleError = queueMode === "history" ? historyError : queueError;
  const visibleLastRefreshedAt =
    queueMode === "history" ? historyLastRefreshedAt : lastRefreshedAt;

  const refreshActivityViews = useCallback(async () => {
    await Promise.all([refreshQueue(), refreshHistory()]);
    window.dispatchEvent(new CustomEvent("scryer:activityQueueRefresh"));
  }, [refreshHistory, refreshQueue]);

  useImportHistorySubscription(() => {
    void refreshActivityViews();
  });

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
          downloadClientItemId: item.downloadClientItemId,
          isHistory,
        },
      });
      if (result.error) {
        const message = result.error.message ?? t("queue.deleteFailed");
        setGlobalStatus(message);
        throw result.error;
      }
      setGlobalStatus(t("queue.deleteSuccess"));
      await refreshActivityViews();
    },
    [refreshActivityViews, executeDeleteDownload, setGlobalStatus, t],
  );

  return (
    <>
      <ActivityView
        state={{
          manualImportRequiredItems,
          queueItems: visibleItems,
          queueLoading: visibleLoading,
          queueError: visibleError,
          lastRefreshedAt: visibleLastRefreshedAt,
          requestManualImport,
          requestAssignTitle: async (item) => {
            setAssignTitleItem(item);
          },
          requestIgnore,
          requestPause,
          requestResume,
          requestDelete,
          queueMode,
          setQueueMode,
          historyHasMore,
          historyLoadingMore,
          requestMoreHistory: loadMoreHistory,
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
          onImportComplete={() => void refreshActivityViews()}
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
