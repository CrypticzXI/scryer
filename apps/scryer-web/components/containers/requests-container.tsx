import * as React from "react";
import { useClient } from "urql";

import { RequestsView } from "@/components/views/requests-view";
import { librariesQuery, mediaRequestsQuery } from "@/lib/graphql/queries";
import type { Facet, LibraryRecord, MediaRequestRecord } from "@/lib/types";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";

type RequestsContainerProps = {
  facet: Facet;
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
  const [libraries, setLibraries] = React.useState<LibraryRecord[]>([]);
  const [selectedLibraryIds, setSelectedLibraryIds] = React.useState<string[]>([]);
  const [requests, setRequests] = React.useState<MediaRequestRecord[]>([]);
  const [loading, setLoading] = React.useState(false);

  const refresh = React.useCallback(async () => {
    setLoading(true);
    try {
      const [librariesResult, requestsResult] = await Promise.all([
        client.query(librariesQuery, {
          facet,
          permission: "manageTitles",
        }).toPromise(),
        client.query(mediaRequestsQuery, {
          facet,
          libraryIds: selectedLibraryIds.length > 0 ? selectedLibraryIds : null,
          status: "pending",
        }).toPromise(),
      ]);
      if (librariesResult.error) throw librariesResult.error;
      if (requestsResult.error) throw requestsResult.error;
      setLibraries((librariesResult.data?.libraries ?? []) as LibraryRecord[]);
      setRequests(collapseMediaRequests((requestsResult.data?.mediaRequests ?? []) as MediaRequestRecord[]));
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.apiError"));
    } finally {
      setLoading(false);
    }
  }, [client, facet, selectedLibraryIds, setGlobalStatus, t]);

  React.useEffect(() => {
    void refresh();
  }, [refresh]);

  React.useEffect(() => {
    setSelectedLibraryIds([]);
  }, [facet]);

  return (
    <RequestsView
      libraries={libraries}
      selectedLibraryIds={selectedLibraryIds}
      onSelectedLibraryIdsChange={setSelectedLibraryIds}
      requests={requests}
      loading={loading}
      onRefresh={() => void refresh()}
    />
  );
}
