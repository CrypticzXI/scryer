import * as React from "react";
import { useClient } from "urql";

import { RequestsView } from "@/components/views/requests-view";
import {
  approveMediaRequestMutation,
  cancelMyMediaRequestMutation,
  dismissMediaRequestMutation,
  updateMyMediaRequestMutation,
} from "@/lib/graphql/mutations";
import {
  mediaRequestAdminLibrariesQuery,
  mediaRequestRequesterLibrariesQuery,
  mediaRequestsQuery,
  myMediaRequestsQuery,
  qualityProfileOptionsQuery,
} from "@/lib/graphql/queries";
import type { Facet, LibraryRecord, MediaRequestRecord } from "@/lib/types";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";
import {
  dispatchNavigationBadgesRefresh,
  NAVIGATION_BADGES_REFRESH_EVENT,
  type NavigationBadgesRefreshDetail,
} from "@/lib/events/navigation-badges";
import { useAuth } from "@/lib/hooks/use-auth";
import { useMediaRequestsSubscription } from "@/lib/hooks/use-media-requests-subscription";
import {
  hasAnyLibraryPermission,
  LIBRARY_PERMISSIONS,
} from "@/lib/utils/permissions";

type RequestsContainerProps = {
  facet: Facet;
};

type RequestsMode = "admin" | "mine";

type QualityProfileOption = {
  id: string;
  name: string;
};

type UpdateRequestValues = {
  requestedQualityProfileId: string;
  requestedMonitorType?: string;
};

type ApproveRequestValues = {
  qualityProfileId: string;
  monitorType?: string;
};

function externalIdKey(source: string, value: string): string {
  return `${source.trim().toLowerCase()}:${value.trim()}`;
}

function requestsOverlap(left: MediaRequestRecord, right: MediaRequestRecord): boolean {
  if (left.libraryId !== right.libraryId || left.facet !== right.facet) {
    return false;
  }

  const rightIds = new Set(
    right.externalIds.map((externalId) =>
      externalIdKey(externalId.source, externalId.value),
    ),
  );
  return left.externalIds.some((externalId) =>
    rightIds.has(externalIdKey(externalId.source, externalId.value)),
  );
}

function collapseRequestGroup(group: MediaRequestRecord[]): MediaRequestRecord {
  const sorted = [...group].sort(
    (a, b) => Date.parse(a.createdAt) - Date.parse(b.createdAt),
  );
  const base = sorted[0];
  const externalIds = new Map<string, MediaRequestRecord["externalIds"][number]>();
  const requesters = new Map<string, MediaRequestRecord["requesters"][number]>();
  let updatedAt = base.updatedAt;

  for (const request of sorted) {
    if (Date.parse(request.updatedAt) > Date.parse(updatedAt)) {
      updatedAt = request.updatedAt;
    }
    for (const externalId of request.externalIds) {
      externalIds.set(externalIdKey(externalId.source, externalId.value), externalId);
    }
    for (const requester of request.requesters) {
      const existing = requesters.get(requester.userId);
      if (!existing || Date.parse(requester.requestedAt) < Date.parse(existing.requestedAt)) {
        requesters.set(requester.userId, requester);
      }
    }
  }

  return {
    ...base,
    externalIds: Array.from(externalIds.values()).sort((a, b) =>
      externalIdKey(a.source, a.value).localeCompare(externalIdKey(b.source, b.value)),
    ),
    requesters: Array.from(requesters.values()).sort(
      (a, b) => Date.parse(a.requestedAt) - Date.parse(b.requestedAt),
    ),
    updatedAt,
  };
}

function collapseMediaRequests(requests: MediaRequestRecord[]): MediaRequestRecord[] {
  const groups: MediaRequestRecord[][] = [];

  for (const request of requests) {
    const matchingIndexes = groups
      .map((group, index) => ({ group, index }))
      .filter(({ group }) => group.some((candidate) => requestsOverlap(candidate, request)))
      .map(({ index }) => index);

    if (matchingIndexes.length === 0) {
      groups.push([request]);
      continue;
    }

    const targetIndex = matchingIndexes[0];
    groups[targetIndex].push(request);
    for (const index of matchingIndexes.slice(1).reverse()) {
      groups[targetIndex].push(...groups[index]);
      groups.splice(index, 1);
    }
  }

  return groups
    .map(collapseRequestGroup)
    .sort((a, b) => Date.parse(b.updatedAt) - Date.parse(a.updatedAt));
}

export function RequestsContainer({ facet }: RequestsContainerProps) {
  const client = useClient();
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const { user } = useAuth();
  const canManageTitle = hasAnyLibraryPermission(user, LIBRARY_PERMISSIONS.manageTitles);
  const canRequestMedia = hasAnyLibraryPermission(user, LIBRARY_PERMISSIONS.request);
  const [mode, setMode] = React.useState<RequestsMode>(
    canManageTitle ? "admin" : "mine",
  );
  const [libraries, setLibraries] = React.useState<LibraryRecord[]>([]);
  const [selectedLibraryIds, setSelectedLibraryIds] = React.useState<string[]>([]);
  const [requests, setRequests] = React.useState<MediaRequestRecord[]>([]);
  const [qualityProfileOptions, setQualityProfileOptions] = React.useState<
    QualityProfileOption[]
  >([]);
  const [loading, setLoading] = React.useState(false);
  const [actionRequestId, setActionRequestId] = React.useState<string | null>(null);

  React.useEffect(() => {
    setMode(canManageTitle ? "admin" : "mine");
  }, [canManageTitle, user?.id]);

  React.useEffect(() => {
    if (mode === "admin" && !canManageTitle) {
      setMode("mine");
    } else if (mode === "mine" && !canRequestMedia && canManageTitle) {
      setMode("admin");
    }
  }, [canManageTitle, canRequestMedia, mode]);

  const refresh = React.useCallback(async () => {
    setLoading(true);
    try {
      const librariesQuery =
        mode === "admin"
          ? mediaRequestAdminLibrariesQuery
          : mediaRequestRequesterLibrariesQuery;
      const requestsQuery = mode === "admin" ? mediaRequestsQuery : myMediaRequestsQuery;
      const requestStatus = mode === "admin" ? "pending" : null;
      const [librariesResult, requestsResult, qualityProfilesResult] = await Promise.all([
        client.query(librariesQuery, {
          facet,
        }).toPromise(),
        client.query(requestsQuery, {
          facet,
          libraryIds: selectedLibraryIds.length > 0 ? selectedLibraryIds : null,
          status: requestStatus,
        }).toPromise(),
        client.query(qualityProfileOptionsQuery, {}).toPromise(),
      ]);
      if (librariesResult.error) throw librariesResult.error;
      if (requestsResult.error) throw requestsResult.error;
      if (qualityProfilesResult.error) throw qualityProfilesResult.error;
      setLibraries((librariesResult.data?.libraries ?? []) as LibraryRecord[]);
      const loadedRequests =
        mode === "admin"
          ? requestsResult.data?.mediaRequests
          : requestsResult.data?.myMediaRequests;
      setRequests(
        mode === "admin"
          ? collapseMediaRequests((loadedRequests ?? []) as MediaRequestRecord[])
          : ((loadedRequests ?? []) as MediaRequestRecord[]),
      );
      setQualityProfileOptions(
        (
          qualityProfilesResult.data?.qualityProfileSettings?.profiles ?? []
        ).map((profile: QualityProfileOption) => ({
          id: profile.id,
          name: profile.name || profile.id,
        })),
      );
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.apiError"));
    } finally {
      setLoading(false);
    }
  }, [client, facet, mode, selectedLibraryIds, setGlobalStatus, t]);

  React.useEffect(() => {
    void refresh();
  }, [refresh]);

  useMediaRequestsSubscription(() => {
    dispatchNavigationBadgesRefresh();
    void refresh();
  });

  React.useEffect(() => {
    const handleNavigationBadgePulse = (event: Event) => {
      if (!(event instanceof CustomEvent)) {
        return;
      }
      const source = (event as CustomEvent<NavigationBadgesRefreshDetail>)
        .detail?.source;
      if (source === "poll" || source === "focus") {
        void refresh();
      }
    };

    window.addEventListener(
      NAVIGATION_BADGES_REFRESH_EVENT,
      handleNavigationBadgePulse,
    );
    return () => {
      window.removeEventListener(
        NAVIGATION_BADGES_REFRESH_EVENT,
        handleNavigationBadgePulse,
      );
    };
  }, [refresh]);

  React.useEffect(() => {
    setSelectedLibraryIds([]);
  }, [facet, mode]);

  const changeMode = React.useCallback((nextMode: RequestsMode) => {
    setMode(nextMode);
  }, []);

  const approveRequest = React.useCallback(
    async (request: MediaRequestRecord, values: ApproveRequestValues) => {
      if (actionRequestId) {
        return;
      }

      setActionRequestId(request.id);
      try {
        const { data, error } = await client
          .mutation(approveMediaRequestMutation, {
            input: {
              requestId: request.id,
              qualityProfileId: values.qualityProfileId,
              monitorType: values.monitorType ?? null,
            },
          })
          .toPromise();
        if (error) throw error;
        const searchError = data?.approveMediaRequest?.searchError;
        setGlobalStatus(
          searchError
            ? t("status.requestApprovedSearchFailed", {
                name: request.title,
                error: searchError,
              })
            : t("status.requestApproved", { name: request.title }),
        );
        dispatchNavigationBadgesRefresh();
        await refresh();
      } catch (error) {
        setGlobalStatus(error instanceof Error ? error.message : t("status.apiError"));
      } finally {
        setActionRequestId(null);
      }
    },
    [actionRequestId, client, refresh, setGlobalStatus, t],
  );

  const dismissRequest = React.useCallback(
    async (request: MediaRequestRecord) => {
      if (actionRequestId) {
        return;
      }

      setActionRequestId(request.id);
      try {
        const { error } = await client
          .mutation(dismissMediaRequestMutation, { input: { requestId: request.id } })
          .toPromise();
        if (error) throw error;
        setGlobalStatus(t("status.requestDismissed", { name: request.title }));
        dispatchNavigationBadgesRefresh();
        await refresh();
      } catch (error) {
        setGlobalStatus(error instanceof Error ? error.message : t("status.apiError"));
      } finally {
        setActionRequestId(null);
      }
    },
    [actionRequestId, client, refresh, setGlobalStatus, t],
  );

  const updateRequest = React.useCallback(
    async (request: MediaRequestRecord, values: UpdateRequestValues) => {
      if (actionRequestId) {
        return;
      }

      setActionRequestId(request.id);
      try {
        const { error } = await client
          .mutation(updateMyMediaRequestMutation, {
            input: {
              requestId: request.id,
              requestedQualityProfileId: values.requestedQualityProfileId,
              requestedMonitorType: values.requestedMonitorType ?? null,
            },
          })
          .toPromise();
        if (error) throw error;
        setGlobalStatus(t("status.requestUpdated", { name: request.title }));
        dispatchNavigationBadgesRefresh();
        await refresh();
      } catch (error) {
        setGlobalStatus(error instanceof Error ? error.message : t("status.apiError"));
      } finally {
        setActionRequestId(null);
      }
    },
    [actionRequestId, client, refresh, setGlobalStatus, t],
  );

  const cancelRequest = React.useCallback(
    async (request: MediaRequestRecord) => {
      if (actionRequestId) {
        return;
      }

      setActionRequestId(request.id);
      try {
        const { error } = await client
          .mutation(cancelMyMediaRequestMutation, { input: { requestId: request.id } })
          .toPromise();
        if (error) throw error;
        setGlobalStatus(t("status.requestCanceled", { name: request.title }));
        dispatchNavigationBadgesRefresh();
        await refresh();
      } catch (error) {
        setGlobalStatus(error instanceof Error ? error.message : t("status.apiError"));
      } finally {
        setActionRequestId(null);
      }
    },
    [actionRequestId, client, refresh, setGlobalStatus, t],
  );

  return (
    <RequestsView
      mode={mode}
      canShowAdminMode={canManageTitle}
      canShowRequesterMode={canRequestMedia}
      onModeChange={changeMode}
      libraries={libraries}
      selectedLibraryIds={selectedLibraryIds}
      onSelectedLibraryIdsChange={setSelectedLibraryIds}
      requests={requests}
      qualityProfileOptions={qualityProfileOptions}
      loading={loading}
      actionRequestId={actionRequestId}
      onRefresh={() => void refresh()}
      onApprove={(request, values) =>
        void approveRequest(request, values)
      }
      onDismiss={(request) => void dismissRequest(request)}
      onUpdateRequest={(request, values) => void updateRequest(request, values)}
      onCancelRequest={(request) => void cancelRequest(request)}
    />
  );
}
