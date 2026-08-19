import * as React from "react";
import { useClient } from "urql";

import { DashboardView } from "@/components/views/dashboard-view";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";
import {
  dispatchNavigationBadgesRefresh,
  NAVIGATION_BADGES_REFRESH_EVENT,
  type NavigationBadgesRefreshDetail,
} from "@/lib/events/navigation-badges";
import {
  approveMediaRequestMutation,
  dismissMediaRequestMutation,
} from "@/lib/graphql/mutations";
import {
  dashboardOverviewQuery,
  dashboardPendingImportsQuery,
  dashboardPendingRequestsQuery,
  dashboardRecentImportsQuery,
  downloadQueuePageQuery,
} from "@/lib/graphql/queries";
import { usePluginManagement } from "@/lib/hooks/use-plugin-management";
import type {
  DashboardImportedItem,
  DashboardOverview,
  DashboardPendingImport,
  DashboardPluginUpdate,
  DashboardRequest,
  DashboardRequestLibrary,
  DownloadQueueItem,
} from "@/lib/types";
import { isBreakingVersionChange } from "@/lib/utils/dashboard";

/** Trailing window the two 24h tiles compare against the window before it. */
const ACTIVITY_WINDOW_HOURS = 24;
/**
 * The top panels show roughly three rows but scroll, so they fetch a short page
 * rather than only what is visible. Totals in the badges come from the server's
 * own counts, not from these lists.
 */
const PREVIEW_FETCH_LIMIT = 15;
/**
 * Enough of the queue to aggregate per-client activity from. `downloadQueuePage`
 * has no per-client aggregate, so the counts are folded client-side; 200 rows
 * covers a realistic queue without pulling the whole history.
 */
const QUEUE_FETCH_LIMIT = 200;

export function DashboardContainer() {
  const client = useClient();
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();

  const [overview, setOverview] = React.useState<DashboardOverview | null>(null);
  const [requests, setRequests] = React.useState<DashboardRequest[]>([]);
  const [requestLibraries, setRequestLibraries] = React.useState<
    DashboardRequestLibrary[]
  >([]);
  const [pendingImports, setPendingImports] = React.useState<
    DashboardPendingImport[]
  >([]);
  const [pendingImportTotal, setPendingImportTotal] = React.useState(0);
  const [recentImports, setRecentImports] = React.useState<DashboardImportedItem[]>(
    [],
  );
  const [queueItems, setQueueItems] = React.useState<DownloadQueueItem[]>([]);
  const [queueTotal, setQueueTotal] = React.useState(0);
  const [loading, setLoading] = React.useState(true);
  const [actionRequestId, setActionRequestId] = React.useState<string | null>(null);

  const reportError = React.useCallback(
    (error: unknown) => {
      setGlobalStatus(
        error instanceof Error ? error.message : t("status.apiError"),
      );
    },
    [setGlobalStatus, t],
  );

  // The plugin strip is the settings page's own plugin machinery: the same
  // registry query, the same upgrade mutation and the same in-flight tracking,
  // so an update started here behaves exactly as it does under Settings.
  const noopRefreshProviderOptions = React.useCallback(async () => {}, []);
  const { plugins, mutatingPluginIds, upgradePlugin, refreshPluginsRegistry } =
    usePluginManagement({
      client,
      t,
      refreshProviderOptions: noopRefreshProviderOptions,
    });

  const pluginUpdates = React.useMemo<DashboardPluginUpdate[]>(
    () =>
      plugins
        .filter((plugin) => plugin.updateAvailable && plugin.isInstalled)
        .map((plugin) => ({
          id: plugin.id,
          name: plugin.name || plugin.id,
          fromVersion: plugin.installedVersion ?? plugin.version ?? null,
          toVersion: plugin.latestVersion ?? null,
          breaking: isBreakingVersionChange(
            plugin.installedVersion ?? plugin.version ?? null,
            plugin.latestVersion ?? null,
          ),
        })),
    [plugins],
  );

  const refreshOverview = React.useCallback(async () => {
    const { data, error } = await client
      .query(dashboardOverviewQuery, {
        activityWindowHours: ACTIVITY_WINDOW_HOURS,
      })
      .toPromise();
    if (error) throw error;

    const badges = data?.navigationBadgeCounts;
    const health = data?.systemHealth;
    setOverview({
      username: data?.me?.username ?? null,
      pendingRequestCount: sumFacetCounts(badges?.pendingMediaRequestCounts),
      pendingImportCount: sumFacetCounts(badges?.pendingImportCounts),
      library: {
        movies: health?.titlesMovie ?? 0,
        series: health?.titlesSeries ?? 0,
        anime: health?.titlesAnime ?? 0,
      },
      activity: data?.dashboardActivityStats ?? {
        current: { grabbed: 0, upgraded: 0, imported: 0, importFailed: 0 },
        previous: { grabbed: 0, upgraded: 0, imported: 0, importFailed: 0 },
      },
      indexerStats: health?.indexerStats ?? [],
      indexers: data?.indexers ?? [],
      downloadClients: data?.downloadClientConfigs ?? [],
      storageRoots: data?.storageRoots ?? [],
    });
  }, [client]);

  const refreshRequests = React.useCallback(async () => {
    const { data, error } = await client
      .query(dashboardPendingRequestsQuery, {})
      .toPromise();
    if (error) throw error;

    const loaded = (data?.mediaRequests ?? []) as DashboardRequest[];
    // Oldest first: the request that has been waiting longest leads.
    setRequests(
      [...loaded]
        .sort((left, right) => Date.parse(left.createdAt) - Date.parse(right.createdAt))
        .slice(0, PREVIEW_FETCH_LIMIT),
    );
    setRequestLibraries((data?.libraries ?? []) as DashboardRequestLibrary[]);
  }, [client]);

  const refreshPendingImports = React.useCallback(async () => {
    const { data, error } = await client
      .query(dashboardPendingImportsQuery, { limit: PREVIEW_FETCH_LIMIT })
      .toPromise();
    if (error) throw error;

    const merged = [
      ...((data?.movie?.items ?? []) as DashboardPendingImport[]),
      ...((data?.series?.items ?? []) as DashboardPendingImport[]),
      ...((data?.anime?.items ?? []) as DashboardPendingImport[]),
    ];
    setPendingImports(
      merged
        .sort((left, right) => Date.parse(left.createdAt) - Date.parse(right.createdAt))
        .slice(0, PREVIEW_FETCH_LIMIT),
    );
    setPendingImportTotal(
      (data?.movie?.totalCount ?? 0) +
        (data?.series?.totalCount ?? 0) +
        (data?.anime?.totalCount ?? 0),
    );
  }, [client]);

  const refreshRecentImports = React.useCallback(async () => {
    const { data, error } = await client
      .query(dashboardRecentImportsQuery, { limit: PREVIEW_FETCH_LIMIT })
      .toPromise();
    if (error) throw error;

    const loaded = (data?.titleHistory?.items ?? []) as DashboardImportedItem[];
    // Newest first.
    setRecentImports(
      [...loaded].sort(
        (left, right) => Date.parse(right.occurredAt) - Date.parse(left.occurredAt),
      ),
    );
  }, [client]);

  const refreshQueue = React.useCallback(async () => {
    const { data, error } = await client
      .query(downloadQueuePageQuery, {
        limit: QUEUE_FETCH_LIMIT,
        scryerSubmittedOnly: false,
      })
      .toPromise();
    if (error) throw error;

    setQueueItems((data?.downloadQueuePage?.items ?? []) as DownloadQueueItem[]);
    setQueueTotal(data?.downloadQueuePage?.totalCount ?? 0);
  }, [client]);

  const refreshAll = React.useCallback(async () => {
    try {
      await Promise.all([
        refreshOverview(),
        refreshRequests(),
        refreshPendingImports(),
        refreshRecentImports(),
        refreshQueue(),
      ]);
    } catch (error) {
      reportError(error);
    } finally {
      setLoading(false);
    }
  }, [
    refreshOverview,
    refreshPendingImports,
    refreshQueue,
    refreshRecentImports,
    refreshRequests,
    reportError,
  ]);

  React.useEffect(() => {
    void refreshAll();
  }, [refreshAll]);

  // The shell already pulses the badge counts on poll and on window focus;
  // riding that pulse keeps the dashboard fresh without a timer of its own.
  React.useEffect(() => {
    const handlePulse = (event: Event) => {
      if (!(event instanceof CustomEvent)) {
        return;
      }
      const source = (event as CustomEvent<NavigationBadgesRefreshDetail>).detail
        ?.source;
      if (source === "poll" || source === "focus") {
        void refreshAll();
      }
    };

    window.addEventListener(NAVIGATION_BADGES_REFRESH_EVENT, handlePulse);
    return () => {
      window.removeEventListener(NAVIGATION_BADGES_REFRESH_EVENT, handlePulse);
    };
  }, [refreshAll]);

  const approveRequest = React.useCallback(
    async (request: DashboardRequest) => {
      if (actionRequestId) {
        return;
      }
      const qualityProfileId = resolveApprovalProfileId(request, requestLibraries);
      if (!qualityProfileId) {
        // Approving needs a profile and the dashboard has no picker, so send the
        // operator to the requests page rather than guessing.
        setGlobalStatus(t("status.apiError"));
        return;
      }

      setActionRequestId(request.id);
      try {
        const { error } = await client
          .mutation(approveMediaRequestMutation, {
            input: {
              requestId: request.id,
              qualityProfileId,
              monitorType:
                request.facet === "MOVIE" ? null : request.requestedMonitorType,
            },
          })
          .toPromise();
        if (error) throw error;
        setGlobalStatus(t("status.requestApproved", { name: request.title }));
        dispatchNavigationBadgesRefresh();
        await refreshAll();
      } catch (error) {
        reportError(error);
      } finally {
        setActionRequestId(null);
      }
    },
    [
      actionRequestId,
      client,
      refreshAll,
      reportError,
      requestLibraries,
      setGlobalStatus,
      t,
    ],
  );

  const dismissRequest = React.useCallback(
    async (request: DashboardRequest) => {
      if (actionRequestId) {
        return;
      }

      setActionRequestId(request.id);
      try {
        const { error } = await client
          .mutation(dismissMediaRequestMutation, { requestId: request.id })
          .toPromise();
        if (error) throw error;
        setGlobalStatus(t("status.requestDismissed", { name: request.title }));
        dispatchNavigationBadgesRefresh();
        await refreshAll();
      } catch (error) {
        reportError(error);
      } finally {
        setActionRequestId(null);
      }
    },
    [actionRequestId, client, refreshAll, reportError, setGlobalStatus, t],
  );

  const updatePlugin = React.useCallback(
    (pluginId: string) => {
      const plugin = plugins.find((candidate) => candidate.id === pluginId);
      if (!plugin) {
        return;
      }
      void upgradePlugin(plugin);
    },
    [plugins, upgradePlugin],
  );

  const updateAllPlugins = React.useCallback(() => {
    for (const update of pluginUpdates) {
      const plugin = plugins.find((candidate) => candidate.id === update.id);
      if (plugin) {
        void upgradePlugin(plugin);
      }
    }
  }, [pluginUpdates, plugins, upgradePlugin]);

  // A finished upgrade changes the registry, so refresh the strip's source.
  React.useEffect(() => {
    if (mutatingPluginIds.length > 0) {
      return;
    }
    void refreshPluginsRegistry();
  }, [mutatingPluginIds.length, refreshPluginsRegistry]);

  return (
    <DashboardView
      loading={loading}
      overview={overview}
      requests={requests}
      pendingImports={pendingImports}
      pendingImportTotal={pendingImportTotal}
      recentImports={recentImports}
      queueItems={queueItems}
      queueTotal={queueTotal}
      pluginUpdates={pluginUpdates}
      updatingPluginIds={mutatingPluginIds}
      actionRequestId={actionRequestId}
      onApproveRequest={(request) => void approveRequest(request)}
      onDismissRequest={(request) => void dismissRequest(request)}
      onUpdatePlugin={updatePlugin}
      onUpdateAllPlugins={updateAllPlugins}
    />
  );
}

function sumFacetCounts(
  counts: { movie?: number; series?: number; anime?: number } | null | undefined,
): number {
  return (counts?.movie ?? 0) + (counts?.series ?? 0) + (counts?.anime ?? 0);
}

/**
 * Which quality profile an inline approval should use, following the same
 * precedence the requests page's approval dialog pre-selects: what the
 * requester asked for, then the library's own profile, then the library's
 * request default. Returns null when none is known, and the caller declines to
 * approve rather than picking one arbitrarily.
 */
function resolveApprovalProfileId(
  request: DashboardRequest,
  libraries: DashboardRequestLibrary[],
): string | null {
  const requested = request.requestedQualityProfileId?.trim();
  if (requested) {
    return requested;
  }

  const library = libraries.find((entry) => entry.id === request.libraryId);
  return (
    library?.qualityProfileId?.trim() ||
    library?.requestQualityProfileDefaultId?.trim() ||
    null
  );
}
