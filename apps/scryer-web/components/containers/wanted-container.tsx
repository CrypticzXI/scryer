import { memo, useCallback, useEffect, useRef, useState } from "react";
import type { WantedSection } from "@/components/root/types";
import { useClient, useMutation } from "urql";
import { WantedView } from "@/components/views/wanted-view";
import type { CutoffUnmetItem } from "@/components/views/cutoff-unmet-view";
import {
  pendingReleasesQuery,
  releaseDecisionsQuery,
  searchQuery,
  wantedCutoffInitQuery,
  wantedItemsQuery,
} from "@/lib/graphql/queries";
import {
  triggerWantedSearchMutation,
  pauseWantedItemMutation,
  resumeWantedItemMutation,
  resetWantedItemMutation,
  queueExistingMutation,
  forceGrabPendingReleaseMutation,
  dismissPendingReleaseMutation,
} from "@/lib/graphql/mutations";
import {
  qualityProfileSettingsToCategoryOverrides,
  qualityProfileSettingsToEntries,
} from "@/lib/utils/quality-profiles";
import { QUALITY_PROFILE_INHERIT_VALUE } from "@/lib/constants/settings";
import type {
  PendingReleaseItem,
  Release,
  ReleaseDecisionItem,
  TitleRecord,
  WantedItem,
  WantedMediaType,
  WantedStatus,
} from "@/lib/types";
import type { ParsedQualityProfileEntry } from "@/lib/types/quality-profiles";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";

function computeCutoffUnmetItems(
  titles: TitleRecord[],
  profileEntries: ParsedQualityProfileEntry[],
  profileIdByScope: Record<string, string>,
  globalProfileId: string | null,
): CutoffUnmetItem[] {
  const profileMap = new Map(profileEntries.map((e) => [e.id, e]));

  const resolveProfile = (title: TitleRecord): ParsedQualityProfileEntry | null => {
    const scopeId = title.facet === "movie" ? "movie" : title.facet === "series" ? "series" : "anime";
    const titleProfileId = title.qualityProfileId?.trim();
    if (titleProfileId && titleProfileId !== QUALITY_PROFILE_INHERIT_VALUE) {
      const p = profileMap.get(titleProfileId);
      if (p) return p;
    }
    const scopeProfileId = profileIdByScope[scopeId];
    if (scopeProfileId && scopeProfileId !== QUALITY_PROFILE_INHERIT_VALUE) {
      const p = profileMap.get(scopeProfileId);
      if (p) return p;
    }
    if (globalProfileId && globalProfileId !== QUALITY_PROFILE_INHERIT_VALUE) {
      return profileMap.get(globalProfileId) ?? null;
    }
    return null;
  };

  const result: CutoffUnmetItem[] = [];

  for (const title of titles) {
    const currentQualityTier = title.currentQualityTier?.trim().toUpperCase();
    if (!title.monitored || !currentQualityTier) continue;

    const profile = resolveProfile(title);
    if (!profile || !profile.criteria.allow_upgrades) continue;

    const tiers = profile.criteria.quality_tiers;
    const cutoffTier = profile.criteria.cutoff_tier?.trim().toUpperCase();
    if (tiers.length === 0 || !cutoffTier) continue;

    const currentIndex = tiers.findIndex((tier) => tier.toUpperCase() === currentQualityTier);
    const cutoffIndex = tiers.findIndex((tier) => tier.toUpperCase() === cutoffTier);
    if (currentIndex === -1 || cutoffIndex === -1) continue;

    // Already at or above the configured cutoff tier.
    if (currentIndex <= cutoffIndex) continue;

    result.push({
      id: title.id,
      name: title.name,
      facet: title.facet,
      posterUrl: title.posterUrl,
      currentTier: currentQualityTier,
      targetTier: cutoffTier,
    });
  }

  return result;
}

type WantedContainerProps = {
  wantedSection: WantedSection;
};

export const WantedContainer = memo(function WantedContainer({
  wantedSection,
}: WantedContainerProps) {
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const client = useClient();

  // --- Wanted items state ---
  const [items, setItems] = useState<WantedItem[]>([]);
  const [total, setTotal] = useState(0);
  const [loading, setLoading] = useState(false);
  const [statusFilter, setStatusFilter] = useState<WantedStatus | undefined>(undefined);
  const [mediaTypeFilter, setMediaTypeFilter] = useState<WantedMediaType | undefined>(undefined);
  const [offset, setOffset] = useState(0);
  const limit = 50;

  const [expandedItemId, setExpandedItemId] = useState<string | null>(null);
  const [decisions, setDecisions] = useState<ReleaseDecisionItem[]>([]);
  const [decisionsLoading, setDecisionsLoading] = useState(false);

  const [, executeTriggerSearch] = useMutation(triggerWantedSearchMutation);
  const [, executePause] = useMutation(pauseWantedItemMutation);
  const [, executeResume] = useMutation(resumeWantedItemMutation);
  const [, executeReset] = useMutation(resetWantedItemMutation);

  // --- Cutoff state ---
  const [cutoffItems, setCutoffItems] = useState<CutoffUnmetItem[]>([]);
  const [cutoffLoading, setCutoffLoading] = useState(false);
  const [cutoffFacetFilter, setCutoffFacetFilter] = useState<string | undefined>(undefined);
  const [cutoffSearchingId, setCutoffSearchingId] = useState<string | null>(null);
  const [bulkSearching, setBulkSearching] = useState(false);
  const [bulkProgress, setBulkProgress] = useState<{ current: number; total: number } | null>(null);
  const bulkCancelRef = useRef(false);
  // Keep a ref to the full title list so search can access externalIds
  const titlesRef = useRef<TitleRecord[]>([]);

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
  }, [client, statusFilter, mediaTypeFilter, offset, t, setGlobalStatus]);

  useEffect(() => {
    if (wantedSection === "wanted") {
      void refreshItems();
    }
  }, [refreshItems, wantedSection]);

  // --- Cutoff data fetching ---

  const refreshCutoff = useCallback(async () => {
    setCutoffLoading(true);
    try {
      const { data, error } = await client.query(wantedCutoffInitQuery, {}).toPromise();
      if (error) throw error;

      const titles: TitleRecord[] = data?.titles ?? [];
      titlesRef.current = titles;

      const entries = qualityProfileSettingsToEntries(data?.qualityProfileSettings);
      const globalProfileId = data?.qualityProfileSettings?.globalProfileId ?? null;
      const profileIdByScope = qualityProfileSettingsToCategoryOverrides(
        data?.qualityProfileSettings,
      );

      const computed = computeCutoffUnmetItems(titles, entries, profileIdByScope, globalProfileId);
      setCutoffItems(computed);
    } catch (error) {
      const message = error instanceof Error ? error.message : t("status.failedToLoad");
      setGlobalStatus(message);
    } finally {
      setCutoffLoading(false);
    }
  }, [client, t, setGlobalStatus]);

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
      const { error } = await executeTriggerSearch({ input: { wantedItemId: id } });
      if (error) {
        setGlobalStatus(error.message);
      } else {
        setGlobalStatus(t("wanted.searchTriggered"));
        void refreshItems();
      }
    },
    [executeTriggerSearch, refreshItems, setGlobalStatus, t],
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

  // --- Cutoff search actions ---

  const searchAndQueueTitle = useCallback(
    async (cutoffItem: CutoffUnmetItem) => {
      const title = titlesRef.current.find((t) => t.id === cutoffItem.id);
      if (!title) return;

      const imdbId =
        title.externalIds
          ?.find((e) => e.source.toLowerCase() === "imdb")
          ?.value?.trim() || null;
      const tvdbId =
        title.externalIds
          ?.find((e) => e.source.toLowerCase() === "tvdb")
          ?.value?.trim() || null;

      const { data, error } = await client
        .query(searchQuery, {
          query: title.name,
          imdbId,
          tvdbId,
          category: title.facet === "movie" ? "movie" : title.facet === "series" ? "series" : "anime",
          limit: title.facet === "movie" ? 50 : 15,
        })
        .toPromise();

      if (error) throw error;

      const results: Release[] = data?.searchReleases ?? [];
      const top = results.find((r) => r.qualityProfileDecision?.allowed ?? true);
      if (!top) {
        setGlobalStatus(t("status.noReleaseForTitle", { name: title.name }));
        return;
      }

      const sourceHint = top.downloadUrl || top.link;
      if (!sourceHint) {
        setGlobalStatus(t("status.noSource", { name: title.name }));
        return;
      }

      const { error: queueError } = await client
        .mutation(queueExistingMutation, {
          input: {
            titleId: title.id,
            scope: { title: true },
            release: {
              sourceHint,
              sourceKind: top.sourceKind ?? null,
              sourceTitle: top.title,
            },
          },
        })
        .toPromise();

      if (queueError) throw queueError;
      setGlobalStatus(t("cutoff.searchTriggered", { name: title.name }));
    },
    [client, t, setGlobalStatus],
  );

  const cutoffTriggerSearch = useCallback(
    async (item: CutoffUnmetItem) => {
      setCutoffSearchingId(item.id);
      try {
        await searchAndQueueTitle(item);
      } catch (error) {
        setGlobalStatus(error instanceof Error ? error.message : t("status.queueFailed"));
      } finally {
        setCutoffSearchingId(null);
      }
    },
    [searchAndQueueTitle, setGlobalStatus, t],
  );

  const cutoffBulkSearch = useCallback(() => {
    bulkCancelRef.current = false;
    setBulkSearching(true);

    const filtered = cutoffFacetFilter
      ? cutoffItems.filter((i) => i.facet === cutoffFacetFilter)
      : cutoffItems;

    setBulkProgress({ current: 0, total: filtered.length });

    void (async () => {
      let searched = 0;
      for (const item of filtered) {
        if (bulkCancelRef.current) break;
        searched++;
        setBulkProgress({ current: searched, total: filtered.length });
        try {
          await searchAndQueueTitle(item);
        } catch {
          // continue to next title on error
        }
      }
      setBulkSearching(false);
      setBulkProgress(null);
      setGlobalStatus(t("cutoff.bulkComplete", { searched, total: filtered.length }));
    })();
  }, [cutoffItems, cutoffFacetFilter, searchAndQueueTitle, setGlobalStatus, t]);

  const cancelBulkSearch = useCallback(() => {
    bulkCancelRef.current = true;
  }, []);

  return (
    <div className="flex h-full min-h-0 flex-col">
      <WantedView
        section={wantedSection}
        wantedState={{
          items,
          total,
          loading,
          statusFilter,
          setStatusFilter,
          mediaTypeFilter,
          setMediaTypeFilter,
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
        }}
        cutoffState={{
          items: cutoffItems,
          loading: cutoffLoading,
          facetFilter: cutoffFacetFilter,
          setFacetFilter: setCutoffFacetFilter,
          searchingId: cutoffSearchingId,
          bulkSearching,
          bulkProgress,
          triggerSearch: cutoffTriggerSearch,
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
  );
});
