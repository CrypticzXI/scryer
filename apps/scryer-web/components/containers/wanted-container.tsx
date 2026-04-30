import { memo, useCallback, useEffect, useRef, useState } from "react";
import type { OverviewTitleTarget, ViewId, WantedSection } from "@/components/root/types";
import { useClient, useMutation } from "urql";
import { WantedView } from "@/components/views/wanted-view";
import type { CutoffUnmetItem } from "@/components/views/cutoff-unmet-view";
import {
  cutoffUnmetTitlesQuery,
  pendingReleasesQuery,
  releaseDecisionsQuery,
  searchForEpisodeQuery,
  searchForTitleQuery,
  wantedItemsQuery,
} from "@/lib/graphql/queries";
import {
  triggerWantedSearchMutation,
  triggerTitleMismatchRecoverySearchMutation,
  pauseWantedItemMutation,
  resumeWantedItemMutation,
  resetWantedItemMutation,
  queueBestReleaseMutation,
  queueExistingMutation,
  forceGrabPendingReleaseMutation,
  dismissPendingReleaseMutation,
} from "@/lib/graphql/mutations";
import type {
  PendingReleaseItem,
  Release,
  ReleaseDecisionItem,
  WantedItem,
  WantedMediaType,
  WantedStatus,
} from "@/lib/types";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";
import { useDownloadConflictConfirmation } from "@/components/common/download-conflict-confirmation";
import {
  assertNoReplaceConflict,
  retryWithReplaceOnConflict,
} from "@/lib/utils/download-conflicts";
import { releaseQueueScopeInput } from "@/lib/utils/release-queue-scope";

type WantedContainerProps = {
  wantedSection: WantedSection;
  onOpenOverview?: (
    targetView: ViewId,
    overviewTarget: OverviewTitleTarget,
    episodeId?: string,
  ) => void;
};

function cutoffItemKey(item: CutoffUnmetItem) {
  return item.episodeId?.trim() || item.titleId;
}

function cutoffItemEpisodeCode(item: CutoffUnmetItem): string | null {
  const seasonDigits = item.seasonNumber?.match(/\d+/)?.[0] ?? null;
  const episodeDigits = item.episodeNumber?.match(/\d+/)?.[0] ?? null;
  if (!seasonDigits || !episodeDigits) {
    return null;
  }
  return `S${seasonDigits.padStart(2, "0")}E${episodeDigits.padStart(2, "0")}`;
}

function cutoffItemLabel(item: CutoffUnmetItem) {
  const episodeCode = cutoffItemEpisodeCode(item);
  return episodeCode ? `${item.titleName} ${episodeCode}` : item.titleName;
}

function cutoffConflictMessage(item: CutoffUnmetItem) {
  return item.episodeId
    ? "A download is already in progress for this episode."
    : "A download is already in progress for this title.";
}

function cutoffQueueScope(item: CutoffUnmetItem) {
  return item.episodeId?.trim() ? { episode: item.episodeId.trim() } : { title: true };
}

export const WantedContainer = memo(function WantedContainer({
  wantedSection,
  onOpenOverview,
}: WantedContainerProps) {
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const client = useClient();
  const { confirmReplaceConflict, replaceConflictDialog } =
    useDownloadConflictConfirmation();

  // --- Wanted items state ---
  const [items, setItems] = useState<WantedItem[]>([]);
  const [total, setTotal] = useState(0);
  const [loading, setLoading] = useState(false);
  const [statusFilter, setStatusFilter] = useState<WantedStatus | undefined>(undefined);
  const [mediaTypeFilter, setMediaTypeFilter] = useState<WantedMediaType | undefined>(undefined);
  const [latestDecisionCodeFilter, setLatestDecisionCodeFilter] = useState<string | undefined>(undefined);
  const [titleFilterInput, setTitleFilterInput] = useState("");
  const [titleSearch, setTitleSearch] = useState<string | undefined>(undefined);
  const [offset, setOffset] = useState(0);
  const limit = 50;

  const [expandedItemId, setExpandedItemId] = useState<string | null>(null);
  const [decisions, setDecisions] = useState<ReleaseDecisionItem[]>([]);
  const [decisionsLoading, setDecisionsLoading] = useState(false);

  const [, executeTriggerSearch] = useMutation(triggerWantedSearchMutation);
  const [, executePause] = useMutation(pauseWantedItemMutation);
  const [, executeResume] = useMutation(resumeWantedItemMutation);
  const [, executeReset] = useMutation(resetWantedItemMutation);
  const [, executeMismatchRecovery] = useMutation(triggerTitleMismatchRecoverySearchMutation);

  // --- Cutoff state ---
  const [cutoffItems, setCutoffItems] = useState<CutoffUnmetItem[]>([]);
  const [cutoffLoading, setCutoffLoading] = useState(false);
  const [cutoffFacetFilter, setCutoffFacetFilter] = useState<string | undefined>(undefined);
  const [cutoffAutoSearchingId, setCutoffAutoSearchingId] = useState<string | null>(null);
  const [cutoffInteractiveSearchingId, setCutoffInteractiveSearchingId] = useState<string | null>(null);
  const [cutoffActiveInteractiveItemId, setCutoffActiveInteractiveItemId] = useState<string | null>(null);
  const [cutoffSearchResultsByItemId, setCutoffSearchResultsByItemId] = useState<
    Record<string, Release[]>
  >({});
  const [bulkSearching, setBulkSearching] = useState(false);
  const [bulkProgress, setBulkProgress] = useState<{ current: number; total: number } | null>(null);
  const bulkCancelRef = useRef(false);

  // --- Pending releases state ---
  const [pendingItems, setPendingItems] = useState<PendingReleaseItem[]>([]);
  const [pendingLoading, setPendingLoading] = useState(false);
  const [, executeForceGrab] = useMutation(forceGrabPendingReleaseMutation);
  const [, executeDismiss] = useMutation(dismissPendingReleaseMutation);

  const refreshPending = useCallback(async () => {
    setPendingLoading(true);
    try {
      const { data, error } = await client.query(pendingReleasesQuery, {}).toPromise();
      if (error) throw error;
      setPendingItems(data?.pendingReleases ?? []);
    } catch (error) {
      const message = error instanceof Error ? error.message : t("status.failedToLoad");
      setGlobalStatus(message);
    } finally {
      setPendingLoading(false);
    }
  }, [client, t, setGlobalStatus]);

  useEffect(() => {
    if (wantedSection === "pending") {
      void refreshPending();
    }
  }, [refreshPending, wantedSection]);

  const forceGrabPending = useCallback(
    async (id: string) => {
      const { error } = await executeForceGrab({ input: { id } });
      if (error) {
        setGlobalStatus(error.message);
      } else {
        setGlobalStatus(t("pending.grabbed"));
        void refreshPending();
      }
    },
    [executeForceGrab, refreshPending, setGlobalStatus, t],
  );

  const dismissPending = useCallback(
    async (id: string) => {
      const { error } = await executeDismiss({ input: { id } });
      if (error) {
        setGlobalStatus(error.message);
      } else {
        setGlobalStatus(t("pending.dismissed"));
        void refreshPending();
      }
    },
    [executeDismiss, refreshPending, setGlobalStatus, t],
  );

  // --- Wanted data fetching ---

  const refreshItems = useCallback(async () => {
    setLoading(true);
    try {
      const { data, error } = await client
        .query(wantedItemsQuery, {
          status: statusFilter,
          mediaType: mediaTypeFilter,
          titleSearch,
          latestDecisionCode: latestDecisionCodeFilter,
          limit,
          offset,
        })
        .toPromise();
      if (error) throw error;
      setItems(data?.wantedItems?.items ?? []);
      setTotal(data?.wantedItems?.total ?? 0);
    } catch (error) {
      const message = error instanceof Error ? error.message : t("status.failedToLoad");
      setGlobalStatus(message);
    } finally {
      setLoading(false);
    }
  }, [
    client,
    statusFilter,
    mediaTypeFilter,
    titleSearch,
    latestDecisionCodeFilter,
    offset,
    t,
    setGlobalStatus,
  ]);

  useEffect(() => {
    if (wantedSection === "wanted") {
      void refreshItems();
    }
  }, [refreshItems, wantedSection]);

  useEffect(() => {
    const handle = window.setTimeout(() => {
      const normalized = titleFilterInput.trim();
      setOffset(0);
      setTitleSearch(normalized.length > 0 ? normalized : undefined);
    }, 250);

    return () => window.clearTimeout(handle);
  }, [titleFilterInput]);

  // --- Cutoff data fetching ---

  const refreshCutoff = useCallback(async () => {
    setCutoffLoading(true);
    try {
      const { data, error } = await client
        .query(cutoffUnmetTitlesQuery, {
          facet: cutoffFacetFilter ?? null,
        })
        .toPromise();
      if (error) throw error;
      setCutoffItems(data?.cutoffUnmetTitles ?? []);
    } catch (error) {
      const message = error instanceof Error ? error.message : t("status.failedToLoad");
      setGlobalStatus(message);
    } finally {
      setCutoffLoading(false);
    }
  }, [client, cutoffFacetFilter, t, setGlobalStatus]);

  useEffect(() => {
    if (wantedSection === "cutoff") {
      void refreshCutoff();
    }
  }, [refreshCutoff, wantedSection]);

  // --- Wanted actions ---

  const loadDecisions = useCallback(
    async (wantedItemId: string) => {
      if (expandedItemId === wantedItemId) {
        setExpandedItemId(null);
        return;
      }
      setExpandedItemId(wantedItemId);
      setDecisionsLoading(true);
      try {
        const { data, error } = await client
          .query(releaseDecisionsQuery, { wantedItemId, limit: 20 })
          .toPromise();
        if (error) throw error;
        setDecisions(data?.wantedItem?.releaseDecisions ?? []);
      } catch {
        setDecisions([]);
      } finally {
        setDecisionsLoading(false);
      }
    },
    [client, expandedItemId],
  );

  const triggerSearch = useCallback(
    async (id: string) => {
      try {
        const payload = await retryWithReplaceOnConflict(
          { wantedItemId: id },
          async (input) => {
            const { data, error } = await executeTriggerSearch({ input });
            if (error) throw error;
            return data?.triggerWantedSearch;
          },
          "A download is already in progress for this wanted item.",
          confirmReplaceConflict,
        );
        assertNoReplaceConflict(payload, "A download is already in progress for this wanted item.");
        setGlobalStatus(t("wanted.searchTriggered"));
        void refreshItems();
      } catch (error) {
        setGlobalStatus(error instanceof Error ? error.message : t("status.queueFailed"));
      }
    },
    [executeTriggerSearch, confirmReplaceConflict, refreshItems, setGlobalStatus, t],
  );

  const pauseItem = useCallback(
    async (id: string) => {
      const { error } = await executePause({ input: { wantedItemId: id } });
      if (error) {
        setGlobalStatus(error.message);
      } else {
        void refreshItems();
      }
    },
    [executePause, refreshItems, setGlobalStatus],
  );

  const resumeItem = useCallback(
    async (id: string) => {
      const { error } = await executeResume({ input: { wantedItemId: id } });
      if (error) {
        setGlobalStatus(error.message);
      } else {
        void refreshItems();
      }
    },
    [executeResume, refreshItems, setGlobalStatus],
  );

  const resetItem = useCallback(
    async (id: string) => {
      const { error } = await executeReset({ input: { wantedItemId: id } });
      if (error) {
        setGlobalStatus(error.message);
      } else {
        void refreshItems();
      }
    },
    [executeReset, refreshItems, setGlobalStatus],
  );

  const triggerMismatchRecovery = useCallback(
    async (titleId: string) => {
      const { data, error } = await executeMismatchRecovery({ input: { titleId } });
      if (error) {
        setGlobalStatus(error.message);
      } else {
        setGlobalStatus(
          t("status.mismatchRecoveryQueued", {
            count: data?.triggerTitleMismatchRecoverySearch ?? 0,
          }),
        );
        void refreshItems();
      }
    },
    [executeMismatchRecovery, refreshItems, setGlobalStatus, t],
  );

  // --- Cutoff search actions ---

  const searchAndQueueCutoffItem = useCallback(
    async (
      cutoffItem: CutoffUnmetItem,
      options: { allowReplaceConfirmation?: boolean } = {},
    ) => {
      const input = {
        titleId: cutoffItem.titleId,
        scope: cutoffQueueScope(cutoffItem),
      };
      const submit = async (nextInput: typeof input & { replaceInProgress?: boolean }) => {
        const { data, error } = await client
          .mutation(queueBestReleaseMutation, { input: nextInput })
          .toPromise();
        if (error) throw error;
        return data?.queueBestRelease;
      };
      const payload = options.allowReplaceConfirmation
        ? await retryWithReplaceOnConflict(
            input,
            submit,
            cutoffConflictMessage(cutoffItem),
            confirmReplaceConflict,
          )
        : await submit(input);
      assertNoReplaceConflict(payload, cutoffConflictMessage(cutoffItem));
      setGlobalStatus(t("cutoff.searchTriggered", { name: cutoffItemLabel(cutoffItem) }));
    },
    [client, confirmReplaceConflict, t, setGlobalStatus],
  );

  const cutoffTriggerAutoSearch = useCallback(
    async (item: CutoffUnmetItem) => {
      const itemKey = cutoffItemKey(item);
      setCutoffAutoSearchingId(itemKey);
      try {
        await searchAndQueueCutoffItem(item, { allowReplaceConfirmation: true });
      } catch (error) {
        setGlobalStatus(error instanceof Error ? error.message : t("status.queueFailed"));
      } finally {
        setCutoffAutoSearchingId(null);
      }
    },
    [searchAndQueueCutoffItem, setGlobalStatus, t],
  );

  const cutoffTriggerInteractiveSearch = useCallback(
    async (item: CutoffUnmetItem) => {
      const itemKey = cutoffItemKey(item);
      setCutoffInteractiveSearchingId(itemKey);
      try {
        if (item.episodeId) {
          const season = item.seasonNumber?.trim();
          const episode = item.episodeNumber?.trim();
          if (!season || !episode) {
            throw new Error("Episode search is unavailable because the episode numbers are missing.");
          }
          const { data, error } = await client
            .query(searchForEpisodeQuery, {
              titleId: item.titleId,
              season,
              episode,
            })
            .toPromise();
          if (error) throw error;
          const results = data?.searchReleases ?? [];
          setCutoffSearchResultsByItemId((current) => ({ ...current, [itemKey]: results }));
          setCutoffActiveInteractiveItemId(itemKey);
          setGlobalStatus(t("status.foundNzb", { count: results.length }));
        } else {
          const { data, error } = await client
            .query(searchForTitleQuery, { titleId: item.titleId })
            .toPromise();
          if (error) throw error;
          const results = data?.searchReleases ?? [];
          setCutoffSearchResultsByItemId((current) => ({ ...current, [itemKey]: results }));
          setCutoffActiveInteractiveItemId(itemKey);
          setGlobalStatus(t("status.foundNzb", { count: results.length }));
        }
      } catch (error) {
        setGlobalStatus(error instanceof Error ? error.message : t("status.apiError"));
      } finally {
        setCutoffInteractiveSearchingId(null);
      }
    },
    [client, setGlobalStatus, t],
  );

  const cutoffQueueRelease = useCallback(
    async (item: CutoffUnmetItem, release: Release) => {
      if (!release.candidateToken) {
        setGlobalStatus(t("status.releaseMissingCandidateToken"));
        return;
      }

      const conflictMessage = cutoffConflictMessage(item);
      const input = {
        titleId: item.titleId,
        scope: releaseQueueScopeInput(release, cutoffQueueScope(item)),
        candidateToken: release.candidateToken,
      };

      try {
        const payload = await retryWithReplaceOnConflict(
          input,
          async (nextInput) => {
            const { data, error } = await client
              .mutation(queueExistingMutation, { input: nextInput })
              .toPromise();
            if (error) throw error;
            return data?.queueExistingTitleDownload;
          },
          conflictMessage,
          confirmReplaceConflict,
        );
        assertNoReplaceConflict(payload, conflictMessage);
        setGlobalStatus(t("status.queueSuccess", { name: release.title }));
      } catch (error) {
        setGlobalStatus(error instanceof Error ? error.message : t("status.queueFailed"));
      }
    },
    [client, confirmReplaceConflict, setGlobalStatus, t],
  );

  const cutoffBulkSearch = useCallback(() => {
    bulkCancelRef.current = false;
    setBulkSearching(true);

    const filtered = cutoffFacetFilter
      ? cutoffItems.filter((item) => item.titleFacet === cutoffFacetFilter)
      : cutoffItems;

    setBulkProgress({ current: 0, total: filtered.length });

    void (async () => {
      let searched = 0;
      for (const item of filtered) {
        if (bulkCancelRef.current) break;
        searched++;
        setBulkProgress({ current: searched, total: filtered.length });
        try {
          await searchAndQueueCutoffItem(item);
        } catch {
          // continue to next item on error
        }
      }
      setBulkSearching(false);
      setBulkProgress(null);
      setGlobalStatus(t("cutoff.bulkComplete", { searched, total: filtered.length }));
    })();
  }, [cutoffItems, cutoffFacetFilter, searchAndQueueCutoffItem, setGlobalStatus, t]);

  const cancelBulkSearch = useCallback(() => {
    bulkCancelRef.current = true;
  }, []);

  return (
    <>
      <div className="flex h-full min-h-0 flex-col">
      <WantedView
        section={wantedSection}
        onOpenOverview={onOpenOverview}
        wantedState={{
          items,
          total,
          loading,
          statusFilter,
          setStatusFilter,
          mediaTypeFilter,
          setMediaTypeFilter,
          latestDecisionCodeFilter,
          setLatestDecisionCodeFilter,
          titleFilterInput,
          setTitleFilterInput,
          offset,
          setOffset,
          limit,
          refreshItems,
          expandedItemId,
          decisions,
          decisionsLoading,
          loadDecisions,
          triggerSearch,
          pauseItem,
          resumeItem,
          resetItem,
          triggerMismatchRecovery,
        }}
        cutoffState={{
          items: cutoffItems,
          loading: cutoffLoading,
          facetFilter: cutoffFacetFilter,
          setFacetFilter: setCutoffFacetFilter,
          autoSearchingId: cutoffAutoSearchingId,
          interactiveSearchingId: cutoffInteractiveSearchingId,
          activeInteractiveItemId: cutoffActiveInteractiveItemId,
          searchResultsByItemId: cutoffSearchResultsByItemId,
          bulkSearching,
          bulkProgress,
          triggerAutoSearch: cutoffTriggerAutoSearch,
          triggerInteractiveSearch: cutoffTriggerInteractiveSearch,
          queueRelease: cutoffQueueRelease,
          triggerBulkSearch: cutoffBulkSearch,
          cancelBulkSearch,
        }}
        pendingState={{
          items: pendingItems,
          loading: pendingLoading,
          refreshItems: refreshPending,
          forceGrab: forceGrabPending,
          dismiss: dismissPending,
        }}
      />
      </div>
      {replaceConflictDialog}
    </>
  );
});
